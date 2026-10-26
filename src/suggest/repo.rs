//! What the writer may read of the project: the root, the start scripts, and a capped, redacted excerpt of
//! allowlisted files. Nothing here is sent anywhere; `mod.rs` sends the result. Owner: S3b-writer.
use super::schema::Start;
use super::state::env;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Same rule as the engine: `CLAUDE_PROJECT_DIR`, else the given folder, else the current one; canonical.
pub fn resolve_root(cwd: Option<&str>) -> Result<PathBuf, String> {
    let raw = env("CLAUDE_PROJECT_DIR")
        .or_else(|| cwd.filter(|c| !c.is_empty()).map(String::from))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .ok_or("no project folder")?;
    let root =
        fs::canonicalize(&raw).map_err(|e| format!("cannot open the project folder: {e}"))?;
    let home = env("HOME").and_then(|h| fs::canonicalize(h).ok());
    if root == Path::new("/") || home.as_deref() == Some(root.as_path()) {
        return Err("refusing to work in `/` or in the home folder".into());
    }
    Ok(root)
}

/// A `.synthetic` file in the project root says its content is made up (tests, demos).
pub fn is_synthetic(root: &Path) -> bool {
    root.join(".synthetic").is_file()
}

/// The start commands the model may pick, built from `package.json` scripts with fixed names. Never free text.
/// shortcut: one fixed fallback (`node server.js`) when there is no usable script; no other guesses. Add more when real projects need them.
pub fn start_choices(root: &Path) -> Vec<Start> {
    let mut out = Vec::new();
    let scripts = fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|v| v["scripts"].clone());
    for name in ["start", "dev", "serve", "server"] {
        if scripts.as_ref().is_some_and(|s| s[name].is_string()) {
            let cmd = if name == "start" {
                "npm start --silent".to_string()
            } else {
                format!("npm run {name} --silent")
            };
            out.push(Start {
                key: name.into(),
                cmd,
            });
        }
    }
    if out.is_empty() && root.join("server.js").is_file() {
        out.push(Start {
            key: "node server.js".into(),
            cmd: "node server.js".into(),
        });
    }
    out
}

// ---------------------------------------------------------------- redaction

const STRONG: [&str; 14] = [
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "private_key",
    "privatekey",
    "credential",
    "auth_key",
    "authkey",
    "signing",
    "salt",
    "authorization",
    "bearer",
];
const PASSWORDS: [&str; 3] = ["password", "passwd", "pwd"];
// Built with concat! so that this file holds no complete key-like literal.
const PREFIXES: [&str; 17] = [
    "sk-",
    "sk_live_",
    "sk_test_",
    "pk_live_",
    concat!("gh", "p_"),
    concat!("gh", "o_"),
    concat!("gh", "u_"),
    concat!("gh", "s_"),
    concat!("gh", "r_"),
    concat!("github", "_pat_"),
    concat!("gl", "pat-"),
    concat!("xo", "xb-"),
    concat!("xo", "xp-"),
    "AKIA",
    "AIza",
    "LLM_",
    "eyJ",
];
const MASK: &str = "[redacted]";

fn is_tok(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '/' | '.')
}

/// Looks like a key: long and mixed, or a known key prefix with a digit and enough length.
fn key_like(t: &str) -> bool {
    let n = t.chars().count();
    let digits = t.chars().filter(char::is_ascii_digit).count();
    let letters = t.chars().filter(char::is_ascii_alphabetic).count();
    (n >= 24 && digits >= 2 && letters >= 8)
        || (digits >= 1
            && PREFIXES
                .iter()
                .any(|p| t.starts_with(p) && n >= p.len() + 8))
}

/// shortcut: a password literal is kept only in a `.synthetic` project, where the model needs the seed user's login.
/// Anywhere else it is masked like a secret. Upgrade when real projects need their seed logins (an opt-in setting).
fn sensitive(name: &str, synthetic: bool) -> bool {
    STRONG.iter().any(|k| name.contains(k))
        || (!synthetic && (PASSWORDS.iter().any(|k| name.contains(k)) || name == "pass"))
}

fn mask_url_credentials(s: &str) -> String {
    let (mut out, mut rest) = (String::new(), s);
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| matches!(c, '/' | '?' | '#' | '"' | '\'' | '`') || c.is_whitespace())
            .unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) if tail[..at].contains(':') => {
                out.push_str(MASK);
                rest = &tail[at..];
            }
            _ => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

fn apply(c: &[char], cuts: &[(usize, usize)]) -> String {
    let (mut out, mut at) = (String::new(), 0);
    for &(a, b) in cuts {
        out.extend(&c[at..a]);
        out.push_str(MASK);
        at = b;
    }
    out.extend(&c[at..]);
    out
}

/// The value after `name =` / `name:` / `"name": ` is masked when the name is sensitive. A quoted value is masked
/// inside its quotes; a bare word of 6+ characters is masked; an expression (`a.b`, `f(x)`, `a[i]`) is kept.
fn mask_assignments(line: &str, synthetic: bool) -> String {
    let c: Vec<char> = line.chars().collect();
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let skip = !matches!(c[i], '=' | ':')
            || matches!(c.get(i + 1), Some('=' | '>' | '/'))
            || (i > 0 && matches!(c[i - 1], '=' | '!' | '<' | '>'));
        if skip {
            i += 1;
            continue;
        }
        let mut j = i;
        while j > 0 && c[j - 1] == ' ' {
            j -= 1;
        }
        if j > 0 && matches!(c[j - 1], '"' | '\'' | '`') {
            j -= 1;
        }
        let end = j;
        while j > 0 && (c[j - 1].is_ascii_alphanumeric() || matches!(c[j - 1], '_' | '-' | '.')) {
            j -= 1;
        }
        let name: String = c[j..end].iter().collect::<String>().to_lowercase();
        let mut v = i + 1;
        while v < c.len() && matches!(c[v], ' ' | '*') {
            v += 1;
        }
        if v >= c.len() || !sensitive(&name, synthetic) {
            i += 1;
            continue;
        }
        if matches!(c[v], '"' | '\'' | '`') {
            let close = c[v + 1..]
                .iter()
                .position(|&x| x == c[v])
                .map_or(c.len(), |p| v + 1 + p);
            if close > v + 1 {
                cuts.push((v + 1, close));
            }
            i = close.max(i + 1);
        } else {
            let mut e = v;
            while e < c.len()
                && (c[e].is_ascii_alphanumeric() || matches!(c[e], '_' | '-' | '+' | '/'))
            {
                e += 1;
            }
            let word: String = c[v..e].iter().collect::<String>().to_lowercase();
            let plain = ["true", "false", "null", "undefined", "none"].contains(&word.as_str());
            let expression = matches!(c.get(e), Some('.' | '(' | '['));
            if e - v >= 6 && !plain && !expression {
                cuts.push((v, e));
            }
            i = e.max(i + 1);
        }
    }
    apply(&c, &cuts)
}

/// The word after `Bearer ` or `Basic ` (an Authorization header).
fn mask_bearer(s: &str) -> String {
    let low = s.to_ascii_lowercase();
    let c: Vec<char> = s.chars().collect();
    let l: Vec<char> = low.chars().collect();
    let mut cuts = Vec::new();
    for word in ["bearer ", "basic "] {
        let w: Vec<char> = word.chars().collect();
        let mut i = 0;
        while i + w.len() < l.len() {
            if l[i..i + w.len()] == w[..] {
                let a = i + w.len();
                let mut e = a;
                while e < c.len() && (is_tok(c[e]) || c[e] == '=') {
                    e += 1;
                }
                if e - a >= 8 {
                    cuts.push((a, e));
                }
                i = e.max(i + 1);
            } else {
                i += 1;
            }
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    // overlapping cuts cannot happen: the two words never share the text after them
    apply(&c, &cuts)
}

fn mask_key_like(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let (mut cuts, mut i) = (Vec::new(), 0);
    while i < c.len() {
        if !is_tok(c[i]) {
            i += 1;
            continue;
        }
        let mut e = i;
        while e < c.len() && is_tok(c[e]) {
            e += 1;
        }
        let run: String = c[i..e].iter().collect();
        if key_like(&run) {
            cuts.push((i, e));
        }
        i = e;
    }
    apply(&c, &cuts)
}

/// Removes what looks like a secret from text that is about to leave the machine. Best effort by shape and by name:
/// PEM blocks, URL credentials, values of sensitive names, Bearer words and key-like tokens.
pub fn redact(text: &str, synthetic: bool) -> String {
    let (mut out, mut in_pem) = (String::new(), false);
    for line in text.lines() {
        if in_pem {
            in_pem = !line.contains("-----END ");
            continue;
        }
        if line.contains("-----BEGIN ") {
            in_pem = !line.contains("-----END ");
            out.push_str("[key block removed]\n");
            continue;
        }
        let l = mask_url_credentials(line);
        let l = mask_assignments(&l, synthetic);
        let l = mask_bearer(&l);
        out.push_str(&mask_key_like(&l));
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------- excerpt

const TOTAL_CAP: usize = 14_000;
const FILE_CAP: usize = 6_000;
const LINE_CAP: usize = 300;
const BIG_FILE: u64 = 64_000;
const LIST_CAP: usize = 120;
const EXTS: [&str; 22] = [
    "js", "mjs", "cjs", "ts", "tsx", "jsx", "json", "md", "txt", "html", "ejs", "hbs", "pug", "py",
    "rb", "php", "go", "java", "rs", "cs", "sql", "vue",
];
const SKIP_DIRS: [&str; 11] = [
    "node_modules",
    "dist",
    "build",
    "target",
    "vendor",
    "coverage",
    "__pycache__",
    "venv",
    "out",
    "tmp",
    "logs",
];

fn git_list(root: &Path, extra: &[&str]) -> Option<BTreeSet<String>> {
    let out = Command::new("git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "ls-files",
            "-z",
            "--exclude-standard",
        ])
        .args(extra)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| {
        out.stdout
            .split(|&b| b == 0)
            .filter_map(|p| std::str::from_utf8(p).ok())
            .filter(|p| !p.is_empty())
            .map(String::from)
            .collect()
    })
}

/// The files git would show (tracked and not ignored). Without git: the top level only, minus the plain file
/// names in the root `.gitignore`.
fn listing(root: &Path) -> Vec<String> {
    if let Some(mut all) = git_list(root, &["--cached", "--others"]) {
        if let Some(ignored) = git_list(root, &["--cached", "--ignored"]) {
            all.retain(|p| !ignored.contains(p));
        }
        return all.into_iter().collect();
    }
    let ignored: BTreeSet<String> = fs::read_to_string(root.join(".gitignore"))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains(['/', '*', '?', '[', '!', '#']))
        .map(String::from)
        .collect();
    let mut v: Vec<String> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !ignored.contains(n))
        .collect();
    v.sort();
    v
}

fn allowed(p: &str) -> bool {
    let parts: Vec<&str> = p.split('/').collect();
    let name = parts.last().copied().unwrap_or("").to_lowercase();
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    parts
        .iter()
        .all(|c| !c.starts_with('.') && !SKIP_DIRS.contains(c))
        && EXTS.contains(&ext)
        && !name.contains("secret")
        && !name.contains("credential")
        && !name.ends_with("-lock.json")
        && !name.ends_with(".min.js")
        && !name.ends_with(".d.ts")
}

fn rank(p: &str) -> u8 {
    let low = p.to_lowercase();
    let base = low.rsplit('/').next().unwrap_or(&low);
    if base == "package.json" {
        0
    } else if base.starts_with("readme") {
        1
    } else if ["server.", "app.", "index.", "main."]
        .iter()
        .any(|n| base.starts_with(n))
    {
        2
    } else if low.contains("test") || low.contains("spec") {
        6
    } else if !low.contains('/')
        || ["route", "api", "controller", "handler"]
            .iter()
            .any(|d| low.contains(d))
    {
        3
    } else {
        4
    }
}

/// Cuts at `max` bytes on a character boundary.
pub fn cut(s: &str, max: usize) -> &str {
    let mut n = max.min(s.len());
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

fn read_capped(root: &Path, rel: &str, synthetic: bool) -> Option<String> {
    let path = root.join(rel);
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > BIG_FILE || !fs::canonicalize(&path).ok()?.starts_with(root)
    {
        return None;
    }
    let text = String::from_utf8(fs::read(&path).ok()?).ok()?;
    // Redact the whole text first and cut afterwards, so that a cut never leaves half a key behind.
    let red = redact(&text, synthetic);
    let lines: Vec<String> = red
        .lines()
        .map(|l| l.chars().take(LINE_CAP).collect())
        .collect();
    let body = lines.join("\n");
    let mut shown = cut(&body, FILE_CAP).to_string();
    if shown.len() < body.len() {
        shown.push_str("\n[cut]");
    }
    Some(shown)
}

/// The excerpt: a file list, then the start of the most useful files, all redacted. Stable for an unchanged project.
pub fn excerpt(root: &Path, synthetic: bool) -> String {
    let mut files: Vec<String> = listing(root).into_iter().filter(|p| allowed(p)).collect();
    files.sort_by(|a, b| (rank(a), a).cmp(&(rank(b), b)));
    let mut out = String::from("FILE LIST\n");
    let mut names: Vec<&String> = files.iter().take(LIST_CAP).collect();
    names.sort();
    for n in names {
        out.push_str(n);
        out.push('\n');
    }
    let mut used = out.len();
    for rel in &files {
        let Some(body) = read_capped(root, rel, synthetic) else {
            continue;
        };
        let room = TOTAL_CAP.saturating_sub(used + rel.len() + 16);
        if room < 400 {
            break;
        }
        let part = format!("\nFILE {rel}\n{}\n", cut(&body, room));
        used += part.len();
        out.push_str(&part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test secrets are built from pieces so that this file holds no complete key-like literal.
    fn fake_key() -> String {
        format!("{}{}", "sk-", "Zq8w3Lm0Xv2TnB7yHc4Rj9PdKs1Ae6")
    }

    fn ws(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tm-repo-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn secrets_by_shape_and_by_name_are_masked() {
        let key = fake_key();
        let src = format!(
            "const k = '{key}';\nconst API_SECRET = \"canary-value-1\";\nlet db = 'postgres://admin:hunter2@db-host/app';\n\
             headers: {{ Authorization: 'Bearer abcDEF123456xyz' }}\ntoken: abcdef1234567890\nconst long = 'Zq8w3Lm0Xv2TnB7yHc4Rj9PdKs1Ae6Qp';"
        );
        let out = redact(&src, true);
        for leak in [
            &key[..],
            "canary-value-1",
            "hunter2",
            "abcDEF123456xyz",
            "abcdef1234567890",
            "Zq8w3Lm0Xv2TnB7yHc4Rj9PdKs1Ae6Qp",
        ] {
            assert!(!out.contains(leak), "{leak} leaked in:\n{out}");
        }
        assert!(out.contains("API_SECRET"), "names stay: {out}");
    }

    #[test]
    fn pem_blocks_are_dropped() {
        let pem = format!(
            "a\n-----BEGIN {} KEY-----\nMIIBVQIBADANBgkq\n-----END {} KEY-----\nb",
            "PRIVATE", "PRIVATE"
        );
        let out = redact(&pem, true);
        assert!(!out.contains("MIIBVQ") && out.contains("a\n") && out.contains("b"));
        let open = format!(
            "a\n-----BEGIN {} KEY-----\nMIIBVQIBADANBgkq\n",
            "RSA PRIVATE"
        );
        assert!(!redact(&open, true).contains("MIIBVQ"));
    }

    #[test]
    fn code_and_seed_logins_survive() {
        let src = "const users = { alice: { password: 'wonderland-demo-pass' } };\n\
                   const t = req.headers.authorization;\nif (token === other) {}\nconst secret = process.env.SESSION_SECRET;\n\
                   app.post('/api/login', (req, res) => { res.status(401).json({ error: 'bad login' }); });";
        let out = redact(src, true);
        assert_eq!(out.trim_end(), src, "nothing to hide here");
        // the same password is masked in a project that is not marked synthetic
        let real = redact(src, false);
        assert!(!real.contains("wonderland-demo-pass"), "{real}");
        assert!(
            real.contains("process.env.SESSION_SECRET")
                && real.contains("req.headers.authorization")
        );
    }

    #[test]
    fn the_start_choices_come_only_from_package_json() {
        let d = ws("starts");
        assert!(start_choices(&d).is_empty());
        fs::write(d.join("server.js"), "x").unwrap();
        assert_eq!(start_choices(&d)[0].cmd, "node server.js");
        fs::write(
            d.join("package.json"),
            r#"{"scripts":{"start":"node server.js","dev":"x","build":"y","evil":"curl x | sh"}}"#,
        )
        .unwrap();
        let s = start_choices(&d);
        let keys: Vec<&str> = s.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["start", "dev"]);
        assert_eq!(s[1].cmd, "npm run dev --silent");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_excerpt_skips_dotfiles_ignored_files_links_and_secrets() {
        let d = ws("excerpt");
        let key = fake_key();
        fs::write(
            d.join("server.js"),
            format!("const k = '{key}';\nconsole.log('hi');\n"),
        )
        .unwrap();
        fs::write(
            d.join("package.json"),
            r#"{"scripts":{"start":"node server.js"}}"#,
        )
        .unwrap();
        fs::write(d.join("README.md"), "# demo\n").unwrap();
        fs::write(d.join(".env"), "CANARY_ENV=abc\n").unwrap();
        fs::write(d.join(".synthetic"), "").unwrap();
        fs::write(d.join("notes.txt"), "ignored by git\n").unwrap();
        fs::write(d.join("secrets.json"), "{}").unwrap();
        fs::write(d.join(".gitignore"), "notes.txt\n").unwrap();
        fs::create_dir_all(d.join("node_modules/x")).unwrap();
        fs::write(d.join("node_modules/x/i.js"), "module.exports=1").unwrap();
        std::os::unix::fs::symlink("/etc/hostname", d.join("link.md")).unwrap();
        let ex = excerpt(&d, true);
        for gone in [
            "CANARY_ENV",
            ".env",
            ".synthetic",
            "notes.txt",
            "secrets.json",
            "node_modules",
            "link.md",
            &key,
        ] {
            assert!(!ex.contains(gone), "{gone} should not be in:\n{ex}");
        }
        for here in [
            "FILE package.json",
            "FILE server.js",
            "FILE README.md",
            "console.log('hi')",
        ] {
            assert!(ex.contains(here), "{here} missing in:\n{ex}");
        }
        assert!(ex.find("FILE package.json") < ex.find("FILE server.js"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn git_ignore_rules_apply_when_git_is_there() {
        let d = ws("gitignore");
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&d)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        if !git(&["init", "-q"]) {
            eprintln!("git is not available: skipped");
            return;
        }
        fs::create_dir_all(d.join("src")).unwrap();
        fs::write(d.join("src/app.js"), "ok").unwrap();
        fs::write(d.join("src/local.js"), "private").unwrap();
        fs::write(d.join(".gitignore"), "src/local.js\n").unwrap();
        let ex = excerpt(&d, true);
        assert!(
            ex.contains("src/app.js") && !ex.contains("local.js") && !ex.contains("private"),
            "{ex}"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_excerpt_is_capped() {
        let d = ws("cap");
        for i in 0..12 {
            fs::write(d.join(format!("f{i}.js")), "x".repeat(9000)).unwrap();
        }
        let ex = excerpt(&d, true);
        assert!(ex.len() <= TOTAL_CAP + 200, "{}", ex.len());
        assert!(ex.lines().all(|l| l.chars().count() <= LINE_CAP + 10));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_root_is_never_slash_or_home() {
        assert!(resolve_root(Some("/")).is_err());
        let d = ws("root");
        let got = resolve_root(Some(d.to_str().unwrap()));
        // CLAUDE_PROJECT_DIR wins when set, as in the engine; this test only checks the plain case
        if env("CLAUDE_PROJECT_DIR").is_none() {
            assert_eq!(got.unwrap(), fs::canonicalize(&d).unwrap());
        }
        let _ = fs::remove_dir_all(&d);
    }
}

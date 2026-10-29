//! Tests of what the writer may read of a project: the root, the masks and the excerpt.

use super::excerpt::{FILE_CAP, LINE_CAP, TOTAL_CAP};
use super::*;
use crate::suggest::state::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

// Test secrets are built from pieces so that this file holds no complete key-like literal.
fn fake_key() -> String {
    format!("{}{}", "sk-", "Zq8w3Lm0Xv2TnB7yHc4Rj9PdKs1Ae6")
}

fn ws(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tm-repo-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    fs::canonicalize(&d).unwrap() // the root a run gets is canonical; macOS temp dirs sit behind a symlink
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
fn a_bearer_token_after_a_sensitive_name_is_masked() {
    let tok = format!("{}{}", "abc123", "def456");
    for line in [
        format!("curl -H \"Authorization: Bearer {tok}\" https://api.example.com"),
        format!("token: Bearer {tok}"),
        format!("password = Bearer {tok}"),
        format!("apikey=Bearer {tok}"),
        format!("Authorization: Basic {tok}"),
    ] {
        for synthetic in [true, false] {
            let out = redact(&line, synthetic);
            assert!(!out.contains(&tok), "{tok} leaked in: {out}");
        }
    }
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
        real.contains("process.env.SESSION_SECRET") && real.contains("req.headers.authorization")
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
    fs::write(d.join(".env"), "CANARY_ENV=abc\n").unwrap();
    fs::write(d.join(".synthetic"), "").unwrap();
    fs::write(d.join("notes.txt"), "ignored by git\n").unwrap();
    fs::write(d.join("secrets.json"), "{}").unwrap();
    fs::write(d.join(".gitignore"), "notes.txt\n").unwrap();
    fs::create_dir_all(d.join("node_modules/x")).unwrap();
    fs::write(d.join("node_modules/x/i.js"), "module.exports=1").unwrap();
    std::os::unix::fs::symlink("/etc/hosts", d.join("link.js")).unwrap();
    let ex = excerpt(&d, true, "");
    for gone in [
        "CANARY_ENV",
        ".env",
        ".synthetic",
        "notes.txt",
        "secrets.json",
        "node_modules",
        "link.js",
        &key,
    ] {
        assert!(!ex.contains(gone), "{gone} should not be in:\n{ex}");
    }
    for here in ["FILE package.json", "FILE server.js", "console.log('hi')"] {
        assert!(ex.contains(here), "{here} missing in:\n{ex}");
    }
    assert!(ex.find("FILE package.json") < ex.find("FILE server.js"));
    let _ = fs::remove_dir_all(&d);
}

/// dogfood1 M2: the excerpt holds code and the manifests and none of the app's data files (the test below).
#[test]
fn the_excerpt_holds_code_and_manifests_and_none_of_the_data_files() {
    let d = ws("data");
    fs::write(d.join("server.js"), "app.get('/notes', list);\n").unwrap();
    fs::write(
        d.join("package.json"),
        r#"{"scripts":{"start":"node server.js"}}"#,
    )
    .unwrap();
    fs::write(d.join("tsconfig.json"), r#"{"compilerOptions":{}}"#).unwrap();
    fs::write(
        d.join("notes.json"),
        r#"[{"id":"mvqm4ta2","text":"buy oat milk"}]"#,
    )
    .unwrap();
    fs::write(d.join("data.txt"), "my bank pin is in here\n").unwrap();
    fs::write(d.join("dump.sql"), "INSERT INTO users VALUES ('ana');\n").unwrap();
    fs::write(d.join("README.md"), "# my private notes\n").unwrap();
    let ex = excerpt(&d, false, "");
    for gone in [
        "notes.json",
        "mvqm4ta2",
        "data.txt",
        "bank pin",
        "dump.sql",
        "INSERT INTO",
        "README.md",
        "private notes",
    ] {
        assert!(!ex.contains(gone), "{gone} should not be in:\n{ex}");
    }
    for here in [
        "FILE server.js",
        "app.get('/notes'",
        "FILE package.json",
        "FILE tsconfig.json",
    ] {
        assert!(ex.contains(here), "{here} missing in:\n{ex}");
    }
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
    let ex = excerpt(&d, true, "");
    assert!(
        ex.contains("src/app.js") && !ex.contains("local.js") && !ex.contains("private"),
        "{ex}"
    );
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn the_excerpt_is_capped() {
    let d = ws("cap");
    let body = "const line = 'abcdefghijklmnopqrstuvwxyz0123456789ab';\n".repeat(220); // about 10 KB a file
    for i in 0..12 {
        fs::write(d.join(format!("f{i}.js")), &body).unwrap();
    }
    let ex = excerpt(&d, true, "");
    assert!(ex.len() <= TOTAL_CAP + 400, "{}", ex.len());
    assert!(
        ex.len() > TOTAL_CAP / 2,
        "the cap binds, the excerpt is not far below it: {}",
        ex.len()
    );
    let files: Vec<&str> = ex.split("\nFILE ").skip(1).collect();
    assert!(
        !files.is_empty() && files.len() < 12,
        "later files are left out: {}",
        files.len()
    );
    assert!(
        files.iter().all(|f| f.len() <= FILE_CAP + 200),
        "each file is cut to its own cap"
    );
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

/// A project whose `server.js` is long (about 10 KB of filler routes), then `route`, then a little more.
fn long_project(name: &str, route: &str) -> PathBuf {
    let d = ws(name);
    let mut js = String::new();
    for i in 0..100 {
        js += &format!(
            "app.get('/api/thing{i}', (req, res) => {{\n  const row = db.find({i});\n  res.json(row);\n}});\n\n"
        );
    }
    js += route;
    js += "\napp.listen(3000);\n";
    fs::write(d.join("server.js"), js).unwrap();
    fs::write(
        d.join("lib.js"),
        "exports.add = (a, b) => a + b;\n".repeat(40),
    )
    .unwrap();
    fs::write(
        d.join("package.json"),
        r#"{"scripts":{"start":"node server.js"}}"#,
    )
    .unwrap();
    d
}

const ZEBRA_ROUTE: &str = "// Zebra census: how many zebras the park holds.\napp.get('/api/zebras', (req, res) => {\n  res.json({ zebras: db.count('zebra') });\n});\n";

/// The words only add: every file shows what it showed without them, and the cap holds.
#[test]
fn words_add_windows_after_the_start_and_change_nothing_before_it() {
    let d = long_project("windows", ZEBRA_ROUTE);
    let plain = excerpt(&d, true, "");
    let with = excerpt(&d, true, "The zebra census route works.");
    assert!(!plain.contains("/api/zebras"), "past the start: {plain}");
    assert!(
        with.contains("/api/zebras") && with.contains("db.count('zebra')"),
        "{with}"
    );
    assert!(
        with.len() > plain.len() && with.len() <= TOTAL_CAP,
        "{}",
        with.len()
    );
    let (head, rest) = plain.split_once("\nFILE ").unwrap();
    assert!(with.starts_with(head), "the file list is the same");
    let mut shown = 0;
    for part in rest.split("\nFILE ") {
        let part = part.trim_end_matches('\n');
        let name = part.lines().next().unwrap();
        let there = with
            .split("\nFILE ")
            .find(|p| p.starts_with(&format!("{name}\n")));
        assert!(there.is_some_and(|p| p.starts_with(part)), "{name} changed");
        shown += 1;
    }
    assert_eq!(shown, 3);
    assert_eq!(
        excerpt(&d, true, "The zebra census route works."),
        with,
        "stable"
    );
    let _ = fs::remove_dir_all(&d);
}

/// A window comes from the redacted text, so what the start hides, a window hides.
#[test]
fn a_secret_inside_a_window_is_masked() {
    let key = fake_key();
    let route = format!(
        "{ZEBRA_ROUTE}app.get('/api/zebra-admin', (req, res) => {{\n  const zebraKey = '{key}';\n  const stripeSecret = \"canary-zebra-1\";\n  const url = 'postgres://admin:hunter2@db-host/zebras';\n  res.json({{ zebras: 3 }});\n}});\n"
    );
    let d = long_project("window-secret", &route);
    for synthetic in [true, false] {
        let ex = excerpt(&d, synthetic, "the zebra admin route");
        assert!(
            ex.contains("/api/zebra-admin") && ex.contains("const zebraKey"),
            "{ex}"
        );
        for leak in [&key[..], "canary-zebra-1", "hunter2"] {
            assert!(!ex.contains(leak), "{leak} leaked in a window:\n{ex}");
        }
    }
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn windows_of_many_long_files_stay_inside_the_total_cap() {
    let d = ws("window-cap");
    let body = "const zebra = db.find('zebra');\n".repeat(400);
    for i in 0..6 {
        fs::write(d.join(format!("f{i}.js")), &body).unwrap();
    }
    for words in ["", "zebra", "zebra find db const"] {
        let ex = excerpt(&d, true, words);
        assert!(ex.len() <= TOTAL_CAP, "{words}: {}", ex.len());
    }
    let _ = fs::remove_dir_all(&d);
}

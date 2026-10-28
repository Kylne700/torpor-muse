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
    let ex = excerpt(&d, true);
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

/// dogfood1 M2: the app's own data never goes to the model, only code and the manifests.
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
    let ex = excerpt(&d, false);
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
    let body = "const line = 'abcdefghijklmnopqrstuvwxyz0123456789ab';\n".repeat(220); // about 10 KB a file
    for i in 0..12 {
        fs::write(d.join(format!("f{i}.js")), &body).unwrap();
    }
    let ex = excerpt(&d, true);
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

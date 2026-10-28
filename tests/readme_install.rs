//! The one-line install runs install.sh; the README INSTALL block runs the same script from a here-document
//! (`sh -eu <<'EOF'` ... `EOF`). scripts/cleanroom.sh tests the block at release, so install.sh must be that
//! here-document's text byte for byte: then the one line installs the same files with the same checksum checks.
use std::fs;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The text after the first line of `text` that starts with `open` and before the next line that is exactly `close`,
/// every line with its newline (what a here-document hands to `sh`).
fn between<'a>(text: &'a str, open: &str, close: &str) -> &'a str {
    let mut from = None;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        match from {
            None if line.starts_with(open) => from = Some(at + line.len()),
            Some(start) if line.trim_end_matches('\n') == close => return &text[start..at],
            _ => {}
        }
        at += line.len();
    }
    panic!("no line starting {open:?} followed by a line {close:?}");
}

#[test]
fn install_sh_is_the_script_the_readme_install_block_runs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let readme = fs::read_to_string(root.join("README.md")).unwrap();
    let section = between(&readme, "<!-- INSTALL:start -->", "<!-- INSTALL:end -->");
    let block = between(section, "```sh", "```");
    let script = between(block, "sh -eu <<'EOF'", "EOF");
    assert!(
        !script.is_empty(),
        "the README INSTALL block has an empty here-document"
    );
    let install = fs::read(root.join("install.sh")).unwrap();
    assert!(
        install == script.as_bytes(),
        "install.sh is not the text between `sh -eu <<'EOF'` and `EOF` in the README INSTALL block: change both together"
    );
}

/// install.sh as `curl | sh -eu` runs it, from a release on disk: `rel=` and `eng=` point at file:// folders that hold
/// this build of torpor-muse (with its SHA256SUMS) and the pinned engine (TORPOR_BIN, whose sha256 install.sh
/// checks). An empty HOME, a bare PATH, stdout a pipe (no terminal, so init asks nothing). `settings` is planted in
/// Muse's settings file first when given. Returns the exit code, stdout, stderr and HOME; None when it cannot run here.
/// `fake_muse` is the text of a script that stands in for this build as the release's torpor-muse; the installer's `sh`
/// then leads a session of its own, so a signal the script sends to its process group reaches only that `sh` and its
/// children, never this test run.
fn install_run(
    name: &str,
    settings: Option<&str>,
    fake_muse: Option<&str>,
) -> Option<(i32, String, String, PathBuf)> {
    let engine = match std::env::var_os("TORPOR_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        Some(p) => p,
        None if std::env::var_os("CI").is_some() => {
            panic!("TORPOR_BIN must name the engine binary when CI is set")
        }
        None => {
            eprintln!("SKIPPED: TORPOR_BIN does not name the engine binary");
            return None;
        }
    };
    let p = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-arm64",
        ("macos", "x86_64") => "darwin-x64",
        ("macos", "aarch64") => "darwin-arm64",
        _ => {
            eprintln!("SKIPPED: install.sh has no build for this platform");
            return None;
        }
    };
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("readme-install-{name}"));
    let _ = fs::remove_dir_all(&dir);
    let (rel, eng, home) = (dir.join("rel"), dir.join("eng"), dir.join("home"));
    for d in [&rel, &eng.join(p), &home] {
        fs::create_dir_all(d).unwrap();
    }
    fs::copy(&engine, eng.join(p).join("torpor")).unwrap();
    let muse = rel.join(format!("torpor-muse-{p}"));
    match fake_muse {
        Some(script) => fs::write(&muse, script).unwrap(),
        None => {
            fs::copy(env!("CARGO_BIN_EXE_torpor-muse"), &muse).unwrap();
        }
    }
    let sums = Command::new("sh")
        .args(["-c", "sha256sum \"$0\" 2>/dev/null || shasum -a 256 \"$0\""])
        .arg(format!("torpor-muse-{p}"))
        .current_dir(&rel)
        .output()
        .unwrap();
    assert!(sums.status.success());
    fs::write(rel.join("SHA256SUMS"), &sums.stdout).unwrap();
    if let Some(s) = settings {
        fs::create_dir_all(home.join(".config/muse")).unwrap();
        fs::write(home.join(".config/muse/settings.json"), s).unwrap();
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script: String = fs::read_to_string(root.join("install.sh"))
        .unwrap()
        .lines()
        .map(|l| match l.split_once('=') {
            Some(("rel", _)) => format!("rel=file://{}\n", rel.display()),
            Some(("eng", _)) => format!("eng=file://{}\n", eng.display()),
            _ => format!("{l}\n"),
        })
        .collect();
    let mut sh = Command::new("sh");
    sh.arg("-eu")
        .env_clear()
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if fake_muse.is_some() {
        // SAFETY: setsid(2) only, in the forked child before exec; it allocates nothing. A session of its own also
        // means no controlling terminal, so the installer's /dev/tty cannot be opened (or stop this run) here.
        unsafe {
            sh.pre_exec(|| match libc::setsid() {
                -1 => Err(std::io::Error::last_os_error()),
                _ => Ok(()),
            })
        };
    }
    let mut sh = sh.spawn().unwrap();
    sh.stdin
        .take()
        .unwrap()
        .write_all(script.as_bytes())
        .unwrap();
    let o = sh.wait_with_output().unwrap();
    Some((
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
        home,
    ))
}

/// This build as the release's torpor-muse; see `install_run`.
fn install_from_disk(name: &str, settings: Option<&str>) -> Option<(i32, String, String, PathBuf)> {
    install_run(name, settings, None)
}

#[test]
fn install_sh_ends_by_running_init() {
    let Some((code, out, err, home)) = install_from_disk("ok", None) else {
        return;
    };
    assert_eq!(code, 0, "{out}{err}");
    let bin = home.join(".local/bin");
    assert!(bin.join("torpor").is_file() && bin.join("torpor-muse").is_file());
    let settings = home.join(".config/muse/settings.json");
    assert!(
        out.contains(&format!(
            "torpor-muse hooks installed in {}",
            settings.display()
        )),
        "{out}{err}"
    );
    let hooks = fs::read_to_string(&settings).unwrap();
    assert!(
        hooks.contains(&format!("{} hook Stop", bin.join("torpor-muse").display())),
        "init put in hooks that run the installed torpor-muse: {hooks}"
    );
    assert!(!err.contains("run torpor-muse init later"), "{err}");
}

#[test]
fn when_init_fails_the_install_still_succeeds_and_says_why_in_one_line() {
    let Some((code, out, err, home)) = install_from_disk("init-fails", Some("{ not json")) else {
        return;
    };
    assert_eq!(code, 0, "{out}{err}");
    assert!(home.join(".local/bin/torpor-muse").is_file());
    let lines: Vec<&str> = err
        .lines()
        .filter(|l| l.contains("torpor-muse init"))
        .collect();
    assert_eq!(lines.len(), 1, "{err}");
    assert!(
        lines[0].starts_with("torpor-muse init: ")
            && lines[0].ends_with("; the install is done, run torpor-muse init later"),
        "{err}"
    );
}

/// A Ctrl-C at the key prompt: the terminal sends SIGINT to every process of the foreground group, installer included.
/// The fake init does the same to its own group (the installer's `sh` leads it, see `install_run`) and dies of it, as
/// the real init does. The install must still end 0 (the one-line install's `&& export PATH=...` then runs).
#[test]
fn when_ctrl_c_stops_init_the_install_still_succeeds_and_says_so_in_one_line() {
    let fake = "#!/bin/sh\n[ \"$1\" = init ] && kill -INT 0\n";
    let Some((code, out, err, home)) = install_run("ctrl-c", None, Some(fake)) else {
        return;
    };
    assert_eq!(code, 0, "{out}{err}");
    let bin = home.join(".local/bin");
    assert!(bin.join("torpor").is_file() && bin.join("torpor-muse").is_file());
    assert_eq!(
        err.trim(),
        "torpor-muse init stopped; the install is done, run torpor-muse init later"
    );
}

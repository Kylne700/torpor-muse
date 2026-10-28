//! `init` asks for the Meta API key when a person can type it: stdout is a terminal, /dev/tty opens and the config
//! names no usable key file. A `sh` child reads one line from /dev/tty with echo off and hands it over a pipe; its trap
//! turns echo back on however it ends, Ctrl-C included. The key goes alone into ~/.config/torpor-muse/meta-api-key and
//! the config gets its `key_file` line, both 0600. The key is not printed, checked or sent: a wrong one shows only when
//! Meta refuses it (`torpor-muse doctor` checks the file, not what is in it).

use crate::suggest::{config_path, key_file_gap, load_config, parse_config};
use crate::util::{private_dir, write_atomic};
use std::fs;
use std::io::{ErrorKind, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The `key_file` line init writes (the config reader expands `~/`).
const LINE: &str = "key_file = ~/.config/torpor-muse/meta-api-key";

/// Shows `$1`, reads one line with echo off and prints it to stdout (the pipe). The EXIT trap turns echo back on;
/// INT, TERM and HUP leave through it.
const READ: &str = "trap 'stty echo' EXIT; trap 'echo >/dev/tty; exit 130' INT TERM HUP; stty -echo || exit 1; \
                    printf '%s' \"$1\" >/dev/tty; IFS= read -r k || exit 1; echo >/dev/tty; printf '%s' \"$k\"";

const PROMPT: &str = "Paste your Meta API key (it is not shown; Enter alone skips): ";

/// The key file init writes, next to the config.
fn key_path() -> Option<PathBuf> {
    config_path().map(|c| c.with_file_name("meta-api-key"))
}

/// The config text with `LINE` as its one `key_file` setting; every other line is kept.
fn with_key_line(old: &str) -> String {
    let mut s: String = old
        .lines()
        .filter(|l| {
            l.split_once('=')
                .is_none_or(|(k, _)| k.trim() != "key_file")
        })
        .map(|l| format!("{l}\n"))
        .collect();
    s.push_str(LINE);
    s.push('\n');
    s
}

/// Writes `key` alone on one line to `file`, then `config` (both 0600, each through a temp file and a rename).
fn save(file: &Path, key: &str, config: &Path, text: &str) -> Result<(), String> {
    let dir = config.parent().unwrap_or(Path::new("."));
    private_dir(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    for (path, bytes) in [(file, format!("{key}\n")), (config, text.to_owned())] {
        write_atomic(path, bytes.as_bytes(), 0o600, true, || Ok(()))
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    Ok(())
}

/// After the hooks are in: ask for the key when a person is at a terminal and the config names no usable key file.
/// Without a terminal it returns at once and prints nothing.
pub(super) fn offer() -> Result<(), String> {
    if !std::io::stdout().is_terminal() {
        return Ok(());
    }
    let Ok(tty) = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
    else {
        return Ok(());
    };
    if let Ok(cfg) = load_config()
        && let Some(f) = &cfg.key_file
        && key_file_gap(&cfg).is_none()
    {
        println!("Meta API key file already set: {}", f.display());
        return Ok(());
    }
    let (Some(config), Some(file)) = (config_path(), key_path()) else {
        return Err("HOME is not set".into());
    };
    let old = match fs::read_to_string(&config) {
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        r => r.map_err(|e| format!("cannot read {}: {e}", config.display()))?,
    };
    let text = with_key_line(&old);
    parse_config(&text).map_err(|e| {
        format!(
            "{e} in {}; fix that, then run torpor-muse init again to add your Meta API key",
            config.display()
        )
    })?;
    let out = Command::new("/bin/sh")
        .args(["-c", READ, "sh", PROMPT])
        .stdin(tty)
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("cannot start sh to read the key: {e}"))?;
    let key = String::from_utf8(out.stdout).unwrap_or_default();
    if !out.status.success() {
        return Err(
            "no key was read from the terminal; run torpor-muse init again to add it".into(),
        );
    }
    let key = key.trim();
    if key.is_empty() {
        println!(
            "No key: Torpor saves and undoes without one; the check-writer stays off (run torpor-muse init again to add a key)."
        );
        return Ok(());
    }
    save(&file, key, &config, &text)?;
    println!("Meta API key saved in {}", file.display());
    Ok(())
}

/// For `uninstall`, which never deletes it: the key file the config names, or else the one init writes, if it exists.
pub(super) fn kept() -> Option<PathBuf> {
    load_config()
        .ok()
        .and_then(|c| c.key_file)
        .filter(|f| f.exists())
        .or_else(|| key_path().filter(|f| f.exists()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_key_line_is_added_once_and_every_other_line_is_kept() {
        assert_eq!(with_key_line(""), format!("{LINE}\n"));
        let old = "# mine\nkey_file = /old/k\nday_cap_usd = 2\n  key_file=/other\n# key_file = /x\nday_cap_usd = 3";
        let new = with_key_line(old);
        assert_eq!(
            new,
            format!("# mine\nday_cap_usd = 2\n# key_file = /x\nday_cap_usd = 3\n{LINE}\n")
        );
        assert!(parse_config(&new).is_ok());
        assert_eq!(with_key_line(&new), new, "a second run changes nothing");
    }

    #[test]
    fn the_key_and_the_config_are_written_private() {
        let d = std::env::temp_dir().join(format!("tm-install-key-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let config = d.join("torpor-muse/config");
        let file = d.join("torpor-muse/meta-api-key");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "day_cap_usd = 2\n").unwrap();
        fs::set_permissions(&config, fs::Permissions::from_mode(0o644)).unwrap();
        let text = with_key_line(&fs::read_to_string(&config).unwrap());
        save(&file, "test-not-a-key", &config, &text).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(fs::read_to_string(&file).unwrap(), "test-not-a-key\n");
        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            format!("day_cap_usd = 2\n{LINE}\n")
        );
        assert_eq!((mode(&file), mode(&config)), (0o600, 0o600));
        let _ = fs::remove_dir_all(&d);
    }
}

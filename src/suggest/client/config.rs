//! The API key and the config file: where the key may come from, who may read it and the day cap.

use crate::suggest::state::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// The API key. It has no Display, and Debug hides it.
#[derive(Clone)]
pub struct Key(String);

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Key(hidden)")
    }
}

impl Key {
    fn new(s: &str) -> Result<Self, String> {
        let s = s.trim();
        if s.is_empty() || s.len() > 400 || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("the API key is empty or not one line of plain text".into());
        }
        Ok(Self(s.to_string()))
    }

    /// Replaces the key in text that is about to leave this process or be shown (belt and braces).
    pub fn scrub(&self, s: &str) -> String {
        s.replace(&self.0, "[key]")
    }

    pub(super) fn bearer(&self) -> String {
        format!("Bearer {}", self.0)
    }
}

#[derive(Default, Debug, PartialEq)]
pub struct Config {
    pub key_file: Option<PathBuf>,
    pub day_cap: Option<f64>,
}

pub fn config_path() -> Option<PathBuf> {
    env("HOME").map(|h| Path::new(&h).join(".config/torpor-muse/config"))
}

/// A file with a secret or a path to one must not be readable by group or others.
fn check_private(p: &Path, what: &str) -> Result<(), String> {
    let m = fs::metadata(p).map_err(|e| format!("cannot read {what} ({}): {e}", p.display()))?;
    if !m.is_file() || m.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "{what} ({}) must be a plain file that only you can read: run chmod 600 on it",
            p.display()
        ));
    }
    Ok(())
}

/// `~/x` in a config value is the home folder (a shell does that for a typed path, nothing does it for a file).
fn expand_home(v: &str) -> PathBuf {
    match (v.strip_prefix("~/"), env("HOME")) {
        (Some(rest), Some(h)) => Path::new(&h).join(rest),
        _ => PathBuf::from(v),
    }
}

pub fn parse_config(text: &str) -> Result<Config, String> {
    let mut c = Config::default();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line
            .split_once('=')
            .ok_or_else(|| format!("config line {}: expected `name = value`", n + 1))?;
        let v = v.trim().trim_matches('"');
        match k.trim() {
            "key_file" if !v.is_empty() => c.key_file = Some(expand_home(v)),
            "day_cap_usd" => {
                c.day_cap = Some(
                    v.parse::<f64>()
                        .ok()
                        .filter(|x| x.is_finite() && *x >= 0.0)
                        .ok_or_else(|| {
                            format!("config line {}: day_cap_usd must be a number", n + 1)
                        })?,
                );
            }
            other => {
                return Err(format!(
                    "config line {}: unknown or empty setting `{other}`",
                    n + 1
                ));
            }
        }
    }
    Ok(c)
}

/// `~/.config/torpor-muse/config` (mode 0600, refused otherwise): `key_file = <path>`, `day_cap_usd = <number>`.
/// It holds the path of the key file, never the key.
pub fn load_config() -> Result<Config, String> {
    let Some(p) = config_path().filter(|p| p.exists()) else {
        return Ok(Config::default());
    };
    check_private(&p, "the config file")?;
    let text = fs::read_to_string(&p).map_err(|e| format!("cannot read the config file: {e}"))?;
    parse_config(&text)
}

/// For the trigger and `doctor`: why a detached run cannot read its key from a file, or None when it can (the config
/// names a file only its owner can read; what is inside is not read here).
pub fn key_file_gap(cfg: &Config) -> Option<String> {
    match cfg.key_file.as_deref() {
        None => Some("there is no `key_file = <path>` line in ~/.config/torpor-muse/config".into()),
        Some(f) => check_private(f, "the key file").err(),
    }
}

/// Is there a usable `META_API_KEY` in this process's environment?
pub fn env_key_ok() -> bool {
    env("META_API_KEY").is_some_and(|k| Key::new(&k).is_ok())
}

/// `META_API_KEY` when it is set (run by hand); otherwise the file the config names. Never from argv. A run started
/// by the trigger gets no `META_API_KEY` when the config names a key file, so it takes the file.
pub fn load_key(cfg: &Config) -> Result<Key, String> {
    key_from(cfg, env("META_API_KEY"))
}

fn key_from(cfg: &Config, from_env: Option<String>) -> Result<Key, String> {
    if let Some(k) = from_env {
        return Key::new(&k);
    }
    let f = cfg.key_file.as_deref().ok_or(
        "no API key: set META_API_KEY (for a run by hand), or put `key_file = <path of a file holding the key>` in ~/.config/torpor-muse/config (the run Muse Code starts after a prompt needs the key file)",
    )?;
    check_private(f, "the key file")?;
    Key::new(&fs::read_to_string(f).map_err(|e| format!("cannot read the key file: {e}"))?)
}

pub fn day_cap(cfg: &Config) -> Result<f64, String> {
    match env("TORPOR_MUSE_DAY_CAP_USD") {
        Some(v) => v
            .parse::<f64>()
            .ok()
            .filter(|x| x.is_finite() && *x >= 0.0)
            .ok_or_else(|| "TORPOR_MUSE_DAY_CAP_USD must be a number".into()),
        None => Ok(cfg.day_cap.unwrap_or(crate::ledger::DEFAULT_CAP_USD)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_config_names_a_file_and_nothing_else() {
        let c = parse_config("# a comment\nkey_file = /tmp/k\nday_cap_usd = 1.5\n").unwrap();
        assert_eq!(c.key_file, Some(PathBuf::from("/tmp/k")));
        assert_eq!(c.day_cap, Some(1.5));
        assert!(
            parse_config("api_key = abc").is_err(),
            "a key in the config is refused"
        );
        assert!(parse_config("day_cap_usd = lots").is_err());
        assert!(parse_config("key_file").is_err());
    }

    #[test]
    fn a_key_file_may_start_with_the_home_folder_sign() {
        let _g = crate::suggest::state::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let saved = std::env::var_os("HOME");
        // SAFETY: every test that touches the environment holds ENV_LOCK.
        unsafe { std::env::set_var("HOME", "/h/u") };
        let at_home = parse_config("key_file = ~/keys/k\n").unwrap().key_file;
        let quoted = parse_config("key_file = \"~/k\"\n").unwrap().key_file;
        let elsewhere = parse_config("key_file = /a/~/k\n").unwrap().key_file;
        unsafe {
            match saved {
                Some(h) => std::env::set_var("HOME", h),
                None => std::env::remove_var("HOME"),
            }
        }
        assert_eq!(at_home, Some(PathBuf::from("/h/u/keys/k")));
        assert_eq!(quoted, Some(PathBuf::from("/h/u/k")));
        assert_eq!(elsewhere, Some(PathBuf::from("/a/~/k")));
    }

    #[test]
    fn files_that_others_can_read_are_refused_and_the_key_stays_hidden() {
        let d = std::env::temp_dir().join(format!("tm-client-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        let f = d.join("k");
        fs::write(&f, "LLM_testkey0123456789\n").unwrap();
        fs::set_permissions(&f, fs::Permissions::from_mode(0o644)).unwrap();
        let cfg = Config {
            key_file: Some(f.clone()),
            day_cap: None,
        };
        assert!(
            key_from(&cfg, None)
                .unwrap_err()
                .contains("only you can read")
        );
        assert!(key_file_gap(&cfg).unwrap().contains("only you can read"));
        fs::set_permissions(&f, fs::Permissions::from_mode(0o600)).unwrap();
        let key = key_from(&cfg, None).unwrap();
        assert_eq!(key_file_gap(&cfg), None);
        assert!(
            key_file_gap(&Config::default())
                .unwrap()
                .contains("key_file")
        );
        assert_eq!(format!("{key:?}"), "Key(hidden)");
        assert_eq!(key.scrub("a LLM_testkey0123456789 b"), "a [key] b");
        assert!(Key::new("two words").is_err() && Key::new("").is_err());
        let _ = fs::remove_dir_all(&d);
    }
}

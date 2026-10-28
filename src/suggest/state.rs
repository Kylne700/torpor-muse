//! Files of the check-writer that live outside the project: a lock and a result per project. Owner: S3b-writer.
use std::fs::{self, File, OpenOptions, TryLockError};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Tests that change the process environment hold this, one at a time.
#[cfg(test)]
pub static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// An environment variable that is set and not empty.
pub fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// `TORPOR_MUSE_STATE`, else `<torpor home>/muse/suggest` (next to the rest of our files). Not XDG_STATE_HOME: Muse
/// gives its hooks no XDG_* variables, so a run started by a hook and one typed by hand would keep two ledgers and
/// two day caps (found in A10 of the end-to-end check).
pub fn state_dir() -> Result<PathBuf, String> {
    if let Some(d) = env("TORPOR_MUSE_STATE") {
        return Ok(PathBuf::from(d));
    }
    if env("TORPOR_HOME").is_none() && env("HOME").is_none() {
        return Err("HOME is not set".into());
    }
    Ok(crate::engine::state_dir().join("suggest"))
}

/// The folder of one project (by its canonical path) under the state dir; created private.
pub fn ws_dir(root: &Path) -> Result<PathBuf, String> {
    let d = ws_path(root)?;
    crate::util::private_dir(&d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    Ok(d)
}

fn ws_path(root: &Path) -> Result<PathBuf, String> {
    let name = format!("{:016x}", crate::util::fnv1a(root.as_os_str().as_bytes()));
    Ok(state_dir()?.join("ws").join(name))
}

/// What a saved check does not cover (L3 N4), for the hook to show: `gaps.json` in the project's folder, the last 5,
/// newest first. `session`: the Muse session whose prompt started the run (None: a run typed by hand).
#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct Gap {
    pub promise: u32,
    pub session: Option<String>,
    pub gap: String,
}

/// Only a run that holds the project's lock writes here, so there is one writer at a time.
pub fn add_gap(root: &Path, g: Gap) -> Result<(), String> {
    let path = ws_dir(root)?.join("gaps.json");
    let mut all = gaps(root);
    all.retain(|x| x.promise != g.promise);
    all.insert(0, g);
    all.truncate(5);
    write_atomic(&path, &serde_json::to_vec(&all).map_err(|e| e.to_string())?)
}

pub fn gaps(root: &Path) -> Vec<Gap> {
    ws_path(root)
        .ok()
        .and_then(|d| fs::read(d.join("gaps.json")).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// The fields of the project's last `result.json` that the hook reads to tell the user a run failed (it writes none
/// of them). A missing or unreadable file is None; a field a row lacks stays empty.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
pub struct Last {
    pub saved: bool,
    pub code: String,
    pub outcome: String,
    pub message: String,
    pub session: Option<String>,
}

pub fn last_result(root: &Path) -> Option<Last> {
    let bytes = fs::read(ws_path(root).ok()?.join("result.json")).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// The lock of one project: held while this value (the open file) lives.
pub struct Lock {
    _file: File,
}

/// `None` when another run holds the lock.
pub fn try_lock(dir: &Path) -> Result<Option<Lock>, String> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(dir.join("lock"))
        .map_err(|e| format!("cannot open the lock file: {e}"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Lock { _file: file })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(format!("cannot take the lock: {e}")),
    }
}

/// Writes next to the target and renames, so a reader never sees half a file.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), String> {
    crate::util::write_atomic(path, data, 0o600, true, || Ok(()))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tm-state-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_second_lock_is_refused_until_the_first_is_dropped() {
        let d = tmp("lock");
        let a = try_lock(&d).unwrap();
        assert!(a.is_some());
        assert!(try_lock(&d).unwrap().is_none());
        drop(a);
        assert!(try_lock(&d).unwrap().is_some());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_result_is_replaced_whole_and_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp("atomic");
        let p = d.join("result.json");
        write_atomic(&p, b"{\"a\":1}").unwrap();
        write_atomic(&p, b"{\"a\":2}").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "{\"a\":2}");
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(fs::read_dir(&d).unwrap().count(), 1, "no temp file is left");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_state_folder_ignores_xdg_so_a_hook_and_a_typed_run_share_it() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let saved: Vec<_> = ["TORPOR_MUSE_STATE", "TORPOR_HOME", "XDG_STATE_HOME", "HOME"]
            .iter()
            .map(|k| (*k, std::env::var_os(k)))
            .collect();
        // SAFETY: every test that touches the environment holds ENV_LOCK.
        unsafe {
            std::env::remove_var("TORPOR_MUSE_STATE");
            std::env::set_var("TORPOR_HOME", "/th");
            std::env::set_var("HOME", "/hm/u");
            std::env::set_var("XDG_STATE_HOME", "/xdg");
        }
        let with_xdg = state_dir();
        unsafe { std::env::remove_var("XDG_STATE_HOME") };
        let without_xdg = state_dir();
        unsafe {
            std::env::remove_var("TORPOR_HOME");
            std::env::remove_var("HOME");
        }
        let nowhere = state_dir();
        unsafe {
            for (k, v) in saved {
                match v {
                    Some(v) => std::env::set_var(k, v),
                    None => std::env::remove_var(k),
                }
            }
        }
        assert_eq!(
            with_xdg.as_deref().ok(),
            Some(Path::new("/th/muse/suggest"))
        );
        assert_eq!(with_xdg, without_xdg);
        assert!(nowhere.is_err());
    }

    #[test]
    fn two_projects_get_two_folders() {
        assert_ne!(crate::util::fnv1a(b"/a/one"), crate::util::fnv1a(b"/a/two"));
    }
}

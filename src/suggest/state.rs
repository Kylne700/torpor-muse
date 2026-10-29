//! Files of the check-writer that live outside the project: a lock and a result per project. Owner: S3b-writer.
use std::fs::{self, File, OpenOptions, TryLockError};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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

/// Where the folder of a project is (or will be), without making it: for a reader.
pub fn ws_path(root: &Path) -> Result<PathBuf, String> {
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
    lock_file(dir, "lock")
}

fn lock_file(dir: &Path, name: &str) -> Result<Option<Lock>, String> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(dir.join(name))
        .map_err(|e| format!("cannot open the lock file: {e}"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Lock { _file: file })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(format!("cannot take the lock: {e}")),
    }
}

/// The file the writer locks while the engine saves a new check, and the Stop hook looks at (not `lock`, which a run
/// holds from start to end). The system drops a flock when its process ends, so a crashed run leaves nothing held.
const SAVE_LOCK: &str = "save.lock";
const SAVE_POLL: Duration = Duration::from_millis(50);

/// Taken by the writer around the engine's `promise add`, which holds the engine's own run lock for seconds (each proof
/// run). `None` when it cannot be taken: the save goes on, only without a Stop that waits for it.
/// shortcut: a Stop that probes in the very microsecond the writer locks makes this one try fail, and that save then goes
/// unwaited-for (as before this change); retry a few times if a measured run ever shows it.
pub fn save_lock(dir: &Path) -> Option<Lock> {
    lock_file(dir, SAVE_LOCK).ok().flatten()
}

/// What the Stop hook calls before it asks the engine to check the saved promises: waits, at most `cap`, while a check
/// is being saved, and says how long it waited. No save in flight, no lock file yet, or a lock that cannot be asked
/// (all of these are one `flock` call, no sleep): it returns at once. Opens the file read-only.
pub fn wait_for_save(dir: &Path, cap: Duration) -> Duration {
    let started = Instant::now();
    let Ok(file) = File::open(dir.join(SAVE_LOCK)) else {
        return Duration::ZERO;
    };
    while matches!(file.try_lock_shared(), Err(TryLockError::WouldBlock)) {
        let left = cap.saturating_sub(started.elapsed());
        if left.is_zero() {
            break;
        }
        std::thread::sleep(SAVE_POLL.min(left));
    }
    started.elapsed()
}

/// Writes next to the target and renames it into place (one `rename` call).
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
        // a child forked by a parallel test can hold the inherited lock fd until it execs: wait for it, up to 2 s
        let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut b = try_lock(&d).unwrap();
        while b.is_none() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(10));
            b = try_lock(&d).unwrap();
        }
        assert!(b.is_some());
        let _ = fs::remove_dir_all(&d);
    }

    // A parallel test's fork can hold a copy of an open lock fd until it execs: the bounds below allow for that.
    const SLACK: Duration = Duration::from_secs(2);

    #[test]
    fn a_stop_waits_for_a_save_in_flight_and_for_nothing_else() {
        let d = tmp("save");
        let cap = Duration::from_secs(5);
        // no save started here: the wait is zero, and no lock file appears (the third assert)
        assert_eq!(wait_for_save(&d, cap), Duration::ZERO);
        assert_eq!(wait_for_save(&d.join("nope"), cap), Duration::ZERO);
        assert!(!d.join(SAVE_LOCK).exists());
        assert!(
            save_lock(&d.join("nope")).is_none(),
            "no folder, no lock, no panic"
        );
        // a save in flight: the Stop is let go when the writer lets go, long before the cap
        let saving = save_lock(&d).expect("the writer takes the lock");
        assert!(save_lock(&d).is_none(), "one save at a time");
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            drop(saving);
        });
        let waited = wait_for_save(&d, cap);
        release.join().unwrap();
        assert!(
            waited >= Duration::from_millis(350) && waited < SLACK,
            "{waited:?}"
        );
        // nothing in flight any more: back at once
        assert!(wait_for_save(&d, cap) < SLACK);
        // a save that outlasts the cap: the Stop gives up at the cap and goes on
        let _held = save_lock(&d).expect("taken again");
        let waited = wait_for_save(&d, Duration::from_millis(300));
        assert!(
            waited >= Duration::from_millis(300) && waited < SLACK,
            "{waited:?}"
        );
        let _ = fs::remove_dir_all(&d);
    }

    /// A writer that is killed (no `drop`, no cleanup) leaves the lock free: the system lets go of a flock when the
    /// process ends. Python's flock is the same call as std's on Unix.
    #[test]
    fn a_writer_that_is_killed_leaves_no_save_in_flight() {
        use std::io::{BufRead, BufReader};
        use std::process::{Command, Stdio};
        let d = tmp("killed");
        let code = "import fcntl,sys,time; f=open(sys.argv[1],'a'); fcntl.flock(f, fcntl.LOCK_EX); print('held', flush=True); time.sleep(120)";
        let mut child = Command::new("python3")
            .args(["-I", "-c", code])
            .arg(d.join(SAVE_LOCK))
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3 starts");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        assert_eq!(line.trim(), "held");
        let waited = wait_for_save(&d, Duration::from_millis(300));
        assert!(
            waited >= Duration::from_millis(300),
            "the lock was not seen as held: {waited:?}"
        );
        child.kill().unwrap(); // SIGKILL
        child.wait().unwrap();
        assert!(
            wait_for_save(&d, Duration::from_secs(5)) < SLACK,
            "the dead writer still holds it"
        );
        assert!(save_lock(&d).is_some(), "and the next writer can take it");
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

//! Our own small files in `<TORPOR_HOME>/muse/` (best effort: a failure here never stops a hook):
//! `problems.log` (read by doctor), `seen-keys` (payload keys we have no field for), `session/<id>.json`.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const LOG_CAP: u64 = 32 << 10;
const KEYS_CAP: u64 = 16 << 10;
const LOCK_WAIT: Duration = Duration::from_millis(500);
const KEEP_DAYS: u64 = 14;
/// Torpor's own cap of Stop blocks per turn. Muse stops a turn after 8 continuations (research/59 D8).
pub const STOP_CAP: u32 = 2;

/// What the hooks of one session share. Parallel tool calls run their hooks at the same time, hence the file lock.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct Session {
    /// Number of the last state note we put in front of the model (research/59 D9).
    pub seq: u64,
    /// The user was told once that Torpor is not saving.
    pub told: bool,
    /// (turn id, Stop blocks let through), oldest first, the last 8 turns.
    pub blocks: Vec<(String, u32)>,
}

impl Session {
    pub fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// True when one more Stop block may go out in `turn`; counts it.
    pub fn allow_block(&mut self, turn: &str) -> bool {
        let at = match self.blocks.iter().position(|(t, _)| t == turn) {
            Some(i) => i,
            None => {
                self.blocks.push((turn.to_string(), 0));
                if self.blocks.len() > 8 {
                    self.blocks.remove(0);
                }
                self.blocks.len() - 1
            }
        };
        let n = &mut self.blocks[at].1;
        *n += 1;
        *n <= STOP_CAP
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn append(path: &Path, bytes: &[u8]) {
    let open = || {
        OpenOptions::new()
            .append(true)
            .create(true)
            .mode(0o600)
            .open(path)
    };
    let mut f = match open() {
        Ok(f) => f,
        Err(_) => {
            // the directory may not exist yet
            if let Some(d) = path.parent() {
                let _ = crate::engine::private_dir(d);
            }
            match open() {
                Ok(f) => f,
                Err(_) => return,
            }
        }
    };
    let _ = f.write_all(bytes);
}

/// Session ids become file names: letters, digits, `-` and `_` only.
pub fn safe(id: &str) -> String {
    let s: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect();
    if s.is_empty() { "unknown".into() } else { s }
}

/// One line in `problems.log` (kept to about 32 KiB, newest last) and one on stderr.
pub fn problem(dir: &Path, event: &str, sid: &str, msg: &str) {
    let msg: String = msg
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(400)
        .collect();
    eprintln!("torpor-muse: {event}: {msg}");
    let path = dir.join("problems.log");
    let sid: String = safe(sid).chars().take(8).collect();
    append(
        &path,
        format!("{} {event} {sid} {msg}\n", unix_now()).as_bytes(),
    );
    if fs::metadata(&path).is_ok_and(|m| m.len() > LOG_CAP)
        && let Ok(text) = fs::read_to_string(&path)
    {
        let mut cut = text.len() / 2;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        let tail = &text[cut..];
        let tail = tail.split_once('\n').map_or(tail, |x| x.1);
        let _ = crate::engine::write_private(&path, tail.as_bytes());
    }
}

/// True the first time `Event.key` is seen (and records it). The file stops growing at 16 KiB.
pub fn new_key(dir: &Path, event: &str, key: &str) -> bool {
    let path = dir.join("seen-keys");
    let line = format!("{event}.{key}");
    if fs::read_to_string(&path).is_ok_and(|t| t.lines().any(|l| l == line)) {
        return false;
    }
    if fs::metadata(&path).is_ok_and(|m| m.len() > KEYS_CAP) {
        return false;
    }
    append(&path, format!("{line}\n").as_bytes());
    true
}

/// Runs `f` on the state of session `sid` under an exclusive file lock (waits up to 500 ms) and saves the result.
/// None = the state could not be opened or locked; callers then do without it.
pub fn with_session<R>(dir: &Path, sid: &str, f: impl FnOnce(&mut Session) -> R) -> Option<R> {
    let sdir = dir.join("session");
    crate::engine::private_dir(&sdir).ok()?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(sdir.join(format!("{}.json", safe(sid))))
        .ok()?;
    let start = Instant::now();
    loop {
        match file.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if start.elapsed() < LOCK_WAIT => {
                sleep(Duration::from_millis(5))
            }
            Err(_) => return None,
        }
    }
    let mut text = String::new();
    file.read_to_string(&mut text).ok()?;
    let mut s: Session = serde_json::from_str(&text).unwrap_or_default();
    let r = f(&mut s);
    file.set_len(0).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    file.write_all(serde_json::to_string(&s).ok()?.as_bytes())
        .ok()?;
    Some(r)
}

/// SessionEnd: the session is over, its file is not needed.
pub fn end_session(dir: &Path, sid: &str) {
    let _ = fs::remove_file(dir.join("session").join(format!("{}.json", safe(sid))));
}

/// SessionStart: removes session files nobody has touched for 14 days (a crashed Muse never sends SessionEnd).
pub fn sweep(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir.join("session")) else {
        return;
    };
    let limit = Duration::from_secs(KEEP_DAYS * 86_400);
    for e in entries.flatten() {
        let old = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > limit);
        if old {
            let _ = fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::scratch;

    #[test]
    fn the_stop_cap_is_two_per_turn_and_old_turns_are_forgotten() {
        let mut s = Session::default();
        assert!(s.allow_block("t1"));
        assert!(s.allow_block("t1"));
        assert!(!s.allow_block("t1"), "third block in the same turn");
        assert!(s.allow_block("t2"), "a new turn starts at zero");
        for i in 0..20 {
            s.allow_block(&format!("x{i}"));
        }
        assert_eq!(s.blocks.len(), 8);
        assert!(s.allow_block("t1"), "t1 was forgotten");
    }

    #[test]
    fn session_state_is_saved_between_calls_and_ids_cannot_escape() {
        let dir = scratch("state-session");
        assert_eq!(with_session(&dir, "s/../1", |s| s.next_seq()), Some(1));
        assert_eq!(with_session(&dir, "s/../1", |s| s.next_seq()), Some(2));
        assert!(
            dir.join("session/s1.json").is_file(),
            "{:?}",
            fs::read_dir(&dir)
        );
        end_session(&dir, "s/../1");
        assert_eq!(with_session(&dir, "s/../1", |s| s.next_seq()), Some(1));
    }

    #[test]
    fn parallel_hooks_get_distinct_numbers() {
        let dir = scratch("state-parallel");
        let seen: Vec<u64> = (0..8)
            .map(|_| {
                let d = dir.clone();
                std::thread::spawn(move || with_session(&d, "p", |s| s.next_seq()).unwrap_or(0))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();
        let mut sorted = seen.clone();
        sorted.sort();
        assert_eq!(sorted, (1..=8).collect::<Vec<_>>(), "{seen:?}");
    }

    #[test]
    fn a_key_is_new_once_and_the_log_stays_small() {
        let dir = scratch("state-keys");
        assert!(new_key(&dir, "Stop", "brand_new"));
        assert!(!new_key(&dir, "Stop", "brand_new"));
        assert!(new_key(&dir, "Stop", "other"));
        for i in 0..3000 {
            problem(&dir, "Stop", "s", &format!("line {i} {}", "x".repeat(40)));
        }
        let len = fs::metadata(dir.join("problems.log")).unwrap().len();
        assert!(len <= LOG_CAP + 200, "{len}");
        let text = fs::read_to_string(dir.join("problems.log")).unwrap();
        assert!(text.contains("line 2999"), "newest lines are kept");
    }
}

//! Our own small files in `<TORPOR_HOME>/muse/` (best effort: a failure here never stops a hook):
//! `problems.log` (read by doctor), `seen-keys` (payload keys we have no field for), `session/<id>.json`,
//! `open/<hash of folder>.json` (the main turn open in a folder, for a subagent's tool events), `db/<hash of
//! folder>.json` (the folder's last turns that changed a database, kept past SessionEnd).

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
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
    /// The newest database turn of the folder the model of this session was told about (`hook::db_note`).
    #[serde(default)]
    pub db_told: u64,
    /// The newest promise whose not-covered text the model of this session was given (`hook::gap_note`).
    #[serde(default)]
    pub gap_told: u32,
}

/// A turn whose own commands changed or deleted a database file: its number, its start save point, the files.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DbTurn {
    pub n: u64,
    pub p: u64,
    pub files: Vec<String>,
}

/// The database turns of one folder, newest first. The folder is stored inside and compared whole.
#[derive(Serialize, Deserialize, Default)]
struct DbTurns {
    cwd: String,
    turns: Vec<DbTurn>,
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
    let flat = |s: &str| -> String {
        s.chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .take(400)
            .collect()
    };
    let (event, msg) = (flat(event), flat(msg));
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
    locked(&dir.join("session").join(format!("{}.json", safe(sid))), f)
}

/// Adds turn `t` to the database turns of folder `cwd` (the last 5, newest first; a turn already there stays once).
pub fn db_add(dir: &Path, cwd: &str, t: DbTurn) {
    locked(&folder_file(dir, "db", cwd), |d: &mut DbTurns| {
        if d.cwd != cwd {
            *d = DbTurns {
                cwd: cwd.into(),
                turns: Vec::new(),
            };
        }
        d.turns.retain(|x| x.n != t.n);
        d.turns.push(t);
        d.turns.sort_by_key(|x| std::cmp::Reverse(x.n));
        d.turns.truncate(5);
    });
}

/// The database turns of folder `cwd`, newest first.
pub fn db_turns(dir: &Path, cwd: &str) -> Vec<DbTurn> {
    fs::read(folder_file(dir, "db", cwd))
        .ok()
        .and_then(|b| serde_json::from_slice::<DbTurns>(&b).ok())
        .filter(|d| d.cwd == cwd)
        .map_or_else(Vec::new, |d| d.turns)
}

/// Runs `f` on the JSON value in `path` (a default one when there is none) under an exclusive file lock (waits up to
/// 500 ms) and saves the result. None = the file could not be opened or locked.
fn locked<T: Serialize + serde::de::DeserializeOwned + Default, R>(
    path: &Path,
    f: impl FnOnce(&mut T) -> R,
) -> Option<R> {
    crate::engine::private_dir(path.parent()?).ok()?;
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
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
    let mut s: T = serde_json::from_str(&text).unwrap_or_default();
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

/// SessionStart: removes the files of sessions (state, open turns, heartbeats, transcript copies) nobody has touched
/// for 14 days (a crashed Muse never sends SessionEnd).
pub fn sweep(dir: &Path) {
    let limit = Duration::from_secs(KEEP_DAYS * 86_400);
    for sub in ["session", "open", "db", "beats", "transcripts"] {
        let Ok(entries) = fs::read_dir(dir.join(sub)) else {
            continue;
        };
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
}

/// The main turn open in a folder. Muse gives a subagent's tool events no parent id (its session id and turn id are its
/// own), only the folder, so hook.rs files them under the turn recorded here. One file per folder, replaced whole
/// (the hooks of a parent and its subagents run at the same time).
#[derive(Serialize, Deserialize)]
struct Open {
    cwd: String,
    sid: String,
    turn: String,
    at: u64,
}

/// A turn open for longer than this is a crashed Muse's leftover, not a running turn.
const OPEN_MAX_S: u64 = 12 * 3600;

fn folder_file(dir: &Path, sub: &str, cwd: &str) -> PathBuf {
    // FNV-1a: the same hash on every run (std's hasher promises none); the folder is stored inside and compared whole.
    let h = cwd.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    dir.join(sub).join(format!("{h:016x}.json"))
}

/// A main session's UserPromptSubmit: `turn` of session `sid` is the one open in `cwd`.
pub fn open_set(dir: &Path, cwd: &str, sid: &str, turn: &str) {
    let o = Open {
        cwd: cwd.into(),
        sid: sid.into(),
        turn: turn.into(),
        at: unix_now(),
    };
    if let Ok(b) = serde_json::to_vec(&o) {
        let _ = crate::engine::write_private(&folder_file(dir, "open", cwd), &b);
    }
}

/// (session id, turn id) of the main turn open in `cwd`, if there is one.
pub fn open_get(dir: &Path, cwd: &str) -> Option<(String, String)> {
    let o: Open = serde_json::from_slice(&fs::read(folder_file(dir, "open", cwd)).ok()?).ok()?;
    (o.cwd == cwd && unix_now().saturating_sub(o.at) < OPEN_MAX_S).then_some((o.sid, o.turn))
}

/// The turn of `sid` is over (a Stop that let it end, StopFailure, SessionEnd). A record of another session stays.
pub fn open_clear(dir: &Path, cwd: &str, sid: &str) {
    let f = folder_file(dir, "open", cwd);
    let mine = fs::read(&f)
        .ok()
        .and_then(|b| serde_json::from_slice::<Open>(&b).ok())
        .is_some_and(|o| o.sid == sid);
    if mine {
        let _ = fs::remove_file(f);
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
    fn a_folder_has_one_open_turn_and_only_its_session_closes_it() {
        let dir = scratch("state-open");
        assert_eq!(open_get(&dir, "/w/a"), None);
        open_set(&dir, "/w/a", "s1", "t1");
        open_set(&dir, "/w/b", "s2", "t2");
        assert_eq!(open_get(&dir, "/w/a"), Some(("s1".into(), "t1".into())));
        assert_eq!(open_get(&dir, "/w/b"), Some(("s2".into(), "t2".into())));
        open_clear(&dir, "/w/a", "s2");
        assert!(open_get(&dir, "/w/a").is_some(), "another session's end");
        open_set(&dir, "/w/a", "s3", "t3");
        assert_eq!(open_get(&dir, "/w/a"), Some(("s3".into(), "t3".into())));
        open_clear(&dir, "/w/a", "s3");
        assert_eq!(open_get(&dir, "/w/a"), None);
        assert!(open_get(&dir, "/w/b").is_some(), "the other folder");
    }

    #[test]
    fn an_open_turn_from_a_crashed_muse_is_not_used_and_is_swept() {
        let dir = scratch("state-open-old");
        let old = Open {
            cwd: "/w".into(),
            sid: "s".into(),
            turn: "t".into(),
            at: unix_now() - OPEN_MAX_S - 1,
        };
        crate::engine::write_private(
            &folder_file(&dir, "open", "/w"),
            &serde_json::to_vec(&old).unwrap(),
        )
        .unwrap();
        assert_eq!(open_get(&dir, "/w"), None);
        // sweep: 15 days untouched goes, in every folder; a fresh file stays
        let stale = UNIX_EPOCH + Duration::from_secs(unix_now() - 15 * 86_400);
        let old_ones = [
            "db/old.json",
            "open/old.json",
            "session/old.json",
            "beats/old.jsonl",
            "transcripts/old.jsonl",
            "transcripts/old.state",
            "transcripts/old.lock",
        ];
        for f in old_ones.iter().chain(&["open/new.json", "beats/new.jsonl"]) {
            let p = dir.join(f);
            crate::engine::write_private(&p, b"{}").unwrap();
            if !f.contains("new.") {
                fs::File::open(&p).unwrap().set_modified(stale).unwrap();
            }
        }
        sweep(&dir);
        assert!(old_ones.iter().all(|f| !dir.join(f).exists()));
        assert!(dir.join("open/new.json").exists() && dir.join("beats/new.jsonl").exists());
    }

    #[test]
    fn a_problem_line_is_one_line_whatever_the_event_argument_holds() {
        let dir = scratch("state-problem-event");
        problem(&dir, "Bad\x1b[31m\nEvent", "s1", "wrong\tthing\r\n");
        let log = fs::read_to_string(dir.join("problems.log")).unwrap();
        assert_eq!(log.lines().count(), 1, "{log:?}");
        assert!(!log.chars().any(|c| c.is_control() && c != '\n'), "{log:?}");
        assert!(log.contains("Bad [31m Event s1 wrong thing"), "{log:?}");
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

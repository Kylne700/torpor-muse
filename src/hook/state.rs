//! Our own small files in `<TORPOR_HOME>/muse/` (best effort: a failure here never stops a hook):
//! `problems.log` (read by doctor), `seen-keys` (payload keys we have no field for), `session/<id>.json`,
//! `open/<hash of folder>.json` (the main turn open in a folder, for a subagent's tool events), `db/<hash of
//! folder>.json` (the folder's last turns that changed a database, kept past SessionEnd), `gate/<hash of folder>.json`
//! (the folder's engine gate was switched off once, `hook::gate_default`; never swept), `app/<hash of folder>.json`
//! (the user was told once that the check-writer needs an app with a start script; never swept) and
//! `no-key/<hash of folder>.json` (the same for the check-writer being off for lack of a key; never swept).

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

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
    /// The turn whose end Torpor blocked once for a broken check or a risky turn (`hook::stop_note`).
    #[serde(default)]
    pub noted: String,
    /// The kinds of check-writer news the user of this session was told, one note each (`hook::writer_note`); not
    /// `app` and `no-key`, which are marked per folder (`mark`) unless that is not possible.
    #[serde(default)]
    pub writer_told: Vec<String>,
    /// Why the check-writer could not start on the last prompt (`busy`, `start`), to tell at the next prompt.
    #[serde(default)]
    pub writer_skip: String,
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

    /// True once per `turn`: the one block of `hook::stop_note` may go out. It counts against the same cap as the
    /// engine's blocks; a turn without an id cannot be told apart from another, so it gets none.
    pub fn allow_note_block(&mut self, turn: &str) -> bool {
        if turn.is_empty() || self.noted == turn || !self.allow_block(turn) {
            return false;
        }
        self.noted = turn.to_string();
        true
    }
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
                let _ = crate::util::private_dir(d);
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
        format!("{} {event} {sid} {msg}\n", crate::util::now_secs()).as_bytes(),
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
    crate::util::private_dir(path.parent()?).ok()?;
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

/// A folder marked once for `sub`: `gate` (`hook::gate_default` switched its engine gate off), `app` (the user was told
/// the check-writer needs an app with a start script) or `no-key` (told it is off for lack of a key); stored inside and
/// compared whole. Kept without an end (`sweep` leaves `gate/`, `app/` and `no-key/` alone): the `gate` file is what
/// lets a later `torpor gate on` of the user's stay on.
#[derive(Serialize, Deserialize)]
struct Mark {
    cwd: String,
}

/// Was folder `cwd` marked for `sub`?
pub fn marked(dir: &Path, sub: &str, cwd: &str) -> bool {
    fs::read(folder_file(dir, sub, cwd))
        .ok()
        .and_then(|b| serde_json::from_slice::<Mark>(&b).ok())
        .is_some_and(|d| d.cwd == cwd)
}

/// Marks `cwd` for `sub`; false when that could not be written.
pub fn mark(dir: &Path, sub: &str, cwd: &str) -> bool {
    serde_json::to_vec(&Mark { cwd: cwd.into() })
        .is_ok_and(|b| crate::engine::write_private(&folder_file(dir, sub, cwd), &b).is_ok())
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
    // the folder is stored inside the file and compared whole
    dir.join(sub)
        .join(format!("{:016x}.json", crate::util::fnv1a(cwd.as_bytes())))
}

/// A main session's UserPromptSubmit: `turn` of session `sid` is the one open in `cwd`.
pub fn open_set(dir: &Path, cwd: &str, sid: &str, turn: &str) {
    let o = Open {
        cwd: cwd.into(),
        sid: sid.into(),
        turn: turn.into(),
        at: crate::util::now_secs(),
    };
    if let Ok(b) = serde_json::to_vec(&o) {
        let _ = crate::engine::write_private(&folder_file(dir, "open", cwd), &b);
    }
}

/// (session id, turn id) of the main turn open in `cwd`, if there is one.
pub fn open_get(dir: &Path, cwd: &str) -> Option<(String, String)> {
    let o: Open = serde_json::from_slice(&fs::read(folder_file(dir, "open", cwd)).ok()?).ok()?;
    (o.cwd == cwd && crate::util::now_secs().saturating_sub(o.at) < OPEN_MAX_S)
        .then_some((o.sid, o.turn))
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
mod tests;

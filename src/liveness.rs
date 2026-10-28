//! Per-event heartbeats and the turn-open/turn-closed counter (research/60 AD7, AD9). Owner: S3a-shim.
//!
//! Every event appends one short line to `<vault>/muse/beats/<session>.jsonl` (one O_APPEND write, so parallel hooks
//! cannot tear it). Nothing here fails the hook or writes to stdout: an error goes to `<vault>/muse/shim-errors.log`,
//! or to stderr when even that cannot be written. The counters are derived from the file when `doctor` asks, so the
//! hook path does no reading.

use crate::util::private_dir;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A session's heartbeat file is moved to `.1` once it passes this size; `read_beats` reads both.
/// shortcut: a session keeps its last 256-512 KiB of beats (a few thousand events), enough for doctor; raise if it is not.
const BEATS_MAX: u64 = 256 * 1024;
const ERRORS_MAX: u64 = 64 * 1024;
/// The session id doctor's own probe uses; reports leave it out so a probe never counts as a live turn.
pub const PROBE_SID: &str = "torpor-doctor-probe";

/// The store the engine uses: TORPOR_HOME when it is not empty, else `$HOME/.torpor` (same rule as the engine).
pub fn vault() -> PathBuf {
    match std::env::var_os("TORPOR_HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h),
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".torpor"),
    }
}

/// Our files: the engine skips every folder in the vault that is not a 16/20-hex workspace.
pub fn shim_dir(vault: &Path) -> PathBuf {
    vault.join("muse")
}

/// A session id is used in file names: plain characters only, so a hostile payload cannot leave the folder.
pub fn safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 80
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// Best effort, never fails: the line goes to `shim-errors.log` (cut when it passes 64 KiB), else to stderr.
pub fn log_error(vault: &Path, what: &str) {
    let dir = shim_dir(vault);
    let put = || -> std::io::Result<()> {
        private_dir(&dir)?;
        let f = dir.join("shim-errors.log");
        if fs::metadata(&f).is_ok_and(|m| m.len() > ERRORS_MAX) {
            fs::write(&f, b"")?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&f)?
            .write_all(format!("{} {}\n", now_ms(), what.replace('\n', " ")).as_bytes())
    };
    if put().is_err() {
        eprintln!("torpor-muse: {what}");
    }
}

/// src/hook/handle.rs calls this first for every event it parsed, observer child sessions too (`main` = false,
/// research/59 D10).
pub fn beat(event: &str, session_id: &str, main: bool) {
    let v = vault();
    match std::panic::catch_unwind(|| beat_in(&v, event, session_id, main, now_ms())) {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log_error(&v, &format!("beat {event} {session_id}: {e}")),
        Err(_) => log_error(&v, &format!("beat {event} {session_id}: panic")),
    }
}

pub fn beat_in(
    vault: &Path,
    event: &str,
    session_id: &str,
    main: bool,
    t_ms: u64,
) -> std::io::Result<()> {
    if !safe_id(session_id) {
        return Err(std::io::Error::other("session id is not a plain name"));
    }
    let dir = shim_dir(vault).join("beats");
    private_dir(&dir)?;
    let f = dir.join(format!("{session_id}.jsonl"));
    if fs::metadata(&f).is_ok_and(|m| m.len() > BEATS_MAX) {
        let _ = fs::rename(&f, dir.join(format!("{session_id}.jsonl.1")));
    }
    let ev: String = event
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(40)
        .collect();
    let line = format!("{{\"t\":{t_ms},\"e\":\"{ev}\",\"m\":{main}}}\n");
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(f)?
        .write_all(line.as_bytes())
}

// ---------------------------------------------------------------- reading (doctor)

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct Beat {
    pub t: u64,
    pub e: String,
    pub m: bool,
}

/// All beats of a session in order (the rotated half first). Lines that do not parse are skipped.
pub fn read_beats(vault: &Path, session_id: &str) -> Vec<Beat> {
    let dir = shim_dir(vault).join("beats");
    ["jsonl.1", "jsonl"]
        .iter()
        .filter_map(|ext| fs::read_to_string(dir.join(format!("{session_id}.{ext}"))).ok())
        .flat_map(|t| {
            t.lines()
                .filter_map(|l| serde_json::from_str::<Beat>(l).ok())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Session ids that have heartbeats, newest activity first.
pub fn sessions(vault: &Path) -> Vec<String> {
    let mut v: Vec<(SystemTime, String)> = fs::read_dir(shim_dir(vault).join("beats"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let sid = name.strip_suffix(".jsonl")?.to_string();
            Some((e.metadata().ok()?.modified().ok()?, sid))
        })
        .collect();
    v.sort_by(|a, b| b.cmp(a));
    v.into_iter().map(|(_, s)| s).collect()
}

/// A main session has beats whose last `m` is true; an observer child's are false.
pub fn is_main(beats: &[Beat]) -> bool {
    beats.last().is_some_and(|b| b.m)
}

/// Newest beat per event over the `limit` newest sessions: (main, observers) as event -> (time ms, session, count).
pub type Last = BTreeMap<String, (u64, String, u32)>;
pub fn last_by_event(vault: &Path, limit: usize) -> (Last, Last) {
    let (mut main, mut obs): (Last, Last) = (Last::new(), Last::new());
    for sid in sessions(vault).into_iter().take(limit) {
        for b in read_beats(vault, &sid) {
            let into = if b.m { &mut main } else { &mut obs };
            let e = into.entry(b.e).or_insert((0, sid.clone(), 0));
            e.2 += 1;
            if b.t >= e.0 {
                e.0 = b.t;
                e.1 = sid.clone();
            }
        }
    }
    (main, obs)
}

/// What the Stop/UserPromptSubmit pairs of one session say (AD9).
#[derive(Default, Debug, PartialEq, Clone)]
pub struct Turns {
    /// UserPromptSubmit beats.
    pub opened: u32,
    /// Stop or StopFailure beats that closed an open turn.
    pub closed: u32,
    /// A turn is open now (its Stop may still come).
    pub open: bool,
    /// Turns that ended without a Stop: the next prompt or the SessionEnd came first (interrupted, step limit, lost hook).
    pub no_stop: u32,
    /// Stops that found no open turn: Muse reruns the turn after a blocking Stop, so these are normal and never flagged.
    pub extra_stops: u32,
}

pub fn turns(beats: &[Beat]) -> Turns {
    let mut t = Turns::default();
    for b in beats {
        match b.e.as_str() {
            "UserPromptSubmit" => {
                if t.open {
                    t.no_stop += 1;
                }
                t.opened += 1;
                t.open = true;
            }
            "Stop" | "StopFailure" if t.open => {
                t.closed += 1;
                t.open = false;
            }
            "Stop" | "StopFailure" => t.extra_stops += 1,
            "SessionEnd" if t.open => {
                t.no_stop += 1;
                t.open = false;
            }
            _ => {}
        }
    }
    t
}

/// The gate is live when a Stop closed a turn: its hook ran, and the engine answered in time for Muse to take it.
pub fn gate_live(beats: &[Beat]) -> bool {
    turns(beats).closed >= 1
}

const PAIRS: [(&str, &[&str]); 2] = [
    ("PreToolUse", &["PostToolUse", "PostToolUseFailure"]),
    ("SubagentStart", &["SubagentStop"]),
];

/// Events that arrived without their pair, as report lines. `armed(event)` says the hook for that event is installed:
/// an event nobody listens to has no beat and proves nothing. One opening more than its closings may still be running.
pub fn unpaired(beats: &[Beat], armed: impl Fn(&str) -> bool) -> Vec<String> {
    let n = |names: &[&str]| {
        beats
            .iter()
            .filter(|b| names.contains(&b.e.as_str()))
            .count()
    };
    let mut out = Vec::new();
    for (before, afters) in PAIRS {
        if !armed(before) || !afters.iter().all(|a| armed(a)) {
            continue;
        }
        let (b, a) = (n(&[before]), n(afters));
        if b > a + 1 {
            out.push(format!("{} {before} without {}", b - a, afters.join("/")));
        } else if a > b {
            out.push(format!("{} {} without {before}", a - b, afters.join("/")));
        }
    }
    out
}

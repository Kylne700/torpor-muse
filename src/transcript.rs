//! Claude-style transcript for the engine. Muse sends `transcript_path: null` (research/59 D3), but the engine reads a
//! transcript for the human origin of promise words, compaction boundaries and the delivery check (research/60 AD4).
//! This module keeps one per main session, translated from Muse's own session log (never invented). Owner: S3a-shim.
//!
//! Muse's log (`<data>/muse/sessions/Y/M/D/<session-id>/session.jsonl`, one JSON record per line, observed on 1.4.13):
//! - `runtime.user_intent.accepted` is a prompt. It is written before the UserPromptSubmit hook runs. Muse does not say
//!   whether a person typed it: only `semantic_kind` chat / skill_invocation / plugin_command on surface `main` (the
//!   person-initiated kinds) become `human`/`typed`; any other kind is labeled `unknown`, which the engine reads as
//!   "not human". The prompt text is never copied.
//! - `context_compaction_installed` is a finished compaction (soft, hard or manual): one `compact_boundary` line. The
//!   candidate, checkpoint and `session.fork.compaction` records are not boundaries.
//! - `context_block_updated` / `context_block_update_batch_committed` with `source: runtime_hook` hold the exact
//!   additionalContext Muse took from a hook: one `hook_additional_context` attachment line each. No such record, no
//!   attachment: the engine's delivery check fails when Muse did not take the text.
//!
//! Files in `<vault>/muse/transcripts/`: `<sid>.jsonl` (the transcript, append only: the engine keeps byte offsets),
//! `<sid>.state` (where the translation stands and what it has seen), `<sid>.lock`.
#![allow(dead_code)] // hook.rs does not call path_for yet

use crate::liveness::{log_error, mkdir_private, safe_id, shim_dir, vault};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// A first look at a log longer than this reads only its newest part: older prompts and compactions are history.
const COLD_TAIL: u64 = 8 << 20;
/// At most this much new log per call; the rest waits for the next call (offsets only move over whole lines).
const CALL_MAX: u64 = 32 << 20;
/// A longer line is skipped unread (the longest seen on 1.4.13 is 3 MB: a 1.5 MB prompt, twice).
const LINE_MAX: usize = 16 << 20;
const LOCK_WAIT: Duration = Duration::from_millis(300);

/// Where the translation of one session stands, and what it has seen (doctor prints this).
#[derive(Default, Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct State {
    /// Muse's log for this session.
    pub log: String,
    /// Next unread byte of that log.
    pub off: u64,
    /// Length of our transcript after the last whole batch (a longer file is a batch whose state was lost).
    pub tlen: u64,
    pub lines: u64,
    pub users: u64,
    pub bounds: u64,
    pub ctx: u64,
    /// Relevant log lines that did not have the expected shape.
    pub bad: u64,
}

pub struct Paths {
    pub vault: PathBuf,
    /// Muse's `sessions` folder. Hooks run with a cleared environment, so this is `$HOME/.local/share/muse/sessions`
    /// even when Muse itself was started with XDG_DATA_HOME (doctor warns about that).
    pub sessions: PathBuf,
}

impl Paths {
    pub fn from_env() -> Paths {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        Paths {
            vault: vault(),
            sessions: home.join(".local/share/muse/sessions"),
        }
    }
}

fn dir_of(vault: &Path) -> PathBuf {
    shim_dir(vault).join("transcripts")
}

pub fn transcript_file(vault: &Path, sid: &str) -> PathBuf {
    dir_of(vault).join(format!("{sid}.jsonl"))
}

pub fn read_state(vault: &Path, sid: &str) -> Option<State> {
    let b = fs::read(dir_of(vault).join(format!("{sid}.state"))).ok()?;
    serde_json::from_slice(&b).ok()
}

/// hook.rs calls this on every main-session event before the engine call; Some(path) = put it in `transcript_path`.
/// None only when no transcript exists (no Muse log was found): the engine then does nothing with transcripts.
pub fn path_for(event: &str, payload: &Value) -> Option<String> {
    path_for_in(&Paths::from_env(), event, payload)
}

/// `path_for` with the places given (the tests use their own).
pub fn path_for_in(p: &Paths, event: &str, payload: &Value) -> Option<String> {
    let sid = payload.get("session_id")?.as_str().filter(|s| safe_id(s))?;
    let failed = match std::panic::catch_unwind(|| translate(p, sid)) {
        Ok(Ok(_)) => None,
        Ok(Err(e)) => Some(e),
        Err(_) => Some("panic".to_string()),
    };
    if let Some(e) = failed {
        log_error(&p.vault, &format!("transcript {event} {sid}: {e}"));
    }
    // An old transcript is still better than none: the engine's delivery check then fails instead of passing.
    let t = transcript_file(&p.vault, sid);
    t.is_file().then(|| t.to_string_lossy().into_owned())
}

/// Brings `<vault>/muse/transcripts/<sid>.jsonl` up to date with Muse's log. Err = it could not (the old file stays).
pub fn translate(p: &Paths, sid: &str) -> Result<PathBuf, String> {
    let dir = dir_of(&p.vault);
    mkdir_private(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let tpath = dir.join(format!("{sid}.jsonl"));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(dir.join(format!("{sid}.lock")))
        .map_err(|e| format!("lock file: {e}"))?;
    let t0 = Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if t0.elapsed() < LOCK_WAIT => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(TryLockError::WouldBlock) => {
                return Err(
                    "another hook holds the translation lock; kept the file as it is".into(),
                );
            }
            Err(TryLockError::Error(e)) => return Err(format!("lock: {e}")),
        }
    }
    let spath = dir.join(format!("{sid}.state"));
    let mut st: State = fs::read(&spath)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut log = PathBuf::from(&st.log);
    let mut reset = !log.is_file();
    if reset {
        log = find_log(&p.sessions, sid)
            .ok_or_else(|| format!("no session log of {sid} under {}", p.sessions.display()))?;
    }
    let log_len = fs::metadata(&log)
        .map_err(|e| format!("{}: {e}", log.display()))?
        .len();
    reset |= st.off > log_len; // the log was replaced: start over (the engine copes with a shorter file)
    if reset {
        st = State {
            log: log.to_string_lossy().into_owned(),
            ..State::default()
        };
        if tpath.is_file() {
            File::create(&tpath).map_err(|e| format!("reset transcript: {e}"))?;
        }
    } else {
        // A batch whose state was not saved (crash between the two writes): cut back to the saved length and redo it.
        let tl = fs::metadata(&tpath).map_or(0, |m| m.len());
        if tl > st.tlen {
            OpenOptions::new()
                .write(true)
                .open(&tpath)
                .and_then(|f| f.set_len(st.tlen))
                .map_err(|e| format!("cut transcript: {e}"))?;
        } else {
            st.tlen = tl;
        }
    }

    // Nothing translated yet and a long log: only its newest part (older prompts are history). Later calls go on from
    // the offset, at most CALL_MAX at a time.
    let (mut pos, align) = if st.off == 0 && log_len > COLD_TAIL {
        (log_len - COLD_TAIL - 1, true) // one byte early: when that is a line end, the first line of the tail is whole
    } else {
        (st.off, false)
    };
    let mut f = File::open(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    f.seek(SeekFrom::Start(pos))
        .map_err(|e| format!("seek: {e}"))?;
    let mut rd = BufReader::with_capacity(1 << 20, f);
    let mut out = Vec::new();
    let mut ok = true;
    if align {
        // Landed inside a line: drop the rest of it. No line end in the whole tail: wait for one.
        match skip_line(&mut rd).map_err(|e| format!("read: {e}"))? {
            Some(n) => pos += n,
            None => (ok, pos) = (false, st.off),
        }
    }
    let (mut buf, limit) = (Vec::new(), pos.saturating_add(CALL_MAX));
    while ok && pos < limit {
        buf.clear();
        let n = (&mut rd)
            .take(LINE_MAX as u64 + 1)
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("read: {e}"))?;
        if buf.last() == Some(&b'\n') {
            pos += n as u64;
            st.lines += 1;
            if let Ok(s) = std::str::from_utf8(&buf[..n - 1]) {
                translate_line(s, &mut out, &mut st);
            }
        } else if n > LINE_MAX {
            match skip_line(&mut rd).map_err(|e| format!("read: {e}"))? {
                Some(k) => {
                    pos += n as u64 + k;
                    st.lines += 1;
                    st.bad += 1;
                }
                None => break,
            }
        } else {
            break; // end of file, or a line Muse is still writing: only whole lines are consumed
        }
    }
    if !out.is_empty() || !tpath.is_file() {
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&tpath)
            .and_then(|mut f| f.write_all(&out))
            .map_err(|e| format!("append transcript: {e}"))?;
    }
    st.off = pos;
    st.tlen = fs::metadata(&tpath).map_or(0, |m| m.len());
    let tmp = dir.join(format!("{sid}.state.tmp"));
    OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(&tmp)
        .and_then(|mut f| f.write_all(&serde_json::to_vec(&st).unwrap_or_default()))
        .and_then(|()| fs::rename(&tmp, &spath))
        .map_err(|e| format!("save state: {e}"))?;
    Ok(tpath)
}

/// Consumes up to and including the next newline. None: the file ended first.
fn skip_line<R: BufRead>(r: &mut R) -> std::io::Result<Option<u64>> {
    let mut n = 0u64;
    loop {
        let b = r.fill_buf()?;
        if b.is_empty() {
            return Ok(None);
        }
        let (used, found) = match b.iter().position(|c| *c == b'\n') {
            Some(i) => (i + 1, true),
            None => (b.len(), false),
        };
        r.consume(used);
        n += used as u64;
        if found {
            return Ok(Some(n));
        }
    }
}

/// Sub-folders of `dir`, newest first. `numeric`: only the all-digit names of the year, month and day folders
/// (Muse keeps hidden folders for locks and markers next to them); session folders are UUIDv7, so newest-first by name.
pub fn children_desc(dir: &Path, numeric: bool) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let name = name.as_encoded_bytes();
            if numeric {
                name.iter().all(u8::is_ascii_digit)
            } else {
                name.first() != Some(&b'.')
            }
        })
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    v.sort_by(|a, b| b.cmp(a));
    v
}

/// The folder Muse made for a session. The day folder is UTC or local time (not known), so look through all of them,
/// newest first: a recent session is found in the first few steps.
pub fn find_log(root: &Path, sid: &str) -> Option<PathBuf> {
    for y in children_desc(root, true) {
        for m in children_desc(&y, true) {
            for d in children_desc(&m, true) {
                let p = d.join(sid).join("session.jsonl");
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// The newest `n` session logs under `root`, newest first.
pub fn recent_logs(root: &Path, n: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for y in children_desc(root, true) {
        for m in children_desc(&y, true) {
            for d in children_desc(&m, true) {
                for s in children_desc(&d, false) {
                    let p = s.join("session.jsonl");
                    if p.is_file() {
                        out.push(p);
                        if out.len() >= n {
                            return out;
                        }
                    }
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------- one log line

/// Cheap test on the text first: most lines are of no interest, and a prompt line can be megabytes. The records are
/// then told apart by what they say about themselves, not by this text: a compaction checkpoint and `session.fork.
/// compaction` carry a whole accepted record inside. A record stored inside a `retained_frame` has escaped quotes and
/// does not pass (none of the wanted kinds were seen in a frame on 1.4.13).
fn wanted(s: &str) -> bool {
    s.contains(r#""payload_type":"runtime.user_intent.accepted""#)
        || (s.contains(r#""payload_type":"runtime.session""#)
            && [
                r#""kind":"context_compaction_installed""#,
                r#""kind":"context_block_updated""#,
                r#""kind":"context_block_update_batch_committed""#,
            ]
            .iter()
            .any(|k| s.contains(k)))
}

/// Only what is needed: the prompt text, the model messages and the replayed messages (megabytes) are skipped.
#[derive(Deserialize)]
struct Rec {
    id: Option<String>,
    payload_type: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    intent_id: Option<String>,
    semantic_kind: Option<Sem>,
    surface: Option<String>,
    event: Option<Event>,
}

#[derive(Deserialize)]
struct Sem {
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Event {
    kind: Option<String>,
    trigger: Option<String>,
    source: Option<String>,
    text: Option<String>,
    entries: Option<Vec<Entry>>,
}

#[derive(Deserialize)]
struct Entry {
    update: Option<Block>,
}

#[derive(Deserialize)]
struct Block {
    source: Option<String>,
    text: Option<String>,
}

fn put(v: Value, out: &mut Vec<u8>) {
    out.extend_from_slice(v.to_string().as_bytes());
    out.push(b'\n');
}

fn translate_line(s: &str, out: &mut Vec<u8>, st: &mut State) {
    if !wanted(s) {
        return;
    }
    let Ok(rec) = serde_json::from_str::<Rec>(s) else {
        st.bad += 1;
        return;
    };
    let (Some(id), Some(pl)) = (rec.id, rec.payload) else {
        return;
    };
    match (rec.payload_type.as_deref(), pl.intent_id, pl.event) {
        (Some("runtime.user_intent.accepted"), Some(intent), _) => {
            let surface = pl.surface.as_deref();
            let sem = pl.semantic_kind.and_then(|k| k.kind).unwrap_or_default();
            let human = surface == Some("main")
                && matches!(sem.as_str(), "chat" | "skill_invocation" | "plugin_command");
            let mut v = json!({
                "type": "user", "uuid": intent, "promptId": intent,
                "isSidechain": surface.is_some_and(|s| s != "main"),
                "turnOrigin": if human { "human" } else { "unknown" },
            });
            if human {
                v["promptSource"] = json!("typed");
            }
            st.users += 1;
            put(v, out);
        }
        (Some("runtime.user_intent.accepted"), None, _) => st.bad += 1,
        (Some("runtime.session"), _, Some(ev)) => match ev.kind.as_deref() {
            Some("context_compaction_installed") => {
                let manual = ev.trigger.as_deref() == Some("manual_command");
                st.bounds += 1;
                put(
                    json!({
                        "type": "system", "subtype": "compact_boundary", "uuid": id,
                        "content": "Conversation compacted",
                        "compactMetadata": { "trigger": if manual { "manual" } else { "auto" } },
                    }),
                    out,
                );
            }
            Some("context_block_updated") => context(&ev.source, &ev.text, &id, out, st),
            Some("context_block_update_batch_committed") => {
                for (i, e) in ev.entries.iter().flatten().enumerate() {
                    if let Some(b) = &e.update {
                        context(&b.source, &b.text, &format!("{id}:{i}"), out, st);
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }
}

/// A block Muse took from a hook: the attachment the engine's delivery check looks for.
fn context(
    source: &Option<String>,
    text: &Option<String>,
    uuid: &str,
    out: &mut Vec<u8>,
    st: &mut State,
) {
    if source.as_deref() != Some("runtime_hook") {
        return; // session_identity and the like are Muse's own, not a hook's
    }
    let Some(text) = text else {
        st.bad += 1;
        return;
    };
    st.ctx += 1;
    put(
        json!({
            "type": "attachment", "uuid": uuid,
            "attachment": { "type": "hook_additional_context", "content": [text] },
        }),
        out,
    );
}

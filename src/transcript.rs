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

#[path = "transcript/line.rs"]
mod line;
#[path = "transcript/translate.rs"]
mod translate;

pub use translate::translate;

use crate::liveness::{log_error, safe_id, shim_dir, vault};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

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

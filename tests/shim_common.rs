//! Helpers the shim tests share (each shim_*.rs includes this file with #[path]). Cargo also builds it as a test crate
//! of its own with no tests in it, which is harmless.
#![allow(dead_code)]

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// A temp folder that is removed when the test ends (it lives in `std::env::temp_dir()`).
pub struct Tmp(pub PathBuf);

impl Tmp {
    pub fn new(name: &str) -> Tmp {
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "torpor-shim-{}-{}-{name}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    pub fn join(&self, s: impl AsRef<Path>) -> PathBuf {
        self.0.join(s)
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub const FIXTURES: [&str; 9] = [
    "batch",
    "clear_resume",
    "fork",
    "fork_compaction",
    "hard",
    "manual",
    "noise",
    "rejected",
    "soft",
];

/// A redacted real Muse log (tests/fixtures/shim/<name>.jsonl). Home paths in it are written `<HOME>`.
pub fn fixture(name: &str, home: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/shim")
        .join(format!("{name}.jsonl"));
    std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("{}: {e}", p.display()))
        .replace("<HOME>", home)
}

/// The session id a log is about (the stream of its first record).
pub fn sid_of(text: &str) -> String {
    let first: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    first["stream"]["id"].as_str().unwrap().to_string()
}

/// Muse keeps a session at `<sessions>/YYYY/MM/DD/<session id>/session.jsonl`.
pub fn put_log(sessions: &Path, sid: &str, text: &str) -> PathBuf {
    let dir = sessions.join("2026/10/26").join(sid);
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("session.jsonl");
    std::fs::write(&f, text).unwrap();
    f
}

pub fn append(path: &Path, text: &str) {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(text.as_bytes())
        .unwrap();
}

/// The JSON lines of a file.
pub fn lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad line {l:?}: {e}")))
        .collect()
}

/// Builds records in the shape Muse 1.4.13 writes them (copied from real logs: see tests/fixtures/shim). Ids are
/// made up; the intent ids look like UUIDv7 (version digit 7), as those of live prompts do.
pub struct LogBuilder {
    pub sid: String,
    seq: u64,
    lines: Vec<String>,
    taken: usize,
}

impl LogBuilder {
    pub fn new(sid: &str) -> LogBuilder {
        LogBuilder {
            sid: sid.to_string(),
            seq: 0,
            lines: Vec::new(),
            taken: 0,
        }
    }

    /// The records made since the last call (or since the start), for appending to a log that grows.
    pub fn new_text(&mut self) -> String {
        let s = self.lines[self.taken..]
            .iter()
            .map(|l| format!("{l}\n"))
            .collect();
        self.taken = self.lines.len();
        s
    }

    fn id(&self, version: char) -> String {
        format!("{:08x}-0000-{version}000-8000-{:012x}", self.seq, self.seq)
    }

    /// Returns the record id.
    fn push(&mut self, payload_type: &str, payload: Value, cause: Option<&str>) -> String {
        self.seq += 1;
        let id = self.id('7');
        let rec = json!({
            "schema_version": 1, "id": id, "stream": {"id": self.sid, "kind": "session"},
            "sequence": self.seq, "recorded_at": 1_793_038_390_000_000u64 + self.seq * 1000,
            "record_type": "event", "durability": "durable", "causation_id": cause,
            "payload_type": payload_type, "payload_schema_version": 1, "payload": payload,
        });
        self.lines.push(rec.to_string());
        id
    }

    pub fn metadata(&mut self, semver: &str) {
        self.push(
            "runtime.session.metadata",
            json!({"record": {"build": {"semver": semver}}}),
            None,
        );
    }

    /// A prompt Muse accepted; returns its intent id (also the turn id of the hooks that follow).
    pub fn accepted(&mut self, kind: &str, surface: &str) -> String {
        self.seq += 1; // the intent id is not the record id
        let intent = self.id('7');
        let payload = json!({
            "bindings": [], "delivery_policy": "session_current", "intent_id": intent,
            "model_messages": [], "refill_blocks": [], "semantic_kind": {"kind": kind},
            "source_session_id": self.sid, "surface": surface, "wake_policy": "start_once",
        });
        self.push("runtime.user_intent.accepted", payload, Some(&intent));
        intent
    }

    /// An event of the session stream; returns the record id.
    pub fn event(&mut self, run: &str, event: Value) -> String {
        let payload = json!({
            "event": event, "kind": "run", "run_id": run,
            "source_run_record_id": "00000000-0000-4000-8000-000000000000", "source_run_record_sequence": 1,
        });
        self.push("runtime.session", payload, None)
    }

    /// trigger: hard_threshold_blocking | soft_threshold_async | manual_command
    pub fn installed(&mut self, run: &str, trigger: &str) -> String {
        self.event(
            run,
            json!({
                "kind": "context_compaction_installed", "candidate_id": "candidate-1", "install_id": "install-2",
                "replacement": {"kind": "summary_text", "text": "<synthetic summary>"},
                "strategy_id": "summary-preserved-suffix/v1", "timing": "mid_turn", "trigger": trigger,
            }),
        )
    }

    /// A block of context: source `runtime_hook` is a hook's additionalContext, `session_identity` is Muse's own.
    pub fn cbu(&mut self, run: &str, source: &str, text: &str) -> String {
        self.event(
            run,
            json!({
                "kind": "context_block_updated", "id": "block-1", "lifecycle": "turn", "order": 1,
                "reason": "hook", "role": "user", "source": source, "text": text,
            }),
        )
    }

    pub fn batch(&mut self, run: &str, entries: &[(&str, &str)]) -> String {
        let entries: Vec<Value> = entries
            .iter()
            .map(|(source, text)| {
                json!({
                    "recorded_at": 1, "source_run_record_id": "00000000-0000-4000-8000-000000000000",
                    "source_run_record_sequence": 1,
                    "update": {"id": "block-2", "lifecycle": "turn", "order": 2, "reason": "hook", "role": "user",
                               "source": source, "text": text},
                })
            })
            .collect();
        self.event(
            run,
            json!({"kind": "context_block_update_batch_committed", "entries": entries}),
        )
    }

    /// Muse's record of one hook run. `hook_key` starts with the settings file path; `status` is completed, failed,
    /// timed_out, blocked or cancelled.
    pub fn terminal(
        &mut self,
        run: &str,
        hook_key: &str,
        event: &str,
        status: &str,
        error: Option<&str>,
        stdout: &str,
    ) {
        self.event(
            run,
            json!({
                "kind": "hook_run_terminal", "display_order": 1, "duration_ms": 12, "error": error,
                "event": event, "exit_code": 0, "hook_key": hook_key, "run_id": format!("{hook_key}:3"),
                "status": status, "stderr": "", "stdout": stdout,
            }),
        );
    }

    pub fn text(&self) -> String {
        self.lines.iter().map(|l| format!("{l}\n")).collect()
    }
}

/// Pads a record with filler so that a log reaches a size (a made-up event Muse's translator must skip).
pub fn filler_line(bytes: usize) -> String {
    let rec = json!({
        "schema_version": 1, "id": "ffffffff-0000-7000-8000-000000000000", "sequence": 1,
        "payload_type": "runtime.session", "payload": {"event": {"kind": "tool_output", "output": "x".repeat(bytes)}},
    });
    format!("{rec}\n")
}

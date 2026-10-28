//! What the done check reads from a hook payload: a Bash command and its exit code, an edit, the AI's last message.
//! Runs on the payload as Muse sent it, before `normalise::apply` drops the tool result.

use super::commands::{self, Seen};
use crate::hook::state;
use crate::suggest;
use serde_json::Value;
use std::path::Path;

/// The most of the AI's last message that is read: the end of it.
const READ: usize = 20_000;

/// Keeps what the event says in the session's log. A log that cannot be locked in time misses it (see the module docs).
pub(in crate::hook) fn record(dir: &Path, event: &str, sid: &str, p: &Value) {
    let turn = p["turn_id"].as_str().unwrap_or("");
    let note = |f: &dyn Fn(&mut super::Log)| {
        state::with_session(dir, sid, |s| f(&mut s.done));
    };
    match (event, p["tool_name"].as_str()) {
        ("SessionStart", _) => note(&|l| l.start(p["source"].as_str().unwrap_or(""))),
        ("UserPromptSubmit", _) => note(&|l| l.see(turn)),
        ("PostToolUse" | "PostToolUseFailure", Some("bash")) => {
            let Some(cmd) = p["tool_input"]["command"].as_str() else {
                return;
            };
            let (tests, build) = commands::read(cmd, exit_code(event, p));
            if (tests, build) != (Seen::No, Seen::No) {
                let cmd = suggest::scrub(cmd, 80).replace('`', "'");
                note(&|l| {
                    l.see(turn);
                    l.ran(tests, build, &cmd);
                });
            }
        }
        ("PostToolUse", Some("edit_file" | "write_file")) if code_edit(p) => note(&|l| {
            l.see(turn);
            l.edited();
        }),
        _ => {}
    }
}

/// The AI's last message of a Stop, to read for a claim. Nothing for the Stop that follows a block (`stop_hook_active`):
/// the block was this check's one.
pub(in crate::hook) fn said(p: &Value) -> String {
    if p["stop_hook_active"].as_bool() == Some(true) {
        return String::new();
    }
    let m = p["last_assistant_message"].as_str().unwrap_or("");
    let from = m.char_indices().rev().nth(READ - 1).map_or(0, |(i, _)| i);
    m[from..].to_owned()
}

/// The exit code of a finished Bash command, if Muse says so in the plain form: a command that was interrupted, is still
/// running, was stopped by the sandbox or has a code outside 0..=255 is not known.
pub(super) fn exit_code(event: &str, p: &Value) -> Option<i64> {
    let raw = if event == "PostToolUseFailure" {
        &p["error"]
    } else {
        &p["tool_response"]
    };
    let v = match raw {
        Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
        o @ Value::Object(_) => o.clone(),
        _ => return None,
    };
    let reason = v["terminal_reason"].as_str();
    let plain = match v["terminal_status"].as_str() {
        Some("completed") => true,
        Some("failed") => reason.is_none_or(|r| r.starts_with("process exited with status")),
        _ => false,
    };
    let stopped = p["is_interrupt"] == true || v["is_interrupt"] == true;
    v["exit_code"]
        .as_i64()
        .filter(|c| plain && !stopped && (0..=255).contains(c))
}

/// Is the edit one to code, in the project? Not a document (`.md`, `.txt`, ...), not a file of a dependency or of git,
/// not a file outside the folder the session works in.
///
/// shortcut: a file name decides, not its content, so a `.txt` that a build reads (`requirements.txt`) is a document here
/// and an edit to it does not count; upgrade: when a miss like that shows up in a drill.
pub(super) fn code_edit(p: &Value) -> bool {
    let path = p["tool_input"]["path"]
        .as_str()
        .or_else(|| p["tool_response"]["filePath"].as_str())
        .unwrap_or("");
    let cwd = p["cwd"].as_str().unwrap_or("").trim_end_matches('/');
    let rel = match path.strip_prefix('/') {
        Some(_) => match path.strip_prefix(cwd).and_then(|r| r.strip_prefix('/')) {
            Some(r) if !cwd.is_empty() => r,
            _ => return false,
        },
        None => path,
    };
    let parts: Vec<&str> = rel.split('/').collect();
    let name = parts.last().copied().unwrap_or("").to_lowercase();
    let stem = name.split('.').next().unwrap_or("");
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    !rel.is_empty()
        && !parts
            .iter()
            .any(|c| matches!(*c, ".." | ".git" | "node_modules"))
        && !matches!(ext, "md" | "mdx" | "markdown" | "txt" | "rst" | "adoc")
        && !matches!(
            stem,
            "license" | "notice" | "authors" | "changelog" | "copying"
        )
        && name != ".gitignore"
}

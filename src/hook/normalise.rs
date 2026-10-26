//! Muse payload -> the payload the Torpor engine expects (research/60 AD1, research/59 D11).
//! Only renames and reshapes; every other key goes through untouched.

use serde_json::{Map, Value};

/// Muse tool name -> the name the engine matches. A name already in the engine's form passes through.
pub fn tool_name(muse: &str) -> &str {
    match muse {
        "bash" => "Bash",
        "edit_file" => "Edit",
        "write_file" => "Write",
        "read_file" => "Read",
        "search" => "Grep",
        other => other,
    }
}

/// Muse's bash failure `error` is a JSON string with `exit_code` and `output`; the engine reads "Exit code N" on the
/// first line and the output after it. None = not that shape (a sandbox block, plain text): leave it as it is.
fn bash_error(error: &str) -> Option<String> {
    let v: Value = serde_json::from_str(error).ok()?;
    let code = v.get("exit_code")?.as_i64()?;
    let text = ["output", "terminal_reason"]
        .iter()
        .filter_map(|k| v.get(*k)?.as_str())
        .find(|s| !s.is_empty())
        .unwrap_or("");
    Some(format!("Exit code {code}\n{text}"))
}

/// Edits `p` in place for `torpor hook <event>`:
/// - `prompt_id` = Muse `turn_id` (one id per user prompt; Stop continuations keep it, so the engine's two-block cap
///   counts the right chain);
/// - tool events: engine tool names, bash failures as "Exit code N\n...", and no `tool_response` / file bodies
///   (the engine reads none of them, and a big file would push the payload over its 8 MiB limit).
pub fn apply(event: &str, p: &mut Map<String, Value>) {
    if let Some(t) = p.get("turn_id").and_then(Value::as_str).map(str::to_owned) {
        p.entry("prompt_id").or_insert(Value::String(t));
    }
    if !matches!(event, "PreToolUse" | "PostToolUse" | "PostToolUseFailure") {
        return;
    }
    let name = p
        .get("tool_name")
        .and_then(Value::as_str)
        .map(|n| tool_name(n).to_owned());
    if let Some(n) = &name {
        p.insert("tool_name".into(), n.clone().into());
    }
    p.remove("tool_response");
    if name.as_deref() != Some("Bash")
        && let Some(Value::Object(input)) = p.get_mut("tool_input")
    {
        for k in ["content", "find", "replace"] {
            input.remove(k);
        }
    }
    if event == "PostToolUseFailure"
        && name.as_deref() == Some("Bash")
        && let Some(e) = p.get("error").and_then(Value::as_str).and_then(bash_error)
    {
        p.insert("error".into(), e.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(event: &str, v: Value) -> Value {
        let Value::Object(mut m) = v else { panic!() };
        apply(event, &mut m);
        Value::Object(m)
    }

    #[test]
    fn names_follow_the_engine() {
        let names = [
            ("bash", "Bash"),
            ("edit_file", "Edit"),
            ("write_file", "Write"),
            ("read_file", "Read"),
            ("search", "Grep"),
            ("Bash", "Bash"),
            ("something_new", "something_new"),
        ];
        for (muse, engine) in names {
            assert_eq!(tool_name(muse), engine);
        }
    }

    #[test]
    fn a_tool_event_is_reshaped_and_keeps_everything_else() {
        let out = run(
            "PostToolUse",
            json!({"session_id": "s", "turn_id": "t", "tool_name": "write_file", "tool_use_id": "u",
                   "tool_input": {"path": "a.txt", "content": "x".repeat(100)},
                   "tool_response": {"filePath": "/x", "structuredPatch": []}, "cwd": "/w", "model": "m"}),
        );
        assert_eq!(
            out,
            json!({"session_id": "s", "turn_id": "t", "prompt_id": "t", "tool_name": "Write", "tool_use_id": "u",
                   "tool_input": {"path": "a.txt"}, "cwd": "/w", "model": "m"})
        );
    }

    #[test]
    fn bash_input_is_kept_whole_and_other_events_only_get_a_prompt_id() {
        let cmd = json!({"command": "rm -rf x", "description": "d", "workdir": "/w"});
        let out = run(
            "PreToolUse",
            json!({"tool_name": "bash", "tool_input": cmd.clone()}),
        );
        assert_eq!(out["tool_input"], cmd);
        assert_eq!(out["tool_name"], "Bash");
        let out = run(
            "Stop",
            json!({"session_id": "s", "turn_id": "t", "tool_name": "bash"}),
        );
        assert_eq!(out["prompt_id"], "t");
        assert_eq!(out["tool_name"], "bash", "only tool events are renamed");
        let out = run("SessionStart", json!({"session_id": "s"}));
        assert!(out.get("prompt_id").is_none());
        let out = run("Stop", json!({"turn_id": "t", "prompt_id": "own"}));
        assert_eq!(out["prompt_id"], "own", "an existing prompt_id wins");
    }

    #[test]
    fn a_bash_failure_becomes_exit_code_n_then_output() {
        let err = json!({"exit_code": 2, "output": "boom\nline2", "terminal_status": "failed"})
            .to_string();
        let out = run(
            "PostToolUseFailure",
            json!({"tool_name": "bash", "error": err}),
        );
        assert_eq!(out["error"], "Exit code 2\nboom\nline2");
        let sandbox =
            json!({"exit_code": 126, "output": "", "terminal_reason": "blocked by sandbox"})
                .to_string();
        let out = run(
            "PostToolUseFailure",
            json!({"tool_name": "bash", "error": sandbox}),
        );
        assert_eq!(out["error"], "Exit code 126\nblocked by sandbox");
        for plain in ["tool failed: No such file", "{\"no_code\": 1}", "[1]"] {
            let out = run(
                "PostToolUseFailure",
                json!({"tool_name": "bash", "error": plain}),
            );
            assert_eq!(out["error"], plain);
        }
        let out = run(
            "PostToolUseFailure",
            json!({"tool_name": "read_file", "error": "{\"exit_code\":1}"}),
        );
        assert_eq!(out["error"], "{\"exit_code\":1}", "only bash is reshaped");
    }
}

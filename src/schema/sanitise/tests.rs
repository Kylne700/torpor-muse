//! Tests of `sanitise`: what is fixed in the engine's output for Muse, what a person is told to type, the size cuts.

use super::*;
use serde_json::json;

fn clean(event: &str, engine: &str) -> (Option<String>, Vec<String>) {
    sanitise(event, engine.as_bytes(), "torpor", || {
        "[Torpor note #7]".into()
    })
}

#[test]
fn sanitise_fixes_what_the_engine_prints_for_muse() {
    // a Stop gate block passes through unchanged
    let (o, n) = clean("Stop", r#"{"decision":"block","reason":"prove X"}"#);
    assert_eq!(o.unwrap(), r#"{"decision":"block","reason":"prove X"}"#);
    assert!(n.is_empty());
    // an empty reason gets a default one
    let (o, _) = clean("Stop", r#"{"decision":"block","reason":""}"#);
    assert!(o.unwrap().contains("torpor status"));
    // unknown and snake_case keys are dropped, the rest survives
    let (o, n) = clean(
        "Stop",
        r#"{"systemMessage":"saved","stop_reason":"y","continue":false}"#,
    );
    assert_eq!(o.unwrap(), r#"{"systemMessage":"saved"}"#);
    assert_eq!(n.len(), 2);
    // additionalContext on Stop is dropped, not forwarded
    let (o, _) = clean(
        "Stop",
        r#"{"hookSpecificOutput":{"hookEventName":"Stop","additionalContext":"x"}}"#,
    );
    assert!(o.is_none());
    // the engine says PostToolUse for a failure: the event name is rewritten
    let (o, _) = clean(
        "PostToolUseFailure",
        r#"{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"hi"}}"#,
    );
    assert_eq!(
        o.unwrap(),
        r#"{"hookSpecificOutput":{"hookEventName":"PostToolUseFailure","additionalContext":"hi"}}"#
    );
    // state notes get the sequence header; ask becomes deny
    let (o, _) = clean(
        "UserPromptSubmit",
        r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"state"}}"#,
    );
    assert!(o.unwrap().contains("[Torpor note #7]\\nstate"));
    let (o, n) = clean(
        "PreToolUse",
        r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"sure?"}}"#,
    );
    assert!(o.unwrap().contains(r#""permissionDecision":"deny""#));
    assert_eq!(n.len(), 1);
    let ask = format!("it deletes. {ASK_TAIL}");
    let engine = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": ask}});
    let (o, _) = clean("PreToolUse", &engine.to_string());
    let o = o.unwrap();
    assert!(
        o.contains("it deletes. Muse hooks cannot ask") && !o.contains("Approve"),
        "{o}"
    );
    // not JSON, or not an object: nothing is printed
    assert!(clean("Stop", "hello").0.is_none());
    assert!(clean("Stop", "[1]").0.is_none());
    assert!(clean("Stop", "").0.is_none());
}

/// The person under Muse talks to the AI and never types `torpor`: these are the engine's own lines (engine
/// a2289f4, hook.rs 675 and 1338-1361, gate.rs 241-258 and 683-693) as the person, or the model, reads them.
#[test]
fn what_the_engine_tells_a_person_to_type_is_not_passed_on() {
    let user = |event: &str, msg: &str| {
        let engine = json!({ "systemMessage": msg });
        let (o, n) = clean(event, &engine.to_string());
        assert!(n.is_empty(), "{n:?}");
        let o: Value = serde_json::from_str(&o.unwrap()).unwrap();
        o["systemMessage"].as_str().unwrap().to_string()
    };
    // the Stop line loses the undo command, and what is not saved is for the AI to look up
    assert_eq!(
        user(
            "Stop",
            "Torpor: save point 31 · turn 5 changed 1 file (1 edit) · undo it: torpor undo 5 · saved before npm test · 2 files not saved (torpor status)"
        ),
        "Torpor: save point 31 · turn 5 changed 1 file (1 edit) · saved before npm test · 2 files not saved (ask the AI to run torpor status)"
    );
    let one = "Torpor: save point 3 · turn 1 changed 2 files (2 edits) · undo it: torpor undo 1 · 1 file not saved (torpor status)";
    assert_eq!(
        user("Stop", one),
        "Torpor: save point 3 · turn 1 changed 2 files (2 edits) · 1 file not saved (ask the AI to run torpor status)"
    );
    // a line that only looks like it is left as it is (a tail with no number)
    let odd = "Torpor: save point 3. undo it: torpor undo x";
    assert_eq!(user("Stop", odd), odd);
    // the welcome
    assert_eq!(
        user(
            "SessionStart",
            "Torpor is saving this folder at the start and end of every turn. See the turns: torpor log. Undo one: torpor undo N."
        ),
        "Torpor is saving this folder at the start and end of every turn."
    );
    // a failed check: "check", no "shadow mode" sentence, the command is something to ask the AI for
    assert_eq!(
        user(
            "Stop",
            "Torpor: promise #1 (proof page answers) failed after this turn (3 of 3 runs; it passed on the kept version (save point 3)). It would have stopped the AI, but it is in shadow mode. torpor promise show 1"
        ),
        "Torpor: check #1 (proof page answers) failed after this turn (3 of 3 runs; it passed on the kept version (save point 3)). (ask the AI: torpor promise show 1)"
    );
    assert_eq!(
        user(
            "Stop",
            "Torpor: promise #12 (a (b) failed after this turn (x) c) failed after this turn (3 of 3 runs; it passed on the version from the end of turn 4). The AI was not stopped. torpor promise show 12"
        ),
        "Torpor: check #12 (a (b) failed after this turn (x) c) failed after this turn (3 of 3 runs; it passed on the version from the end of turn 4). The AI was not stopped. (ask the AI: torpor promise show 12)"
    );
    // the other promise lines, a "new promise" line and a failure line cut short are the engine's own words
    for same in [
        "Torpor: promise #2 (api answers) passed and failed on the same version, so it is flaky and advisory only. torpor promise show 2",
        "Torpor: promise #6 (w) failed after this turn (3 of 3 runs). torpor promise show six",
        "Torpor: new promise #1 \"x\" (shadow), from your words at turn 1. Drop it: torpor promise drop 1",
    ] {
        assert_eq!(user("Stop", same), same);
    }
    // what the model reads: it runs `recover` itself
    let (o, _) = clean(
        "UserPromptSubmit",
        r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"Torpor: a restore was interrupted and some files are mixed. Ask the user to run torpor recover before editing."}}"#,
    );
    let o = o.unwrap();
    assert!(
        o.contains("Run torpor recover yourself before editing; it is safe.")
            && !o.contains("Ask the user"),
        "{o}"
    );
    // the nudge to run a check after an edit is not passed on; nothing else is lost, and nothing is noted
    let nudge = "Torpor: edit touches promises #1, #2; `torpor check 1 2` before finishing";
    for event in ["PostToolUse", "PostToolUseFailure"] {
        let engine = json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": nudge}});
        assert_eq!(clean(event, &engine.to_string()), (None, vec![]));
        let engine = json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": format!("Torpor: first\n{nudge}\nTorpor: last")}});
        let (o, n) = clean(event, &engine.to_string());
        assert!(n.is_empty());
        assert_eq!(
            o.unwrap(),
            format!(
                r#"{{"hookSpecificOutput":{{"hookEventName":"{event}","additionalContext":"Torpor: first\nTorpor: last"}}}}"#
            )
        );
    }
    // on the other events that text is not touched
    let engine = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": nudge}});
    assert!(
        clean("PreToolUse", &engine.to_string())
            .0
            .unwrap()
            .contains("edit touches promises")
    );
}

#[test]
fn sanitise_cuts_long_fields_to_fit() {
    // 9,900 two-byte characters plus escapes would be far over the limit
    let big = "\u{5d0}\n".repeat(9_900);
    let engine = json!({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": big}}).to_string();
    let (o, _) = clean("UserPromptSubmit", &engine);
    let o = o.unwrap();
    assert!(o.len() < STDOUT_LIMIT, "{}", o.len());
    validate("UserPromptSubmit", &o).unwrap();
    // most of the text survives the cut, not only the marker (a newline costs two encoded bytes)
    let v: Value = serde_json::from_str(&o).unwrap();
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(ctx.starts_with("[Torpor note #7]\n\u{5d0}") && ctx.ends_with("\n[cut]"));
    assert!(
        o.len() > STDOUT_LIMIT - 1500,
        "cut far too much: {}",
        o.len()
    );
    // a huge deny reason is cut as well, and a field nobody can cut ends in a drop, not a loop
    let deny = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "x\n".repeat(50_000)}}).to_string();
    let (o, _) = clean("PreToolUse", &deny);
    let o = o.unwrap();
    assert!(o.len() < STDOUT_LIMIT && o.contains(r#""permissionDecision":"deny""#));
    let sys = json!({"systemMessage": "y".repeat(5000)}).to_string();
    let (o, _) = clean("Stop", &sys);
    assert!(o.unwrap().len() < 1100);
}

#[test]
fn the_engine_is_named_by_its_path_for_the_model_only() {
    let at = "/opt/e/torpor";
    let run = |event: &str, engine: &str| {
        sanitise(event, engine.as_bytes(), at, || "[H]".into())
            .0
            .unwrap()
    };
    // context and a block reason are for the model; the user's systemMessage keeps the plain name
    let engine = json!({"systemMessage": "run `torpor status`", "hookSpecificOutput": {"hookEventName": "UserPromptSubmit",
            "additionalContext": "`torpor check 3` before finishing; then torpor undo."}})
        .to_string();
    let o = run("UserPromptSubmit", &engine);
    assert!(
        o.contains(r#""systemMessage":"run `torpor status`""#),
        "{o}"
    );
    assert!(
        o.contains("`/opt/e/torpor check 3` before finishing; then /opt/e/torpor undo."),
        "{o}"
    );
    let o = run(
        "Stop",
        r#"{"decision":"block","reason":"`torpor check 2` failed"}"#,
    );
    assert!(o.contains("`/opt/e/torpor check 2` failed"), "{o}");
    // the default reasons are for the model too
    assert!(run("Stop", r#"{"decision":"block"}"#).contains("Run `/opt/e/torpor status`"));
    let deny = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"`torpor undo` first"}}"#;
    assert!(run("PreToolUse", deny).contains("`/opt/e/torpor undo` first"));
}

#[test]
fn only_a_torpor_command_is_rewritten() {
    let at = "'/a b/torpor'";
    assert_eq!(
        runnable(
            "run `torpor check 3`, `torpor find-break` or torpor gate",
            at
        ),
        "run `'/a b/torpor' check 3`, `'/a b/torpor' find-break` or '/a b/torpor' gate"
    );
    for same in [
        "torpor-muse doctor",
        "/usr/bin/torpor check",
        "~/.local/bin/torpor undo",
        "my_torpor undo",
        "not-torpor status",
        "mytorpor status",
        "torpor is off",
        "Torpor status",
        "torpor  status",
        "torpor",
        "torpor ",
        "torpor statuses",
        "the torpor\nstatus",
    ] {
        assert_eq!(runnable(same, at), same);
    }
    assert_eq!(runnable("torpor check", "torpor"), "torpor check");
    assert_eq!(
        runnable("\u{e9}torpor check \u{5d0}torpor undo", at),
        "\u{e9}torpor check \u{5d0}torpor undo"
    );
}

#[test]
fn engine_text_says_muse_code() {
    let engine = json!({"systemMessage": "Start Claude Code inside a project folder", "hookSpecificOutput": {"hookEventName": "SessionStart",
            "additionalContext": "If Claude Code asked whether you trust this folder, answer yes."}})
        .to_string();
    let o = clean("SessionStart", &engine).0.unwrap();
    assert!(
        o.contains("Start Muse Code inside") && o.contains("If Muse Code asked"),
        "{o}"
    );
    assert!(!o.contains("Claude"), "{o}");
}

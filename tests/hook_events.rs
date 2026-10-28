//! Each Muse hook event goes to the engine as documented, a wrong engine never fails a turn, Muse sees a closed schema.

mod hook_common;

use hook_common::*;
use serde_json::{Map, Value, json};
use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

const TURN: &str = "00000000-0000-7000-8000-000000000002";

/// The payload as the engine should see it: the fixture plus the documented changes and nothing else (research/60
/// AD1, research/59 D11). `transcript_path` is left out on both sides: another module may fill it in.
fn expected_for_engine(event: &str, fixture: &Value) -> Value {
    let mut p: Map<String, Value> = fixture.as_object().unwrap().clone();
    p.remove("transcript_path");
    if let Some(t) = p.get("turn_id").cloned() {
        p.insert("prompt_id".into(), t);
    }
    if matches!(event, "PreToolUse" | "PostToolUse" | "PostToolUseFailure") {
        let tool = p["tool_name"].as_str().unwrap().to_owned();
        let name = match tool.as_str() {
            "bash" => "Bash",
            "edit_file" => "Edit",
            "write_file" => "Write",
            "read_file" => "Read",
            "search" => "Grep",
            other => panic!("no engine name for {other}"),
        };
        p.insert("tool_name".into(), name.into());
        p.remove("tool_response");
        if name != "Bash" {
            for k in ["content", "find", "replace"] {
                p["tool_input"].as_object_mut().unwrap().remove(k);
            }
        } else if event == "PostToolUseFailure" {
            let e: Value = serde_json::from_str(p["error"].as_str().unwrap()).unwrap();
            let text = [&e["output"], &e["terminal_reason"]]
                .into_iter()
                .filter_map(Value::as_str)
                .find(|s| !s.is_empty())
                .unwrap_or("");
            p.insert(
                "error".into(),
                format!("Exit code {}\n{text}", e["exit_code"]).into(),
            );
        }
    }
    Value::Object(p)
}

fn without_transcript(mut v: Value) -> Value {
    v.as_object_mut().unwrap().remove("transcript_path");
    v
}

fn alive(pid: &str) -> bool {
    let o = Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&o.stdout);
    !stat.trim().is_empty() && !stat.trim().starts_with('Z')
}

// ---------------------------------------------------------------- events

/// Every event `init` registers: (event, fixture, the engine is called).
const EVENTS: [(&str, &str, bool); 11] = [
    ("SessionStart", "SessionStart", true),
    ("UserPromptSubmit", "UserPromptSubmit", true),
    ("PreToolUse", "PreToolUse-bash", true),
    ("PostToolUse", "PostToolUse-bash", true),
    ("PostToolUseFailure", "PostToolUseFailure-bash", true),
    ("Stop", "Stop", true),
    ("StopFailure", "StopFailure", true),
    ("SessionEnd", "SessionEnd", true),
    ("PreCompact", "PreCompact", true),
    ("SubagentStart", "SubagentStart", false),
    ("SubagentStop", "SubagentStop", true),
];

#[test]
fn every_muse_event_is_taken_and_goes_to_the_engine_or_not_as_documented() {
    let r = Rig::new("events");
    for (event, fixture, called) in EVENTS {
        let mut p = r.fixture(fixture);
        if event == "SubagentStop" {
            // the G0 recording is an observer child's own event; a parent's has a child id of its own
            p["child_session_id"] = json!("00000000-0000-7000-8000-000000000009");
            p["turn_id"] = json!(TURN);
        }
        let run = r.hook_ok(event, &p);
        if event == "SessionStart" {
            // the one exception: the model is given the guide (F4) whatever the engine says
            assert!(context_of(&run).starts_with("[Torpor guide"), "{}", run.out);
        } else {
            assert!(
                run.out.is_empty(),
                "{event}: a quiet engine means a quiet hook: {}",
                run.out
            );
        }
        assert_eq!(r.got(event).len(), usize::from(called), "{event}");
    }
}

#[test]
fn an_event_an_earlier_version_registered_is_answered_with_nothing() {
    // Settings written before this version still name PostToolBatch and PostCompact until `init` is run again.
    let r = Rig::new("retired");
    for event in ["PostToolBatch", "PostCompact"] {
        let p = json!({"session_id": SID, "hook_event_name": event, "cwd": r.cwd, "turn_id": TURN});
        let run = r.hook_ok(event, &p);
        assert!(
            run.out.is_empty() && run.err.is_empty(),
            "{event}: {}{}",
            run.out,
            run.err
        );
        assert!(r.got(event).is_empty(), "{event}: the engine was called");
    }
    assert_eq!(r.problems(), "", "nothing is logged for them");
}

#[test]
fn the_engine_gets_each_recording_in_its_own_form_and_nothing_else_changed() {
    let r = Rig::new("normalise");
    let mut seen = 0;
    for (event, fixture, called) in EVENTS {
        // the tool events have one recording per tool; the others one
        let names: Vec<String> = match (event, fixture) {
            ("PreToolUse" | "PostToolUse", _) => {
                ["bash", "edit_file", "write_file", "read_file", "search"]
                    .iter()
                    .map(|t| format!("{event}-{t}"))
                    .collect()
            }
            ("PostToolUseFailure", _) => vec![
                "PostToolUseFailure-bash".into(),
                "PostToolUseFailure-bash-sandbox".into(),
                "PostToolUseFailure-read_file".into(),
            ],
            ("SessionStart", _) => ["", "-clear", "-compact", "-fork", "-resume"]
                .iter()
                .map(|s| format!("SessionStart{s}"))
                .collect(),
            ("Stop", _) => vec!["Stop".into(), "Stop-active".into()],
            (_, f) if called && event != "SubagentStop" => vec![f.to_owned()],
            _ => continue,
        };
        for name in names {
            let fx = r.fixture(&name);
            let before = r.got(event).len();
            r.hook_ok(event, &fx);
            let got = r.got(event);
            assert_eq!(got.len(), before + 1, "{name} did not reach the engine");
            assert_eq!(
                without_transcript(got[before].clone()),
                expected_for_engine(event, &fx),
                "{name}"
            );
            seen += 1;
        }
    }
    assert_eq!(
        seen,
        5 + 5 + 3 + 5 + 2 + 4,
        "Pre/PostToolUse 5 each, failures 3, SessionStart 5, Stop 2, then UserPromptSubmit/StopFailure/SessionEnd/PreCompact"
    );
    // the shape the engine reads for a bash failure (research/59 D11)
    let failure = r.got("PostToolUseFailure");
    assert!(
        failure[0]["error"]
            .as_str()
            .unwrap()
            .starts_with("Exit code 2\nls: cannot access")
    );
    assert!(
        failure[1]["error"]
            .as_str()
            .unwrap()
            .starts_with("Exit code 2\n/bin/sh: 1: cannot create")
    );
    assert_eq!(
        failure[2]["error"],
        r.fixture("PostToolUseFailure-read_file")["error"],
        "only bash is reshaped"
    );
}

#[test]
fn observer_children_are_ignored_after_the_heartbeat() {
    let r = Rig::new("children");
    // an observer's own events: turn id = session id (D10), or child id = session id
    let mut ups = r.fixture("UserPromptSubmit");
    ups["turn_id"] = ups["session_id"].clone();
    for (event, p) in [
        ("UserPromptSubmit", ups),
        ("SubagentStop", r.fixture("SubagentStop")),
        ("SubagentStart", r.fixture("SubagentStart")),
    ] {
        let run = r.hook_ok(event, &p);
        assert!(
            run.out.is_empty() && run.err.is_empty(),
            "{event}: {}{}",
            run.out,
            run.err
        );
        assert!(r.got(event).is_empty(), "{event} reached the engine");
    }
}

#[test]
fn unknown_keys_are_passed_on_and_logged_once() {
    let r = Rig::new("unknown-key");
    let mut p = r.fixture("UserPromptSubmit");
    p["brand_new_key"] = json!({"x": 1});
    for _ in 0..2 {
        r.hook_ok("UserPromptSubmit", &p);
    }
    let got = r.got("UserPromptSubmit");
    assert_eq!(got.len(), 2);
    assert_eq!(got[0]["brand_new_key"], json!({"x": 1}));
    assert_eq!(
        r.problems().matches("brand_new_key").count(),
        1,
        "{}",
        r.problems()
    );
}

#[test]
fn payloads_of_one_and_a_half_megabytes_are_delivered_whole() {
    let r = Rig::new("big");
    let mut p = r.fixture("UserPromptSubmit");
    p["prompt"] = json!("x".repeat(1_500_000));
    let run = r.hook_ok("UserPromptSubmit", &p);
    assert!(run.out.is_empty());
    assert_eq!(
        r.got("UserPromptSubmit")[0]["prompt"]
            .as_str()
            .unwrap()
            .len(),
        1_500_000
    );
    // a tool result this size is not forwarded: the engine reads none of it
    let mut p = r.fixture("PostToolUse-write_file");
    p["tool_response"]["structuredPatch"] = json!(["y".repeat(1_500_000)]);
    p["tool_input"]["content"] = json!("z".repeat(1_500_000));
    r.hook_ok("PostToolUse", &p);
    let sent = &r.got("PostToolUse")[0];
    assert!(sent.get("tool_response").is_none() && sent["tool_input"].get("content").is_none());
    assert!(sent.to_string().len() < 5_000);
}

#[test]
fn bad_input_never_fails_a_turn() {
    let r = Rig::new("bad-input");
    let cases: [(&str, &[u8]); 6] = [
        ("Stop", b"not json"),
        ("Stop", b"[1]"),
        ("Stop", b"{}"),
        ("Stop", b""),
        ("Stop", br#"{"session_id": 5}"#),
        ("NoSuchEvent", br#"{"session_id":"s"}"#),
    ];
    for (event, stdin) in cases {
        let run = r.hook_bytes(event, stdin);
        assert_eq!(run.code, 0, "{event} {stdin:?}");
        assert!(run.out.is_empty(), "{}", run.out);
    }
    let log = r.problems();
    for want in ["not a JSON object", "no session_id", "unknown event"] {
        assert!(log.contains(want), "{want:?} missing from {log}");
    }
    assert!(r.got("Stop").is_empty());
}

// ---------------------------------------------------------------- the engine goes wrong

#[test]
fn a_wrong_engine_fails_open_and_tells_the_user_once_per_session() {
    let r = Rig::new("wrong-engine");
    write_fake(&r.fake, "9.9.9");
    let first = r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    let v: Value = serde_json::from_str(&first.out).expect("the user is told");
    assert!(
        v["systemMessage"].as_str().unwrap().contains("NOT saving"),
        "{}",
        first.out
    );
    assert!(
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Tell the user")
    );
    // the same session is not told again, and tool events never print the notice
    assert!(
        r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"))
            .out
            .is_empty()
    );
    assert!(
        r.hook_ok("PreToolUse", &r.fixture("PreToolUse-bash"))
            .out
            .is_empty()
    );
    assert!(r.hook_ok("Stop", &r.fixture("Stop")).out.is_empty());
    // another session is
    let mut other = r.fixture("SessionStart");
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000aa");
    assert!(!r.hook_ok("SessionStart", &other).out.is_empty());
    assert!(
        r.problems().matches("9.9.9").count() >= 5,
        "every failure is logged: {}",
        r.problems()
    );
    assert!(
        r.got("SessionStart").is_empty(),
        "a wrong engine is never run"
    );
}

#[test]
fn a_missing_engine_fails_open_too() {
    let mut r = Rig::new("no-engine");
    r.engine = r.dir.join("nowhere/torpor");
    let run = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    assert!(run.out.contains("NOT saving"), "{}", run.out);
    assert!(r.problems().contains("nowhere"));
}

#[test]
fn an_engine_that_exits_with_an_error_does_not_stop_the_turn() {
    let r = Rig::new("engine-error");
    r.set("code.UserPromptSubmit", "3");
    r.set("err.UserPromptSubmit", "boom: database is locked");
    let run = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    assert!(
        run.out.contains("NOT saving"),
        "the user hears of it: {}",
        run.out
    );
    let log = r.problems();
    assert!(
        log.contains("exited 3") && log.contains("boom: database is locked"),
        "{log}"
    );
}

#[test]
fn the_engine_version_is_asked_once_not_on_every_hook() {
    let r = Rig::new("version-cache");
    for _ in 0..3 {
        r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    }
    assert_eq!(r.version_calls(), 1);
    // a different binary at the same path is asked again (size and mtime changed)
    write_fake(&r.fake, &format!("{} ", pinned()));
    for _ in 0..3 {
        r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    }
    assert_eq!(r.version_calls(), 2);
}

#[test]
fn a_stuck_engine_is_killed_with_everything_it_started_and_the_turn_goes_on() {
    let r = Rig::new("stuck");
    r.set("sleep.PreCompact", "30");
    let run = r.hook_ok("PreCompact", &r.fixture("PreCompact"));
    assert!(
        run.took >= Duration::from_millis(2_000),
        "killed before its deadline: {:?}",
        run.took
    );
    assert!(
        run.took < Duration::from_secs(10),
        "the hook hung: {:?}",
        run.took
    );
    assert!(run.out.is_empty());
    assert!(r.problems().contains("timed out"), "{}", r.problems());
    for f in ["pid.PreCompact", "child.PreCompact"] {
        let pid = fs::read_to_string(r.fake.join(f))
            .unwrap()
            .trim()
            .to_owned();
        let t0 = Instant::now();
        while alive(&pid) && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(&pid), "{f}: process {pid} is still running");
    }
}

// ---------------------------------------------------------------- what Muse is allowed to see

#[test]
fn whatever_the_engine_prints_muse_only_gets_its_closed_schema_within_the_size_limit() {
    let r = Rig::new("schema");
    let huge = "\u{e9}\n".repeat(20_000);
    let ctx = |event: &str, text: &str| json!({"hookSpecificOutput": {"hookEventName": event, "additionalContext": text}});
    let cases = [
        // snake_case and unknown keys; additionalContext and a stray hookEventName on Stop
        (
            "Stop",
            "Stop",
            json!({"decision": "block", "reason": "R", "system_message": "x", "stopReason": "y", "hookSpecificOutput": {"hookEventName": "Stop", "additionalContext": "no"}}),
        ),
        // only Stop may block; the oversized text is cut to fit
        (
            "UserPromptSubmit",
            "UserPromptSubmit",
            json!({"decision": "block", "reason": "r", "systemMessage": "s".repeat(4000), "hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": huge}}),
        ),
        // the engine's failure form says PostToolUse
        (
            "PostToolUseFailure",
            "PostToolUseFailure-bash",
            ctx("PostToolUse", "seen"),
        ),
        (
            "PreCompact",
            "PreCompact",
            ctx("PreCompact", "not allowed here"),
        ),
        // an engine question becomes a deny: Muse hooks cannot ask
        (
            "PreToolUse",
            "PreToolUse-bash",
            json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": "it deletes. Approve only if you meant it."}}),
        ),
    ];
    for (event, fixture, engine_out) in cases {
        r.set(&format!("out.{event}"), &engine_out.to_string());
        let run = r.hook_ok(event, &r.fixture(fixture));
        match event {
            "Stop" => assert_eq!(run.out, r#"{"decision":"block","reason":"R"}"#),
            "UserPromptSubmit" => {
                let v: Value = serde_json::from_str(&run.out).unwrap();
                assert!(v.get("decision").is_none(), "{event} cannot block");
                let ctx = v["hookSpecificOutput"]["additionalContext"]
                    .as_str()
                    .unwrap();
                assert!(ctx.starts_with("[Torpor note"), "{:.200}", ctx);
                assert!(
                    ctx.len() > 6000,
                    "most of the text must survive the cut: {}",
                    ctx.len()
                );
            }
            "PostToolUseFailure" => assert!(
                run.out.contains(r#""hookEventName":"PostToolUseFailure""#),
                "{}",
                run.out
            ),
            "PreCompact" => assert!(run.out.is_empty(), "{}", run.out),
            _ => {
                assert!(
                    run.out.contains(r#""permissionDecision":"deny""#),
                    "{}",
                    run.out
                );
                assert!(
                    run.out.contains("Muse hooks cannot ask") && !run.out.contains("Approve"),
                    "{}",
                    run.out
                );
            }
        }
    }
    // not JSON, or not an object: nothing is printed and the turn goes on
    for junk in ["hello", "[1]", "{\"decision\":", "   "] {
        r.set("out.Stop", junk);
        assert!(
            r.hook_ok("Stop", &r.fixture("Stop")).out.is_empty(),
            "{junk:?}"
        );
    }
    assert!(r.problems().contains("dropped"));
}

#[test]
fn torpor_blocks_the_end_of_a_turn_at_most_twice() {
    let r = Rig::new("stop-cap");
    let block = r#"{"decision":"block","reason":"prove X first"}"#;
    r.set("out.Stop", block);
    let stop = r.fixture("Stop");
    let outs: Vec<String> = (0..4).map(|_| r.hook_ok("Stop", &stop).out).collect();
    assert_eq!(
        outs,
        [block, block, "", ""],
        "Muse allows 8 blocks; the engine's cap is 2 and ours is the same"
    );
    assert!(r.problems().contains("Stop block dropped"));
    // a new turn is a new count
    let mut next = stop.clone();
    next["turn_id"] = json!("00000000-0000-7000-8000-000000000003");
    assert_eq!(r.hook_ok("Stop", &next).out, block);
}

#[test]
fn state_notes_carry_a_sequence_number_so_the_newest_replaces_the_older() {
    let r = Rig::new("sequence");
    let note = |text: &str| {
        json!({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": text}}).to_string()
    };
    for (i, text) in ["state A", "state B"].iter().enumerate() {
        r.set("out.UserPromptSubmit", &note(text));
        let run = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
        let ctx = serde_json::from_str::<Value>(&run.out).unwrap()["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            ctx.starts_with(&format!(
                "[Torpor note #{}: the newest Torpor note replaces all earlier ones]",
                i + 1
            )),
            "{ctx}"
        );
        assert!(ctx.ends_with(text));
    }
    // another session counts for itself
    let mut other = r.fixture("UserPromptSubmit");
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000bb");
    other["turn_id"] = json!("00000000-0000-7000-8000-0000000000bc");
    assert!(
        r.hook_ok("UserPromptSubmit", &other)
            .out
            .contains("[Torpor note #1:")
    );
}

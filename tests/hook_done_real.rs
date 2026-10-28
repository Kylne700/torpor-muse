//! The false Done (hook::done) against the real engine: its `status --json` and `show` are what the Stop block is built on,
//! so one session plays the four ways a claim can meet its record.

mod hook_common;

use hook_common::*;
use serde_json::{Value, json};
use std::fs;

const HEAD: &str = "[Torpor] Before you finish:";

fn at(r: &Rig, name: &str, t: u32) -> Value {
    let mut p = r.fixture(name);
    p["turn_id"] = json!(format!("00000000-0000-7000-8000-{t:012}"));
    p
}

/// A command of turn `t` that ended with `code`, then the Stop whose last message is `said`; `edit` = a file edited first.
fn turn(r: &Rig, t: u32, cmd: &str, code: i64, edit: Option<&str>, said: &str) -> Value {
    r.hook_ok("UserPromptSubmit", &at(r, "UserPromptSubmit", t));
    let mut pre = at(r, "PreToolUse-bash", t);
    pre["tool_input"]["command"] = json!(cmd);
    pre["tool_use_id"] = json!(format!("call_{t}"));
    r.hook_ok("PreToolUse", &pre);
    fs::write(r.cwd.join(format!("t{t}.txt")), "x\n").unwrap();
    if let Some(path) = edit {
        let mut p = at(r, "PostToolUse-edit_file", t);
        p["tool_input"]["path"] = json!(path);
        p["tool_response"]["filePath"] = json!(r.cwd.join(path).display().to_string());
        r.hook_ok("PostToolUse", &p);
    }
    let failed = code != 0;
    let mut post = at(
        r,
        if failed {
            "PostToolUseFailure-bash"
        } else {
            "PostToolUse-bash"
        },
        t,
    );
    post["tool_input"]["command"] = json!(cmd);
    post["tool_use_id"] = json!(format!("call_{t}"));
    let result = json!({"exit_code": code, "terminal_status": if failed { "failed" } else { "completed" }, "output": ""});
    post[if failed { "error" } else { "tool_response" }] = json!(result.to_string());
    r.hook_ok(
        if failed {
            "PostToolUseFailure"
        } else {
            "PostToolUse"
        },
        &post,
    );
    stop(r, t, said, false)
}

fn stop(r: &Rig, t: u32, said: &str, active: bool) -> Value {
    let mut p = at(r, "Stop", t);
    p["last_assistant_message"] = json!(said);
    p["stop_hook_active"] = json!(active);
    serde_json::from_str(&r.hook_ok("Stop", &p).out).unwrap_or(Value::Null)
}

fn only_ours<'a>(v: &'a Value, what: &str) -> &'a str {
    let why = blocked(v).unwrap_or_else(|| panic!("no block for {what}: {v}"));
    let rest = why
        .strip_prefix(&format!("{HEAD}\n- "))
        .unwrap_or_else(|| panic!("{what}: {why}"));
    assert!(
        rest.starts_with("You wrote that the tests pass, but ") && !rest.contains('\n'),
        "{what}: {why}"
    );
    rest
}

/// D with the real engine: no test run, the Stop after the block, a passing run, a later edit, a failing run, `torpor off`.
#[test]
fn real_engine_a_claim_meets_its_record_once_per_turn_and_only_when_the_record_says_otherwise() {
    let r = Rig::real("real-done");
    fs::write(r.cwd.join("base.txt"), "keep\n").unwrap();
    started(&r);
    r.hook_ok("Stop", &r.fixture("Stop"));

    // turn 3: no test run in the session
    let v = turn(&r, 3, "echo hi", 0, None, "All tests pass.");
    let ours = only_ours(&v, "no test run");
    assert!(
        ours.contains("no test run was seen in this session"),
        "{ours}"
    );
    // the Stop after the block, and a second look in the same turn, say nothing more
    assert!(blocked(&stop(&r, 3, "All tests pass.", true)).is_none());
    assert!(blocked(&stop(&r, 3, "All tests pass.", false)).is_none());

    // turn 4: a passing run, then the claim: nothing to say
    let v = turn(&r, 4, "npm test", 0, None, "The tests pass.");
    assert!(blocked(&v).is_none(), "{v}");

    // turn 5: code changed after the passing run
    let v = turn(&r, 5, "echo hi", 0, Some("lib.rs"), "The tests pass.");
    let ours = only_ours(&v, "edit after the run");
    assert!(
        ours.contains(
            "code was changed after the last passing test run (`npm test`, the previous turn)"
        ),
        "{ours}"
    );

    // turn 6: the last run failed
    let v = turn(&r, 6, "npm test", 1, None, "All tests pass.");
    let ours = only_ours(&v, "failing run");
    assert!(
        ours.contains("the last test run (`npm test`, this turn) exited with 1"),
        "{ours}"
    );

    // turn 7: no claim, so the Stop is as it was
    let quiet = turn(&r, 7, "echo hi", 0, None, "Done. I changed the loop.");
    assert!(!quiet.to_string().contains("You wrote that"), "{quiet}");

    // turn 8: after `torpor off` the same false claim is left alone
    assert_eq!(engine_cli(&r, &["off"]).0, 0);
    let v = turn(&r, 8, "echo hi", 0, None, "All tests pass.");
    assert!(!v.to_string().contains("You wrote that"), "{v}");
}

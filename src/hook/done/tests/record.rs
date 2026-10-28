//! What the record keeps: the order of runs and edits (a)/(b)/(c), what a hook payload says, a session from its hooks.

use crate::engine::testing::scratch;
use crate::hook::done::claim::Topic;
use crate::hook::done::commands::Seen::{No, Ran, Unsure};
use crate::hook::done::log::Why;
use crate::hook::done::payload::{code_edit, exit_code};
use crate::hook::done::{Log, record, verdict};
use crate::hook::state;
use serde_json::{Value, json};

fn started() -> Log {
    let mut l = Log::default();
    l.start("startup");
    l.see("t1");
    l
}

fn why(l: &Log, t: Topic) -> &'static str {
    match l.why(t) {
        None => "silent",
        Some(Why::NoRun) => "no run",
        Some(Why::Failed(_)) => "failed",
        Some(Why::Edited(_)) => "edited",
    }
}

#[test]
fn nothing_is_judged_by_a_session_that_did_not_start_here() {
    let mut l = Log::default();
    assert_eq!(why(&l, Topic::Test), "silent", "a log that saw only a part");
    for source in ["resume", "fork", "compact", "", "other"] {
        l.start(source);
        assert_eq!(why(&l, Topic::Test), "silent", "{source:?}");
    }
    l.start("startup");
    assert_eq!(
        (why(&l, Topic::Test), why(&l, Topic::Build)),
        ("no run", "no run")
    );
    let mut cleared = Log::default();
    cleared.start("clear");
    assert_eq!(why(&cleared, Topic::Build), "no run");
}

#[test]
fn a_failing_last_run_is_false_until_a_passing_run_follows() {
    let mut l = started();
    l.ran(Ran(1), No, "npm test");
    let Some(Why::Failed(r)) = l.why(Topic::Test) else {
        panic!("not failed")
    };
    assert_eq!((r.code, r.turn, r.cmd.as_str()), (1, 1, "npm test"));
    assert_eq!(why(&l, Topic::Build), "no run", "the build is its own kind");
    l.ran(Ran(0), No, "npm test");
    assert_eq!(why(&l, Topic::Test), "silent");
    l.ran(Ran(2), No, "npm test -- b");
    assert_eq!(
        why(&l, Topic::Test),
        "failed",
        "the last run counts, not an earlier pass"
    );
}

#[test]
fn an_edit_after_the_last_passing_run_makes_it_false_until_the_next_pass() {
    let mut l = started();
    l.ran(Ran(0), Ran(0), "make test");
    assert_eq!(
        (why(&l, Topic::Test), why(&l, Topic::Build)),
        ("silent", "silent")
    );
    l.edited();
    assert_eq!(
        (why(&l, Topic::Test), why(&l, Topic::Build)),
        ("edited", "edited")
    );
    l.ran(Ran(0), No, "make test");
    assert_eq!(
        (why(&l, Topic::Test), why(&l, Topic::Build)),
        ("silent", "edited"),
        "only the test ran again"
    );
    l.ran(No, Ran(0), "make");
    assert_eq!(why(&l, Topic::Build), "silent");
}

#[test]
fn an_edit_before_the_run_does_not_count() {
    let mut l = started();
    l.edited();
    l.ran(Ran(0), No, "pytest");
    assert_eq!(why(&l, Topic::Test), "silent");
}

#[test]
fn a_run_that_may_have_changed_the_answer_silences_the_claim_after_it() {
    let mut l = started();
    l.ran(Unsure, No, "npm test | tail");
    assert_eq!(
        why(&l, Topic::Test),
        "silent",
        "no run seen, but one may have happened"
    );
    assert_eq!(why(&l, Topic::Build), "no run");
    l.ran(Ran(1), No, "pytest");
    assert_eq!(
        why(&l, Topic::Test),
        "failed",
        "a seen run after it is the evidence again"
    );
    l.ran(Unsure, No, "npm test || true");
    assert_eq!(
        why(&l, Topic::Test),
        "silent",
        "the unsure run came after the failure"
    );
    let mut p = started();
    p.ran(Ran(0), No, "pytest");
    p.edited();
    p.ran(Unsure, No, "./run_tests.sh");
    assert_eq!(
        why(&p, Topic::Test),
        "silent",
        "the unsure run may be the re-run after the edit"
    );
    p.edited();
    assert_eq!(
        why(&p, Topic::Test),
        "edited",
        "an edit after it is known again"
    );
}

#[test]
fn turns_are_counted_from_the_ids_the_log_saw() {
    let mut l = Log::default();
    l.see("");
    assert_eq!(l.turn, 0, "an empty id is no turn");
    l.see("a");
    l.see("a");
    assert_eq!(l.turn, 1);
    l.see("b");
    l.see("");
    assert_eq!(l.turn, 2);
    l.start("startup");
    l.ran(Ran(1), No, "jest");
    l.see("c");
    assert_eq!(l.turn, 3);
    let Some(Why::Failed(r)) = l.why(Topic::Test) else {
        panic!("not failed")
    };
    assert_eq!(r.turn, 2, "the turn it ran in");
}

#[test]
fn an_old_state_file_without_the_record_loads_empty() {
    let s: crate::hook::state::Session =
        serde_json::from_str(r#"{"seq":1,"told":false,"blocks":[]}"#).unwrap();
    assert_eq!(s.done, Log::default());
    let back: Log = serde_json::from_str(&serde_json::to_string(&started()).unwrap()).unwrap();
    assert_eq!(back, started());
}

fn done(code: i64) -> String {
    json!({"exit_code": code, "terminal_status": "completed"}).to_string()
}

fn post(cmd: &str, code: i64, turn: &str) -> Value {
    json!({"hook_event_name": "PostToolUse", "tool_name": "bash", "turn_id": turn, "cwd": "/w",
        "tool_input": {"command": cmd}, "tool_response": done(code)})
}

fn failed(cmd: &str, code: i64, turn: &str) -> Value {
    let error = json!({"exit_code": code, "terminal_status": "failed",
        "terminal_reason": format!("process exited with status {code}")});
    json!({"hook_event_name": "PostToolUseFailure", "tool_name": "bash", "turn_id": turn, "cwd": "/w",
        "tool_input": {"command": cmd}, "error": error.to_string(), "is_interrupt": false})
}

fn edit(path: &str, turn: &str) -> Value {
    json!({"hook_event_name": "PostToolUse", "tool_name": "edit_file", "turn_id": turn, "cwd": "/w/app",
        "tool_input": {"path": path}, "tool_response": {"filePath": format!("/w/app/{path}")}})
}

#[test]
fn the_exit_code_is_read_only_when_muse_says_it_in_the_plain_form() {
    assert_eq!(exit_code("PostToolUse", &post("x", 0, "t")), Some(0));
    assert_eq!(exit_code("PostToolUse", &post("x", 255, "t")), Some(255));
    assert_eq!(
        exit_code("PostToolUseFailure", &failed("x", 1, "t")),
        Some(1)
    );
    assert_eq!(
        exit_code("PostToolUseFailure", &failed("x", 101, "t")),
        Some(101)
    );
    // the result as an object, not as text
    let object = json!({"tool_response": {"exit_code": 3, "terminal_status": "completed"}});
    assert_eq!(exit_code("PostToolUse", &object), Some(3));
    // not known: out of range, interrupted, the sandbox, still running, no status, text that is no JSON, nothing
    assert_eq!(exit_code("PostToolUse", &post("x", 256, "t")), None);
    assert_eq!(exit_code("PostToolUse", &post("x", -1, "t")), None);
    let mut p = failed("x", 130, "t");
    p["is_interrupt"] = json!(true);
    assert_eq!(exit_code("PostToolUseFailure", &p), None);
    let sandbox = json!({"exit_code": 2, "terminal_status": "failed",
        "terminal_reason": "The sandbox blocked a write to /bin/sh"});
    assert_eq!(
        exit_code("PostToolUseFailure", &json!({"error": sandbox.to_string()})),
        None
    );
    let running = json!({"exit_code": 0, "terminal_status": "running"});
    assert_eq!(
        exit_code(
            "PostToolUse",
            &json!({"tool_response": running.to_string()})
        ),
        None
    );
    assert_eq!(
        exit_code(
            "PostToolUse",
            &json!({"tool_response": json!({"exit_code": 0}).to_string()})
        ),
        None
    );
    assert_eq!(
        exit_code("PostToolUse", &json!({"tool_response": "done"})),
        None
    );
    assert_eq!(exit_code("PostToolUse", &json!({})), None);
    assert_eq!(
        exit_code("PostToolUseFailure", &post("x", 1, "t")),
        None,
        "a failure event has its code in `error`"
    );
}

fn path_edit(cwd: &str, path: &str) -> bool {
    code_edit(&json!({"cwd": cwd, "tool_input": {"path": path}}))
}

#[test]
fn only_an_edit_to_code_in_the_project_counts() {
    for path in [
        "src/main.rs",
        "a.py",
        "web/app.tsx",
        "Makefile",
        "package.json",
        "tests/a.test.js",
        "docs/gen.py",
    ] {
        assert!(path_edit("/w/app", path), "{path}");
        assert!(
            path_edit("/w/app", &format!("/w/app/{path}")),
            "{path} absolute"
        );
        assert!(
            path_edit("/w/app/", &format!("/w/app/{path}")),
            "{path} absolute, cwd with a slash"
        );
    }
    for path in [
        "README.md",
        "notes.TXT",
        "docs/guide.mdx",
        "CHANGELOG",
        "LICENSE",
        "NOTICE",
        ".gitignore",
        "guide.rst",
        "a.adoc",
        "../x.rs",
        "src/../../x.rs",
        "node_modules/a/index.js",
        ".git/hooks/pre-commit",
        "",
        "/tmp/x.rs",
        "/w/app2/x.rs",
        "/w/x.rs",
    ] {
        assert!(!path_edit("/w/app", path), "{path:?}");
    }
    assert!(
        !path_edit("", "/w/app/a.rs"),
        "an absolute path with no folder to compare"
    );
    assert!(path_edit("", "a.rs"));
    // the path of the result when the input has none
    assert!(code_edit(
        &json!({"cwd": "/w/app", "tool_response": {"filePath": "/w/app/src/a.rs"}})
    ));
    assert!(!code_edit(
        &json!({"cwd": "/w/app", "tool_response": {"filePath": "/w/app/notes.md"}})
    ));
    assert!(!code_edit(&json!({"cwd": "/w/app"})));
}

#[test]
fn the_stored_command_is_short_and_safe_to_show() {
    let dir = scratch("done-cmd");
    let stored = |cmd: &str| {
        record(&dir, "SessionStart", "s", &json!({"source": "startup"}));
        record(&dir, "PostToolUse", "s", &post(cmd, 1, "t1"));
        state::with_session(&dir, "s", |s| match s.done.why(Topic::Test) {
            Some(Why::Failed(r)) => r.cmd.clone(),
            _ => String::new(),
        })
        .unwrap()
    };
    let long = format!(
        "npm test -- --token sk-FAKECANARY1234 --name \"`whoami`\" {}",
        "x".repeat(200)
    );
    let cmd = stored(&long);
    assert_eq!(cmd.chars().count(), 80, "{cmd}");
    assert!(
        cmd.starts_with("npm test -- --token [redacted] --name \"'whoami'\" xxx"),
        "{cmd}"
    );
    assert_eq!(
        stored("cd app\nnpm test"),
        "cd app npm test",
        "a new line is a space"
    );
}

#[test]
fn a_session_is_recorded_and_judged_from_its_hooks() {
    let dir = scratch("done-session");
    let at = |ev: &str, p: Value| record(&dir, ev, "s1", &p);
    let ask = |turn: &str, said: &str| verdict(&dir, "s1", turn, said);
    at("SessionStart", json!({"source": "startup"}));
    at("UserPromptSubmit", json!({"turn_id": "t1"}));
    // (a) nothing ran
    let a = ask("t1", "All tests pass.").unwrap();
    assert!(a.contains("no test run was seen"), "{a}");
    assert!(
        ask("t1", "The build succeeds.")
            .unwrap()
            .contains("no build was seen")
    );
    assert_eq!(
        ask("t1", "I changed the parser."),
        None,
        "no claim, no sentence"
    );
    // (b) the last run failed
    at("PostToolUseFailure", failed("cargo test", 101, "t1"));
    let b = ask("t1", "The tests pass.").unwrap();
    assert!(
        b.contains("`cargo test`, this turn") && b.contains("exited with 101"),
        "{b}"
    );
    // the next turn
    at("UserPromptSubmit", json!({"turn_id": "t2"}));
    assert!(
        ask("t2", "The tests pass.")
            .unwrap()
            .contains("`cargo test`, the previous turn")
    );
    at("UserPromptSubmit", json!({"turn_id": "t3"}));
    assert!(
        ask("t3", "The tests pass.")
            .unwrap()
            .contains("`cargo test`, 2 turns ago")
    );
    // a pass clears it
    at("PostToolUse", post("cargo test", 0, "t3"));
    assert_eq!(ask("t3", "The tests pass."), None);
    // (c) an edit to code after the pass
    at("PostToolUse", edit("src/lib.rs", "t3"));
    let c = ask("t3", "The tests pass.").unwrap();
    assert!(
        c.contains("code was changed after the last passing test run (`cargo test`, this turn)"),
        "{c}"
    );
    // an edit to a document does not count, a pass again clears it
    at("PostToolUse", edit("README.md", "t3"));
    assert!(ask("t3", "The tests pass.").is_some());
    at("PostToolUse", post("cargo test", 0, "t3"));
    assert_eq!(ask("t3", "The tests pass."), None);
    at("PostToolUse", edit("notes.md", "t3"));
    assert_eq!(ask("t3", "The tests pass."), None);
}

#[test]
fn a_message_with_no_claim_costs_no_lock_and_no_state() {
    let dir = scratch("done-nolock");
    assert_eq!(
        verdict(
            &dir,
            "s1",
            "t1",
            "I read the parser and changed two functions."
        ),
        None
    );
    assert!(!dir.join("session").exists(), "nothing was written");
    // the same for hooks that are none of its business
    record(
        &dir,
        "PostToolUse",
        "s1",
        &json!({"tool_name": "read_file", "tool_input": {"path": "a.rs"}}),
    );
    record(&dir, "PostToolUse", "s1", &post("ls", 0, "t1"));
    record(&dir, "Stop", "s1", &json!({"turn_id": "t1"}));
    assert!(!dir.join("session").exists(), "nothing was written");
}

#[test]
fn two_sessions_are_kept_apart() {
    let dir = scratch("done-apart");
    for sid in ["a", "b"] {
        record(&dir, "SessionStart", sid, &json!({"source": "startup"}));
    }
    record(&dir, "PostToolUse", "a", &post("npm test", 0, "t1"));
    assert_eq!(verdict(&dir, "a", "t1", "The tests pass."), None);
    assert!(
        verdict(&dir, "b", "t1", "The tests pass.").is_some(),
        "b ran nothing"
    );
}

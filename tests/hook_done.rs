//! The false Done (hook::done) at the hook, against a fake engine: a Stop whose last message says the tests pass or the build
//! succeeds gets the one Stop block when this session's own record says otherwise; every other Stop is left as it was.

mod hook_common;

use hook_common::*;
use serde_json::{Value, json};

const HEAD: &str = "[Torpor] Before you finish:";
const NO_RUN: &str = "You wrote that the tests pass, but no test run was seen in this session; run the tests now and report the real result, or tell the user they were not run.";
const BROKE: &str = "Torpor: promise #1 (proof page answers) failed after this turn (3 of 3 runs; it passed on the version from the end of turn 4). It would have stopped the AI, but it is in shadow mode. torpor promise show 1";

/// The hook payload `name` as it is in turn `t` (2 is the fixture's own).
fn at(r: &Rig, name: &str, t: u32) -> Value {
    let mut p = r.fixture(name);
    p["turn_id"] = json!(format!("00000000-0000-7000-8000-{t:012}"));
    p
}

/// The Stop of turn `t` that ends a message `said`; `active` is Muse's `stop_hook_active`.
fn stop_with(r: &Rig, t: u32, said: &str, active: bool) -> Run {
    let mut p = at(r, "Stop", t);
    p["last_assistant_message"] = json!(said);
    p["stop_hook_active"] = json!(active);
    r.hook_ok("Stop", &p)
}

fn json_of(run: &Run) -> Value {
    serde_json::from_str(&run.out).unwrap_or(Value::Null)
}

/// A bash command of turn `t` that ended with `code`, as Muse's hook hears it: a failing exit is a PostToolUseFailure.
fn ran(r: &Rig, t: u32, cmd: &str, code: i64) {
    let (event, name) = match code {
        0 => ("PostToolUse", "PostToolUse-bash"),
        _ => ("PostToolUseFailure", "PostToolUseFailure-bash"),
    };
    let mut p = at(r, name, t);
    p["tool_input"]["command"] = json!(cmd);
    let result = match code {
        0 => json!({"exit_code": 0, "terminal_status": "completed", "output": "ok\n"}),
        _ => json!({"exit_code": code, "terminal_status": "failed", "output": "no\n",
            "terminal_reason": format!("process exited with status {code}")}),
    };
    p[if code == 0 { "tool_response" } else { "error" }] = json!(result.to_string());
    assert_eq!(
        r.hook_ok(event, &p).out,
        "",
        "a command is told to the model by no one"
    );
}

/// An edit of `path` (relative to the project folder, or absolute) in turn `t`.
fn edited(r: &Rig, t: u32, path: &str) {
    let mut p = at(r, "PostToolUse-edit_file", t);
    p["tool_input"]["path"] = json!(path);
    p["tool_response"]["filePath"] = json!(r.cwd.join(path).display().to_string());
    assert_eq!(r.hook_ok("PostToolUse", &p).out, "");
}

/// A session that began here (every run since is on record), its engine saving, with nothing of its own to tell.
fn session(name: &str) -> Rig {
    let r = Rig::new(name);
    started(&r);
    engine_turn(&r, 5, 30, &[], &[]);
    engine_stop(&r, &[SAVED]);
    r
}

fn one_block(reason: &str) -> String {
    format!("{HEAD}\n- {reason}")
}

#[test]
fn a_claim_that_the_tests_pass_with_no_test_run_in_the_session_gets_the_one_block() {
    let r = session("done-none");
    ran(&r, 2, "ls -la", 0); // a command that is no test run
    let first = stop_with(&r, 2, "I fixed the parser.\nAll tests pass.", false);
    let v = json_of(&first);
    assert_eq!(blocked(&v), Some(one_block(NO_RUN).as_str()), "{v}");
    assert!(
        said(&v).starts_with("Torpor: save point 31"),
        "the user's line stays: {v}"
    );
    assert_eq!(
        r.calls(),
        ["status --json", "show 5 --json"],
        "the two calls of every Stop, none of ours"
    );
    // Muse runs the blocked turn on and stops again in the same turn: that Stop is not read and says nothing more
    let again = json_of(&stop_with(&r, 2, "The tests pass.", true));
    assert!(blocked(&again).is_none(), "{again}");
    let again = json_of(&stop_with(&r, 2, "The tests pass.", false));
    assert!(blocked(&again).is_none(), "not twice in a turn: {again}");
    // the next turn that says it again is told again
    let next = json_of(&stop_with(&r, 3, "Done, the tests pass.", false));
    assert_eq!(blocked(&next), Some(one_block(NO_RUN).as_str()), "{next}");
}

#[test]
fn a_build_claim_names_the_build_and_a_run_of_the_other_kind_does_not_help() {
    let r = session("done-build");
    ran(&r, 2, "npm test", 0);
    let v = json_of(&stop_with(&r, 2, "Everything compiles cleanly.", false));
    let want = "You wrote that the build succeeds, but no build was seen in this session; run the build now and report the real result, or tell the user it was not run.";
    assert_eq!(blocked(&v), Some(one_block(want).as_str()), "{v}");
    ran(&r, 3, "npm run build", 0);
    let ok = stop_with(&r, 3, "The build succeeds.", false);
    assert!(blocked(&json_of(&ok)).is_none(), "{}", ok.out);
}

#[test]
fn a_claim_after_a_passing_run_and_no_later_edit_leaves_the_output_as_it_was() {
    let r = session("done-pass");
    edited(&r, 2, "src/app.js");
    ran(&r, 2, "cd app && npm test", 0);
    let plain = stop_with(&r, 2, "Finished the work.", false);
    assert!(
        blocked(&json_of(&plain)).is_none() && !plain.out.is_empty(),
        "{}",
        plain.out
    );
    for (t, said) in [
        (3, "All tests pass."),
        (4, "Tests: the unit tests pass, too."),
    ] {
        assert_eq!(stop_with(&r, t, said, false).out, plain.out, "{said}");
    }
    // a change to the docs, or to a file outside the project, after the run is no change to the code
    edited(&r, 5, "README.md");
    edited(&r, 5, "/tmp/claude-elsewhere/notes.py");
    assert_eq!(stop_with(&r, 5, "All tests pass.", false).out, plain.out);
    // a command that is neither: nothing either
    ran(&r, 6, "git status", 0);
    assert_eq!(stop_with(&r, 6, "All tests pass.", false).out, plain.out);
}

#[test]
fn a_claim_after_a_failing_last_run_gets_the_block_until_a_run_passes() {
    let r = session("done-fail");
    ran(&r, 2, "cd app && npm test", 1);
    let v = json_of(&stop_with(&r, 2, "The tests pass now.", false));
    let want = "You wrote that the tests pass, but the last test run (`cd app && npm test`, this turn) exited with 1; run the tests now and report the real result, or tell the user they were not run.";
    assert_eq!(blocked(&v), Some(one_block(want).as_str()), "{v}");
    // a run of another kind that passes does not make it true; a failing run later is the last run
    ran(&r, 3, "npm run build", 0);
    ran(&r, 3, "pytest -q", 2);
    let v = json_of(&stop_with(&r, 3, "The tests pass.", false));
    assert!(
        blocked(&v).is_some_and(|w| w.contains("(`pytest -q`, this turn) exited with 2")),
        "{v}"
    );
    // a passing run after the failing one is the last run
    ran(&r, 4, "pytest -q", 0);
    let v = stop_with(&r, 4, "The tests pass.", false);
    assert!(blocked(&json_of(&v)).is_none(), "{}", v.out);
}

#[test]
fn a_claim_after_an_edit_of_the_code_since_the_last_passing_run_gets_the_block() {
    let r = session("done-edit");
    ran(&r, 2, "pytest -q", 0);
    edited(&r, 3, "app/main.py");
    let v = json_of(&stop_with(&r, 3, "All tests pass.", false));
    let want = "You wrote that the tests pass, but code was changed after the last passing test run (`pytest -q`, the previous turn); run the tests again and report the real result, or tell the user they were not re-run.";
    assert_eq!(blocked(&v), Some(one_block(want).as_str()), "{v}");
    // run again: true again
    ran(&r, 4, "pytest -q", 0);
    let v = stop_with(&r, 4, "All tests pass.", false);
    assert!(blocked(&json_of(&v)).is_none(), "{}", v.out);
    // a new file counts as an edit, too
    let mut p = at(&r, "PostToolUse-write_file", 5);
    p["tool_input"]["path"] = json!("tests/new_test.py");
    p["tool_response"]["filePath"] = json!(r.cwd.join("tests/new_test.py").display().to_string());
    r.hook_ok("PostToolUse", &p);
    let v = json_of(&stop_with(&r, 5, "All tests pass.", false));
    assert!(
        blocked(&v).is_some_and(|w| w.contains("(`pytest -q`, the previous turn)")),
        "{v}"
    );
    // the run is named by the turns the session did something in
    edited(&r, 6, "app/main.py");
    let v = json_of(&stop_with(&r, 6, "All tests pass.", false));
    assert!(
        blocked(&v).is_some_and(|w| w.contains("(`pytest -q`, 2 turns ago)")),
        "{v}"
    );
}

#[test]
fn the_stop_after_our_block_and_a_stop_the_engine_blocked_are_left_alone() {
    let r = session("done-left");
    // the Stop that follows a block: unread, whatever it says and whatever the record
    let v = stop_with(&r, 2, "All tests pass.", true);
    assert!(
        blocked(&json_of(&v)).is_none() && v.out.contains("save point 31"),
        "{}",
        v.out
    );
    // the engine blocked this Stop itself: its block goes out as it is, no call of ours, nothing added
    let theirs = r#"{"decision":"block","reason":"prove X first"}"#;
    r.set("out.Stop", theirs);
    let before = r.calls().len();
    let v = json_of(&stop_with(&r, 3, "All tests pass.", false));
    assert_eq!(blocked(&v), Some("prove X first"), "{v}");
    assert_eq!(r.calls().len(), before, "{:?}", r.calls());
    // the engine lets the Stop that follows its block end: that one is not read either
    engine_stop(&r, &[SAVED]);
    assert!(blocked(&json_of(&stop_with(&r, 3, "All tests pass.", true))).is_none());
    // and the next turn is: the claim is read then
    let v = json_of(&stop_with(&r, 4, "All tests pass.", false));
    assert_eq!(blocked(&v), Some(one_block(NO_RUN).as_str()), "{v}");
}

#[test]
fn a_broken_saved_check_and_a_false_done_in_the_same_stop_are_one_block_with_both() {
    let r = session("done-both");
    engine_stop(&r, &[SAVED, BROKE]);
    let e = r.engine.display().to_string();
    let v = json_of(&stop_with(&r, 2, "I fixed it. All tests pass.", false));
    let why = blocked(&v).unwrap_or_else(|| panic!("no block: {v}"));
    assert_eq!(why.matches(HEAD).count(), 1, "one header: {why}");
    assert!(why.starts_with(HEAD), "{why}");
    for text in [
        "Promise #1 \"proof page answers\" (turn 5 broke it) failed after this turn.",
        &format!("Run `{e} check 1` before you say it is fixed."),
    ] {
        assert!(why.contains(text), "{text}: {why}");
    }
    assert!(
        why.ends_with(&format!("\n- {NO_RUN}")),
        "ours comes last: {why}"
    );
    assert!(blocked(&json_of(&stop_with(&r, 2, "All tests pass.", true))).is_none());
    // a file deleted in the turn: the same one block, with the third thing
    engine_turn(&r, 6, 40, &[], &[("gone.txt", "edit", "deleted")]);
    engine_stop(&r, &[SAVED]);
    let v = json_of(&stop_with(&r, 3, "All tests pass.", false));
    let why = blocked(&v).unwrap_or_else(|| panic!("no block: {v}"));
    assert!(
        why.contains("gone.txt") && why.ends_with(&format!("\n- {NO_RUN}")),
        "{why}"
    );
    assert_eq!(why.matches(HEAD).count(), 1, "{why}");
}

/// What a Stop that claims nothing prints is what the same Stop printed before this check was added.
#[test]
fn a_stop_with_no_claim_prints_what_it_printed_before_whatever_the_record_says() {
    let r = session("done-silent");
    engine_stop(&r, &[SAVED, BROKE]);
    let quiet = stop_with(&r, 2, "Fixed the layout bug.", false);
    assert!(
        blocked(&json_of(&quiet)).is_some_and(|w| !w.contains("You wrote")),
        "{}",
        quiet.out
    );
    ran(&r, 3, "npm test", 1);
    edited(&r, 3, "src/app.js");
    ran(&r, 3, "npm run build", 2);
    for (t, said) in [
        (3, "Fixed the layout bug."),
        (
            4,
            "I could not run the tests, so I cannot say that they pass.",
        ),
        (5, "Do the tests pass? Run `npm test` to find out."),
        (6, "```\nAll tests pass\n```\nThat is the output I expect."),
        (7, "The build should succeed once the key is set."),
        (8, ""),
    ] {
        assert_eq!(stop_with(&r, t, said, false).out, quiet.out, "{said:?}");
    }
    assert_eq!(r.calls().len(), 2 * 7, "the two calls of every Stop");
}

#[test]
fn nothing_is_said_when_the_engine_is_off_or_cannot_tell() {
    let r = session("done-off");
    ran(&r, 2, "npm test", 1);
    // `torpor off`, or a folder that is not being saved: the engine's status says it is not saving
    for status in [
        json!({"saving": false, "off": true, "problems": [], "last_turn": {"n": 5}}),
        json!({"saving": false, "problems": [], "last_turn": {"n": 5}}),
    ] {
        r.set("out.status", &status.to_string());
        let v = stop_with(&r, 2, "All tests pass.", false);
        assert!(blocked(&json_of(&v)).is_none(), "{status}: {}", v.out);
    }
    // no answer from the engine: the claim is not read against a record that cannot be trusted to count
    std::fs::remove_file(r.fake.join("out.status")).unwrap();
    let v = stop_with(&r, 3, "All tests pass.", false);
    assert!(blocked(&json_of(&v)).is_none(), "{}", v.out);
    // saving again: it is told
    engine_turn(&r, 5, 30, &[], &[]);
    let v = json_of(&stop_with(&r, 4, "All tests pass.", false));
    assert!(
        blocked(&v).is_some_and(|w| w.contains("exited with 1")),
        "{v}"
    );
}

#[test]
fn a_session_that_was_resumed_has_no_run_it_can_say_is_missing_but_a_failing_one_is_seen() {
    let r = Rig::new("done-resume");
    r.hook_ok("SessionStart", &r.fixture("SessionStart-resume"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    engine_turn(&r, 5, 30, &[], &[]);
    engine_stop(&r, &[SAVED]);
    // the earlier part of the session is not on record: the tests may have been run there
    let v = stop_with(&r, 2, "All tests pass.", false);
    assert!(blocked(&json_of(&v)).is_none(), "{}", v.out);
    ran(&r, 3, "npm test", 1);
    let v = json_of(&stop_with(&r, 3, "All tests pass.", false));
    assert!(
        blocked(&v).is_some_and(|w| w.contains("exited with 1")),
        "{v}"
    );
}

#[test]
fn a_subagents_runs_and_edits_are_the_sessions_own() {
    let r = session("done-child");
    let child = "00000000-0000-7000-8000-0000000000c1";
    let as_child = |mut p: Value| {
        p["session_id"] = json!(child);
        p["turn_id"] = json!(child);
        p
    };
    // the subagent ran the tests and they passed: the parent that reports it is right
    let mut run = as_child(r.fixture("PostToolUse-bash"));
    run["tool_input"]["command"] = json!("npm test");
    r.hook_ok("PostToolUse", &run);
    let v = stop_with(&r, 2, "All tests pass.", false);
    assert!(blocked(&json_of(&v)).is_none(), "{}", v.out);
    // the next turn: the subagent changed the code, so the tests were not run since
    r.hook_ok("UserPromptSubmit", &at(&r, "UserPromptSubmit", 3));
    let mut edit = as_child(r.fixture("PostToolUse-edit_file"));
    edit["tool_input"]["path"] = json!("src/app.js");
    edit["tool_response"]["filePath"] = json!(r.cwd.join("src/app.js").display().to_string());
    r.hook_ok("PostToolUse", &edit);
    let v = json_of(&stop_with(&r, 3, "All tests pass.", false));
    assert!(
        blocked(&v).is_some_and(|w| w.contains("code was changed after")),
        "{v}"
    );
}

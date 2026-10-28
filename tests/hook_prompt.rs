//! The check-writer's notes at the prompt: what it sends, off, failed, stopped or not taken up, each told once in plain words.

mod hook_common;

use hook_common::*;
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};

// ---------------------------------------------------------------- the check-writer's note, and text made safe

#[test]
fn the_first_prompt_in_a_folder_says_once_that_the_check_writer_is_off_and_how_to_turn_it_on() {
    let r = Rig::unseeded("writer-note");
    let first = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    let (said, ctx) = note_parts(&first.out);
    assert!(
        said.contains("check-writer is off") && said.contains("key_file"),
        "{said}"
    );
    assert!(
        said.contains("chmod 600") && said.contains("torpor-muse doctor"),
        "{said}"
    );
    // the notice is for the user: the model's context gets nothing (an eval arm without a key sees what the others see)
    assert!(ctx.is_empty(), "{ctx}");
    // once for this folder: its next prompt hears nothing more, and neither does a new session in the same folder
    let again = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    assert!(again.out.is_empty(), "{}", again.out);
    let mut other = r.fixture("UserPromptSubmit");
    other["session_id"] = json!(OTHER);
    let new_session = r.hook_ok("UserPromptSubmit", &other);
    assert!(new_session.out.is_empty(), "{}", new_session.out);
    // another folder hears it again, once
    let elsewhere = r.dir.join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000dd");
    other["cwd"] = json!(elsewhere);
    let (said, _) = note_parts(&r.hook_ok("UserPromptSubmit", &other).out);
    assert!(said.contains("check-writer is off"), "{said}");
    assert!(r.hook_ok("UserPromptSubmit", &other).out.is_empty());
    assert!(r.problems().is_empty(), "{}", r.problems());
}

#[test]
fn the_off_note_is_told_once_per_session_where_the_folder_is_unknown_or_cannot_be_marked() {
    let heard_twice = |r: &Rig, p: &mut Value| {
        let (said, _) = note_parts(&r.hook_ok("UserPromptSubmit", p).out);
        assert!(said.contains("check-writer is off"), "{said}");
        assert!(
            r.hook_ok("UserPromptSubmit", p).out.is_empty(),
            "same session"
        );
        p["session_id"] = json!(OTHER);
        let (said, _) = note_parts(&r.hook_ok("UserPromptSubmit", p).out);
        assert!(said.contains("check-writer is off"), "new session: {said}");
    };
    // no folder in the payload: nothing to mark, so nothing is shared between sessions
    let r = Rig::unseeded("writer-note-no-cwd");
    let mut p = r.fixture("UserPromptSubmit");
    p["cwd"] = json!("");
    heard_twice(&r, &mut p);
    // a mark that cannot be written (a file is in the way of its folder): the session still hears it once only
    let r = Rig::unseeded("writer-note-no-mark");
    fs::create_dir_all(r.store().join("muse")).unwrap();
    fs::write(r.store().join("muse/no-key"), "").unwrap();
    heard_twice(&r, &mut r.fixture("UserPromptSubmit"));
}

#[test]
fn the_off_note_is_told_once_per_folder_and_reason() {
    let r = Rig::unseeded("writer-note-reasons");
    let sid = |n: u32| format!("00000000-0000-7000-8000-0000000000{n:02x}");
    // no key_file line: the first reason, told once for the folder (not again in a new session)
    let said = told_at_prompt(&r, &sid(1));
    assert!(
        said.contains("there is no `key_file = <path>` line"),
        "{said}"
    );
    assert_eq!(told_at_prompt(&r, &sid(1)), "", "same session");
    assert_eq!(told_at_prompt(&r, &sid(2)), "", "new session, same reason");
    // another reason in the same folder: told once more, once
    key_setup(&r, "key_file = ~/key.txt\n", 0o644);
    let said = told_at_prompt(&r, &sid(3));
    assert!(said.contains("only you can read"), "{said}");
    assert_eq!(told_at_prompt(&r, &sid(4)), "");
    // and a third one
    key_setup(&r, "keyfile = ~/key.txt\n", 0o600);
    let said = told_at_prompt(&r, &sid(5));
    assert!(said.contains("unknown or empty setting"), "{said}");
    assert_eq!(told_at_prompt(&r, &sid(6)), "");
    // back to the first reason: this folder was told that one already
    fs::remove_file(r.dir.join("home/.config/torpor-muse/config")).unwrap();
    assert_eq!(told_at_prompt(&r, &sid(7)), "", "a reason told before");
    assert!(r.problems().is_empty(), "{}", r.problems());
}

/// An earlier version marked a folder for `no-key` by the folder alone. Such a mark does not stand for any reason now,
/// so the note is told once more (not told silently for a reason it was never written for).
#[test]
fn a_mark_from_before_the_reason_was_part_of_it_is_told_once_more() {
    let r = Rig::unseeded("writer-note-old-mark");
    // the `app` mark has the old shape (a file named by the folder alone, holding the folder), and the hook writes it
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    leave_result(&r, SID, "advisory", "app", "no start script");
    let said = told_at_prompt(&r, SID);
    assert!(said.contains("needs an app"), "{said}");
    let marks = r.store().join("muse");
    let old = fs::read_dir(marks.join("app"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::create_dir_all(marks.join("no-key")).unwrap();
    fs::copy(&old, marks.join("no-key").join(old.file_name().unwrap())).unwrap();
    // the key goes: the off note is told despite that mark, once
    fs::remove_file(r.dir.join("home/.config/torpor-muse/config")).unwrap();
    let said = told_at_prompt(&r, OTHER);
    assert!(said.contains("check-writer is off"), "{said}");
    assert_eq!(told_at_prompt(&r, SID), "");
    assert!(r.problems().is_empty(), "{}", r.problems());
}

#[test]
fn the_note_is_added_to_what_the_engine_printed() {
    let r = Rig::unseeded("writer-note-merged");
    r.set(
        "out.UserPromptSubmit",
        r#"{"systemMessage":"saved","hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"engine says hello"}}"#,
    );
    let run = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    let (said, ctx) = note_parts(&run.out);
    assert!(
        said.starts_with("saved — ") && said.contains("check-writer is off"),
        "{said}"
    );
    assert!(
        ctx.ends_with("\nengine says hello") && !ctx.contains("check-writer"),
        "{ctx}"
    );
}

/// A config in the rig's HOME; the key file it names holds a fake key and is never sent anywhere (the prompt is too
/// short for a run to start).
fn key_setup(r: &Rig, config: &str, key_mode: u32) {
    let home = r.dir.join("home");
    fs::create_dir_all(home.join(".config/torpor-muse")).unwrap();
    let cfg = home.join(".config/torpor-muse/config");
    fs::write(&cfg, config).unwrap();
    fs::set_permissions(&cfg, fs::Permissions::from_mode(0o600)).unwrap();
    let key = home.join("key.txt");
    fs::write(&key, "LLM|0000000000000000|fake-key-for-tests\n").unwrap();
    fs::set_permissions(&key, fs::Permissions::from_mode(key_mode)).unwrap();
}

fn short_prompt(r: &Rig) -> Value {
    let mut p = r.fixture("UserPromptSubmit");
    p["prompt"] = json!("ok");
    p
}

#[test]
fn no_note_when_a_private_key_file_is_configured_and_the_reason_is_named_when_it_is_not_usable() {
    let r = Rig::unseeded("writer-note-keyfile");
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    let run = r.hook_ok("UserPromptSubmit", &short_prompt(&r));
    assert!(
        run.out.is_empty(),
        "a usable key file needs no note: {}",
        run.out
    );

    let r = Rig::unseeded("writer-note-open-file");
    key_setup(&r, "key_file = ~/key.txt\n", 0o644);
    let (said, _) = note_parts(&r.hook_ok("UserPromptSubmit", &short_prompt(&r)).out);
    assert!(said.contains("only you can read"), "{said}");

    let r = Rig::unseeded("writer-note-typo");
    key_setup(&r, "keyfile = ~/key.txt\n", 0o600);
    let (said, _) = note_parts(&r.hook_ok("UserPromptSubmit", &short_prompt(&r)).out);
    assert!(said.contains("unknown or empty setting"), "{said}");
}

const OTHER: &str = "00000000-0000-7000-8000-0000000000aa";

/// What a finished run of session `sid` left in `result.json` (the row `suggest::finish` writes).
fn leave_result(r: &Rig, sid: &str, outcome: &str, code: &str, message: &str) {
    let row = json!({"v": 1, "at": "2026-01-01T00:00:00Z", "outcome": outcome, "saved": outcome == "proven",
        "promise": null, "message": message, "code": code, "session": sid, "model": "muse-spark-1.3",
        "calls": 1, "usd": 0.0, "ms": 5});
    fs::write(writer_ws(r).join("result.json"), row.to_string()).unwrap();
}

/// A prompt of session `sid` too short for a run to start; what the user is told in answer ("" = nothing).
fn told_at_prompt(r: &Rig, sid: &str) -> String {
    let mut p = short_prompt(r);
    p["session_id"] = json!(sid);
    let run = r.hook_ok("UserPromptSubmit", &p);
    if run.out.is_empty() {
        return String::new();
    }
    let (said, ctx) = note_parts(&run.out);
    assert!(ctx.is_empty(), "the model is told nothing: {ctx}");
    said
}

#[test]
fn a_run_that_failed_is_told_to_the_user_once_per_kind_and_session_at_the_next_prompt() {
    let r = Rig::new("writer-news");
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    let cases = [
        (
            "key",
            "advisory",
            "Meta refused the API key (HTTP 401). Check the key file.",
        ),
        (
            "cap",
            "advisory",
            "the daily spend cap would be passed: $1.99 spent today (UTC), the next call could cost up to $0.05, cap $2.00. Nothing was sent. Raise the cap with TORPOR_MUSE_DAY_CAP_USD or day_cap_usd in the config.",
        ),
        ("api", "advisory", "the Meta API did not answer in time"),
        (
            "",
            "error",
            "cannot read the key file: Permission denied (os error 13)",
        ),
    ];
    for (code, outcome, message) in cases {
        leave_result(&r, SID, outcome, code, message);
        let said = told_at_prompt(&r, SID);
        let why = message.trim_end_matches('.');
        assert!(
            said.starts_with(
                "Torpor's check-writer wrote no check from an earlier prompt of yours: "
            ) && said.ends_with(&format!("{why}. Save points and undo are not affected.")),
            "{code}: {said}"
        );
        // the same kind again, in this session: nothing more
        leave_result(&r, SID, outcome, code, message);
        assert_eq!(told_at_prompt(&r, SID), "", "{code}");
        // another session hears it again, once
        leave_result(&r, OTHER, outcome, code, message);
        assert!(told_at_prompt(&r, OTHER).contains(why), "{code}");
        assert_eq!(told_at_prompt(&r, OTHER), "", "{code}");
    }
    // the last run was another session's: this one is told nothing about it
    leave_result(
        &r,
        OTHER,
        "advisory",
        "key",
        "Meta refused the API key (HTTP 403). Check the key file.",
    );
    assert_eq!(
        told_at_prompt(&r, "00000000-0000-7000-8000-0000000000cc"),
        ""
    );
    assert!(r.problems().is_empty(), "{}", r.problems());
}

#[test]
fn a_run_the_watchdog_stopped_is_told_once_per_session_in_one_fixed_sentence() {
    let r = Rig::new("writer-slow");
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    let message = "the run took too long and was stopped";
    leave_result(&r, SID, "advisory", "slow", message);
    assert_eq!(
        told_at_prompt(&r, SID),
        "Torpor's check-writer took too long on an earlier prompt of yours and was stopped before it finished. Save points and undo are not affected."
    );
    leave_result(&r, SID, "advisory", "slow", message);
    assert_eq!(told_at_prompt(&r, SID), "");
    assert!(r.problems().is_empty(), "{}", r.problems());
}

#[test]
fn an_answer_the_check_writer_could_not_use_is_told_once_per_session_without_the_raw_error() {
    let r = Rig::new("writer-rejected");
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    let message = "the model ran out of room before it finished its answer";
    leave_result(&r, SID, "rejected", "rejected", message);
    assert_eq!(
        told_at_prompt(&r, SID),
        "Torpor's check-writer could not use the model's answer to an earlier prompt of yours, so it wrote no check. Save points and undo are not affected."
    );
    leave_result(&r, SID, "rejected", "rejected", message);
    assert_eq!(told_at_prompt(&r, SID), "");
    assert!(r.problems().is_empty(), "{}", r.problems());
}

/// A run of session `SID` that ended with `code` is told in `said` at the next prompt, once; the same end again is not
/// told, and another session of the folder hears it once.
fn told_once(name: &str, code: &str, message: &str, said: &str) {
    let r = Rig::new(name);
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    leave_result(&r, SID, "advisory", code, message);
    assert_eq!(told_at_prompt(&r, SID), said);
    leave_result(&r, SID, "advisory", code, message);
    assert_eq!(told_at_prompt(&r, SID), "");
    leave_result(&r, OTHER, "advisory", code, message);
    assert_eq!(told_at_prompt(&r, OTHER), said);
    assert_eq!(told_at_prompt(&r, OTHER), "");
    assert!(r.problems().is_empty(), "{}", r.problems());
}

#[test]
fn a_check_that_fails_on_the_code_as_it_is_is_told_once_per_session_in_one_fixed_sentence() {
    told_once(
        "writer-fails",
        "fails",
        "the engine turned the check down: it fails on the version from the start of turn 2 in 5 of 5 runs (POST /api/login answered 200, expected 201)",
        "The check Torpor's check-writer wrote for an earlier claim of yours fails on the code as it is now, which can mean that what you claimed does not work yet or that the check is wrong, so no check was saved. Save points and undo are not affected.",
    );
}

#[test]
fn an_engine_that_did_not_finish_is_told_once_per_session_in_one_fixed_sentence() {
    told_once(
        "writer-engine",
        "engine",
        "the Torpor engine ended with exit Some(2): boom",
        "Torpor's check-writer wrote a check for an earlier claim of yours, but the Torpor engine did not finish saving it (it timed out or ended with an error), so no check was saved. Save points and undo are not affected.",
    );
}

/// Each source of the `nosave` code (see `suggest/work.rs`) is told in one sentence about the claim, built from the run's
/// words: what the check-writer did, then why nothing was saved, each part once. A key-shaped string in the words is
/// masked.
#[test]
fn a_claim_whose_check_was_not_saved_is_told_once_per_session_in_one_plain_sentence() {
    let cases = [
        (
            "records",
            "no check saved: step 1 (PUT /api/notes/mvqm4ta2) left out: it needs a record already in your data, which the check did not create",
            "Torpor's check-writer wrote a check for an earlier claim of yours, but every step needed a record that already existed in your data and was not created by the check, so no check was saved. Save points and undo are not affected.",
        ),
        (
            "lint",
            "the engine turned the check down: Rejected: step 1 expects a long number sk-FAKECANARY1234",
            "Torpor's check-writer wrote a check for an earlier claim of yours, but the Torpor engine did not save it: step 1 expects a long number [redacted]. Save points and undo are not affected.",
        ),
        (
            "lint-more",
            "the engine turned the check down: Rejected: steps is empty. Rejected: and 2 more problems.",
            "Torpor's check-writer wrote a check for an earlier claim of yours, but the Torpor engine did not save it: steps is empty, and 2 more problems. Save points and undo are not affected.",
        ),
        // a lint line has a second sentence of advice for the model: the user reads the first
        (
            "lint-advice",
            "the engine turned the check down: Rejected: the check has no assertion (an expect_* step, or http with expect). A check that cannot fail proves nothing.",
            "Torpor's check-writer wrote a check for an earlier claim of yours, but the Torpor engine did not save it: the check has no assertion (an expect_* step, or http with expect). Save points and undo are not affected.",
        ),
        // one mistake in two steps is said once, with the number of the others
        (
            "lint-twice",
            "the engine turned the check down: Rejected: step 1 uses an absolute URL (http://x.test/a). Paths must start with /. Rejected: step 2 uses an absolute URL (http://x.test/b). Paths must start with /.",
            "Torpor's check-writer wrote a check for an earlier claim of yours, but the Torpor engine did not save it: step 1 uses an absolute URL (http://x.test/a), and 1 more problem. Save points and undo are not affected.",
        ),
        (
            "lint-twice-more",
            "the engine turned the check down: Rejected: step 1 uses an absolute URL (http://x.test/a). Paths must start with /. Rejected: step 2 uses an absolute URL (http://x.test/b). Paths must start with /. Rejected: and 3 more problems.",
            "Torpor's check-writer wrote a check for an earlier claim of yours, but the Torpor engine did not save it: step 1 uses an absolute URL (http://x.test/a), and 4 more problems. Save points and undo are not affected.",
        ),
        (
            "cannot-run",
            "the engine turned the check down: the check could not run on the version from the start of turn 2: connection refused",
            "Torpor's check-writer wrote a check for an earlier claim of yours, but the Torpor engine did not save it: the check could not run on the version from the start of turn 2: connection refused. Save points and undo are not affected.",
        ),
        (
            "refused",
            "Torpor did not save the check: there is no version to call good.",
            "Torpor's check-writer wrote a check for an earlier claim of yours, but the Torpor engine did not save it: there is no version to call good. Save points and undo are not affected.",
        ),
        (
            "refused-no-reason",
            "Torpor did not save the check:",
            "Torpor's check-writer wrote a check for an earlier claim of yours, but the Torpor engine did not save it. Save points and undo are not affected.",
        ),
        // the end of the run's loop; no path reaches it, so this row is written by hand
        (
            "none-made",
            "no check was made",
            "Torpor's check-writer made no check for an earlier claim of yours. Save points and undo are not affected.",
        ),
        // words from no known source are quoted, not guessed at
        (
            "other",
            "something else ended the run.",
            "Torpor's check-writer saved no check for an earlier claim of yours: something else ended the run. Save points and undo are not affected.",
        ),
    ];
    for (name, message, said) in cases {
        told_once(&format!("writer-nosave-{name}"), "nosave", message, said);
    }
}

#[test]
fn what_is_normal_for_the_check_writer_is_not_told() {
    let r = Rig::new("writer-normal");
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    let normal = [
        ("proven", "Promise #1 saved. Proof: passed 5 of 5."),
        (
            "advisory",
            "nothing to check: not a statement that something works (it is a question)",
        ),
        (
            "advisory",
            "cannot check this with HTTP requests (it needs a browser)",
        ),
        ("skipped", "Torpor is not saving in this folder"),
    ];
    for (outcome, message) in normal {
        leave_result(&r, SID, outcome, "", message);
        assert_eq!(told_at_prompt(&r, SID), "", "{outcome}: {message}");
    }
    // no result at all, and a result file that is not a result
    fs::remove_file(writer_ws(&r).join("result.json")).unwrap();
    assert_eq!(told_at_prompt(&r, SID), "");
    fs::write(writer_ws(&r).join("result.json"), "{not json").unwrap();
    assert_eq!(told_at_prompt(&r, SID), "");
}

#[test]
fn a_folder_that_is_not_an_app_with_a_start_script_is_told_once_per_folder() {
    let r = Rig::new("writer-app");
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    let message = "cannot make a check: package.json has no start, dev, serve or server script to start the app with";
    leave_result(&r, SID, "advisory", "app", message);
    let said = told_at_prompt(&r, SID);
    assert!(
        said.contains("needs an app it can start with a script") && said.contains("`start`"),
        "{said}"
    );
    assert!(said.ends_with("this folder has none, so it will write no checks here. Save points and undo still work."), "{said}");
    // once for the folder: this session and any other stay quiet, whatever the run says
    leave_result(&r, SID, "advisory", "app", message);
    assert_eq!(told_at_prompt(&r, SID), "");
    leave_result(&r, OTHER, "advisory", "app", message);
    assert_eq!(told_at_prompt(&r, OTHER), "");
    assert!(r.problems().is_empty(), "{}", r.problems());
}

/// A run that never started leaves no result: the hook itself notes why, and tells at the next prompt.
#[test]
fn a_prompt_the_check_writer_could_not_take_up_is_told_at_the_next_prompt_once() {
    let r = Rig::new("writer-busy");
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    // another run holds the folder: this test holds its lock. The API base is a closed loopback port, so that even a
    // run that did start could reach nothing.
    let held = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(writer_ws(&r).join("lock"))
        .unwrap();
    held.try_lock().unwrap();
    let env = [("TORPOR_MUSE_API_BASE", "http://127.0.0.1:9")];
    let prompt = r.fixture("UserPromptSubmit").to_string();
    let hear = || r.hook_env("UserPromptSubmit", prompt.as_bytes(), &env).out;
    assert_eq!(
        hear(),
        "",
        "the prompt that found the writer busy hears nothing yet"
    );
    let out = hear();
    let (said, ctx) = note_parts(&out);
    assert!(
        said.contains("still busy with an earlier prompt, so it did not look at your last one. Save points and undo are not affected."),
        "{said}"
    );
    assert!(ctx.is_empty(), "{ctx}");
    assert_eq!(hear(), "", "once");

    // a run that cannot get its state folder: told the same way, as another kind
    let r = Rig::new("writer-start");
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    fs::create_dir_all(r.store().join("muse")).unwrap();
    fs::write(
        r.store().join("muse/suggest"),
        "a file where the folder should be",
    )
    .unwrap();
    assert_eq!(
        r.hook_env("UserPromptSubmit", prompt.as_bytes(), &env).out,
        ""
    );
    let said = note_parts(&r.hook_env("UserPromptSubmit", prompt.as_bytes(), &env).out).0;
    assert!(
        said.starts_with(
            "Torpor could not start its check-writer on your last prompt. Save points"
        ),
        "{said}"
    );
}

// ---------------------------------------------------------------- what the check-writer sends, told once per folder

/// The notice, word for word. Its facts are those of the README, "What is sent to Meta".
const SEND_NOTICE: &str = "Torpor's check-writer sends each prompt you type, cleaned and redacted, to Meta to ask whether it says something works, and sends file names and code from this project only when the answer is yes or when no usable answer came back. `torpor off` in this folder stops this, and stops Torpor's saving here too.";

/// The API base of these prompts: a closed loopback port, so that even a run that did start could reach nothing.
const CLOSED_API: [(&str, &str); 1] = [("TORPOR_MUSE_API_BASE", "http://127.0.0.1:9")];

const SAVING_HERE: &str = r#"{"off":false,"saving":true,"last_turn":{"n":1}}"#;

/// The rig's folder as one a run could work in: a script to start the app with, and an engine that says Torpor is
/// saving here.
fn app_here(r: &Rig) {
    let scripts = r#"{"scripts":{"start":"node server.js"}}"#;
    fs::write(r.cwd.join("package.json"), scripts).unwrap();
    r.set("out.status", SAVING_HERE);
}

/// Another run holds the folder (this test holds its lock until the file is dropped), so no prompt there starts one.
fn hold_lock(r: &Rig) -> fs::File {
    let held = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(writer_ws(r).join("lock"))
        .unwrap();
    held.try_lock().unwrap();
    held
}

/// A prompt of session `sid` in the rig's folder (`text`, or the fixture's long words); the hook must exit 0 with output
/// Muse accepts.
fn prompt_at(r: &Rig, sid: &str, text: Option<&str>) -> Run {
    let mut p = r.fixture("UserPromptSubmit");
    p["session_id"] = json!(sid);
    if let Some(t) = text {
        p["prompt"] = json!(t);
    }
    let run = r.hook_env("UserPromptSubmit", p.to_string().as_bytes(), &CLOSED_API);
    assert_eq!(run.code, 0, "{}", run.err);
    assert_muse_output("UserPromptSubmit", &run.out);
    run
}

/// What the user is told at such a prompt ("" = nothing); the model is told nothing.
fn said_at(r: &Rig, sid: &str, text: Option<&str>) -> String {
    let run = prompt_at(r, sid, text);
    if run.out.is_empty() {
        return String::new();
    }
    let (said, ctx) = note_parts(&run.out);
    assert!(ctx.is_empty(), "the model is told nothing: {ctx}");
    said
}

/// A session of its own; never the fixture's turn id (a payload whose session id is its turn id is a subagent's).
fn sid(n: u32) -> String {
    format!("00000000-0000-7000-8000-00000000a{n:03x}")
}

#[test]
fn the_first_prompt_the_check_writer_will_send_says_once_per_folder_what_goes_to_meta_and_how_to_stop_it()
 {
    let r = Rig::new("writer-send");
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    app_here(&r);
    let _held = hold_lock(&r);
    assert_eq!(said_at(&r, &sid(1), None), SEND_NOTICE);
    // the same session: the news is that the folder was busy with the last prompt, and the notice is not told twice
    let again = said_at(&r, &sid(1), None);
    assert!(
        again.contains("still busy with an earlier prompt") && !again.contains("sends each prompt"),
        "{again}"
    );
    // a later session of the same folder hears nothing
    assert_eq!(said_at(&r, &sid(2), None), "");
    // another folder is told, once
    let elsewhere = Rig {
        dir: r.dir.clone(),
        cwd: r.dir.join("elsewhere"),
        fake: r.fake.clone(),
        engine: r.engine.clone(),
        quiet: true,
    };
    fs::create_dir_all(&elsewhere.cwd).unwrap();
    app_here(&elsewhere);
    let _held_there = hold_lock(&elsewhere);
    assert_eq!(said_at(&elsewhere, &sid(3), None), SEND_NOTICE);
    assert_eq!(said_at(&elsewhere, &sid(4), None), "");
    assert!(r.problems().is_empty(), "{}", r.problems());
}

#[test]
fn the_notice_waits_until_the_check_writer_would_really_send_a_prompt() {
    let r = Rig::new("writer-send-waits");
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    app_here(&r);
    let _held = hold_lock(&r);
    let marked = || r.store().join("muse/send").exists();
    // each of these prompts sends nothing: no notice, and the folder is not marked as told
    let quiet = |n: u32, why: &str, text: Option<&str>| {
        assert_eq!(said_at(&r, &sid(n), text), "", "{why}");
        assert!(!marked(), "{why}: marked as told");
    };
    quiet(1, "nothing to check in the prompt", Some("ok"));
    // the engine says Torpor is off or not saving here, or does not say
    r.set(
        "out.status",
        r#"{"off":true,"saving":false,"last_turn":{"n":1}}"#,
    );
    quiet(2, "Torpor is off", None);
    r.set(
        "out.status",
        r#"{"off":false,"saving":false,"last_turn":{"n":1}}"#,
    );
    quiet(3, "Torpor is not saving", None);
    fs::remove_file(r.fake.join("out.status")).unwrap();
    quiet(4, "the engine gives no status", None);
    r.set("out.status", SAVING_HERE);
    key_setup(&r, "key_file = ~/key.txt\nday_cap_usd = 0\n", 0o600);
    quiet(5, "a day cap of 0 sends nothing", None);
    fs::remove_file(r.dir.join("home/.config/torpor-muse/config")).unwrap();
    quiet(6, "no key", None);
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    fs::remove_file(r.cwd.join("package.json")).unwrap();
    quiet(7, "no script to start the app with", None);
    // a folder a prompt is sent from: told now, once
    app_here(&r);
    assert_eq!(said_at(&r, &sid(8), None), SEND_NOTICE);
    assert!(marked());
    assert_eq!(said_at(&r, &sid(9), None), "");
    assert!(r.problems().is_empty(), "{}", r.problems());
}

#[test]
fn with_no_key_the_only_note_is_that_the_check_writer_is_off() {
    let r = Rig::unseeded("writer-send-no-key");
    app_here(&r);
    let _held = hold_lock(&r);
    let said = said_at(&r, &sid(1), None);
    assert!(
        said.contains("check-writer is off") && !said.contains("sends each prompt"),
        "{said}"
    );
    assert_eq!(said_at(&r, &sid(2), None), "", "told for the folder");
    assert!(!r.store().join("muse/send").exists());
}

#[test]
fn the_notice_is_for_the_user_the_text_for_the_model_is_the_same_with_and_without_it() {
    let r = Rig::new("writer-send-model");
    key_setup(&r, "key_file = ~/key.txt\n", 0o600);
    app_here(&r);
    let _held = hold_lock(&r);
    r.set(
        "out.UserPromptSubmit",
        r#"{"systemMessage":"saved","hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"engine says hello"}}"#,
    );
    let with = prompt_at(&r, &sid(1), None);
    let without = prompt_at(&r, &sid(2), None); // the folder has been told
    let (said, ctx) = note_parts(&with.out);
    assert!(
        said.starts_with("saved") && said.ends_with(SEND_NOTICE),
        "{said}"
    );
    assert_eq!(note_parts(&without.out).0, "saved");
    assert!(
        ctx.ends_with("engine says hello"),
        "the engine's text is there: {ctx}"
    );
    assert_eq!(context_of(&with), context_of(&without));
    // and nothing but the user's line differs
    let model_part = |run: &Run| {
        let mut v: Value = serde_json::from_str(&run.out).unwrap();
        v.as_object_mut().unwrap().remove("systemMessage");
        v
    };
    assert_eq!(model_part(&with), model_part(&without));
}

#[test]
fn what_the_engine_wrote_to_stderr_reaches_the_user_and_the_model_without_control_characters_or_keys()
 {
    let r = Rig::new("stderr-scrub");
    r.set("code.UserPromptSubmit", "1");
    r.set(
        "err.UserPromptSubmit",
        "boom \x1b[31mred\x1b[0m sk-FAKECANARY1234 done",
    );
    let run = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    let (said, ctx) = note_parts(&run.out);
    for text in [&said, &ctx] {
        assert!(
            text.contains("boom") && text.contains("[redacted]"),
            "{text}"
        );
        assert!(!text.contains("FAKECANARY"), "{text}");
        assert!(!text.chars().any(char::is_control), "{text:?}");
    }
    assert!(!run.out.contains("\\u001b"), "{}", run.out);
}

#[test]
fn the_engines_text_says_muse_code() {
    let r = Rig::new("muse-code");
    r.set(
        "out.SessionStart",
        r#"{"systemMessage":"Start Claude Code inside a project folder","hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"If Claude Code asked whether you trust this folder, say yes."}}"#,
    );
    let run = r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    let (said, ctx) = note_parts(&run.out);
    assert_eq!(said, "Start Muse Code inside a project folder");
    assert!(ctx.contains("If Muse Code asked whether"), "{ctx}");
    assert!(!run.out.contains("Claude Code"), "{}", run.out);
}

#[test]
fn a_command_in_the_text_for_the_model_runs_in_a_shell_that_has_no_engine_on_its_path() {
    let r = Rig::new("runnable-command");
    r.set(
        "out.UserPromptSubmit",
        r#"{"systemMessage":"see torpor log","hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"Before you finish, run `torpor version`; torpor is fast."}}"#,
    );
    let run = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    let (said, ctx) = note_parts(&run.out);
    assert_eq!(said, "see torpor log", "the user's line is left alone");
    assert!(ctx.contains("torpor is fast"), "other words stay: {ctx}");
    let cmd = ctx
        .split('`')
        .nth(1)
        .expect("the command stays in backticks");
    assert!(cmd.ends_with(" version") && cmd.starts_with('/'), "{cmd}");
    // the model's shell: no TORPOR_BIN, and a PATH without the engine's folder
    let out = Command::new("sh")
        .args(["-c", cmd])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(out.status.success(), "{cmd}: {:?}", out);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("torpor {}", pinned())
    );
}

#[test]
fn a_hook_without_a_usable_home_does_nothing_and_says_so() {
    let r = Rig::new("no-home");
    let run_with = |home: Option<&str>| {
        let mut c = Command::new(EXE);
        c.args(["hook", "Stop"])
            .current_dir(&r.cwd)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("TORPOR_BIN", &r.engine);
        if let Some(h) = home {
            c.env("HOME", h);
        }
        let mut child = c
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let _ = child
            .stdin
            .take()
            .unwrap()
            .write_all(r.fixture("Stop").to_string().as_bytes());
        child.wait_with_output().unwrap()
    };
    for home in [None, Some(""), Some("relative/home")] {
        let o = run_with(home);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(0), "{home:?}: {err}");
        assert!(o.stdout.is_empty(), "{home:?}");
        assert!(
            err.contains("HOME is not a full path") && err.lines().count() == 1,
            "{home:?}: {err}"
        );
        assert!(
            !r.cwd.join(".torpor").exists() && r.cwd.read_dir().unwrap().next().is_none(),
            "{home:?}: the project folder must stay empty"
        );
    }
    assert!(r.got("Stop").is_empty(), "the engine is not run either");
}

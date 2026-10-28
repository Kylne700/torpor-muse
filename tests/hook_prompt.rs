//! The check-writer's notes at the prompt: off, failed, stopped or not taken up, each told once in plain words.

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

#[test]
fn any_other_end_without_a_saved_check_is_told_once_per_session_in_the_runs_own_words_made_safe() {
    // a key-shaped string in the words is masked
    told_once(
        "writer-nosave",
        "nosave",
        "the engine turned the check down: Rejected: step 1 expects a long number sk-FAKECANARY1234",
        "Torpor's check-writer wrote no check from an earlier prompt of yours: the engine turned the check down: Rejected: step 1 expects a long number [redacted]. Save points and undo are not affected.",
    );
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

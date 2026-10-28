//! `torpor-muse doctor` against made-up problems.log lines, as the hook core writes them.
// Each test crate compiles these src files on its own: what only the binary or another test crate calls looks unused.
#[allow(dead_code)]
#[path = "../src/doctor.rs"]
mod doctor;
#[allow(dead_code)]
#[path = "../src/liveness.rs"]
mod liveness;
#[allow(dead_code)]
#[path = "../src/transcript.rs"]
mod transcript;
#[allow(dead_code)]
#[path = "../src/util.rs"]
mod util;

#[path = "shim_common.rs"]
mod common;
mod doctor_common;

use common::LogBuilder;
use doctor::{Level, Report};
use doctor_common::{World, every_problem_says_how, healthy, hook, install, must, mustnt, suggest};
use std::fs;
use std::path::PathBuf;

/// Writes `<vault>/muse/problems.log` as core does: `<unix seconds> <Event> <session, 8 chars> <message>`.
/// Each line is (seconds before doctor's "now", event, message).
fn write_problems(w: &World, lines: &[(u64, &str, &str)]) -> PathBuf {
    let now_s = (w.t0() + 60_000) / 1000;
    let f = liveness::shim_dir(&w.vault).join("problems.log");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    let text: String = lines
        .iter()
        .map(|(ago, event, msg)| format!("{} {event} 01a10000 {msg}\n", now_s - ago))
        .collect();
    fs::write(&f, text).unwrap();
    f
}

fn warns(r: &Report) -> usize {
    r.0.iter().filter(|(l, _)| *l == Level::Warn).count()
}

#[test]
fn a_problem_line_saying_the_engine_did_not_run_is_a_fix_and_says_how() {
    // the messages are the ones the hook core writes (src/engine.rs, src/hook.rs, src/hook/handle.rs)
    let msgs = [
        (
            "UserPromptSubmit",
            "torpor not found: set TORPOR_BIN or put it on PATH",
        ),
        (
            "UserPromptSubmit",
            "torpor engine /x/torpor: No such file or directory (os error 2)",
        ),
        (
            "SessionStart",
            "torpor engine /x/torpor prints \"torpor 9.9.9\", engine.lock pins \"torpor 2.3.1\"",
        ),
        (
            "Stop",
            "/x/torpor hook Stop timed out after 31500 ms and was killed",
        ),
        (
            "PreToolUse",
            "torpor hook PreToolUse exited 3: boom: database is locked",
        ),
        (
            "Stop",
            "cannot run /x/torpor: Permission denied (os error 13)",
        ),
        ("Stop", "engine.lock has no version line"),
        ("Stop", "cannot read stdin: broken pipe"),
        ("Stop", "stdin is not a JSON object (12 bytes)"),
        ("Stop", "payload has no session_id"),
        ("Stop", "unknown event name on the command line"),
    ];
    for (event, msg) in msgs {
        let w = healthy("problems-fix");
        let f = write_problems(&w, &[(300, event, msg)]);
        let before = fs::read(&f).unwrap();
        let r = w.examine();
        must(
            &r,
            Level::Fix,
            "1 hook call(s) went through without a check",
        );
        must(
            &r,
            Level::Fix,
            &format!("the last 5m ago on {event}: {msg}"),
        );
        must(&r, Level::Fix, "do: fix what that line says");
        must(&r, Level::Fix, "torpor-muse init");
        must(
            &r,
            Level::Fix,
            &format!(": > {}", util::quote(&f.to_string_lossy())),
        );
        assert_eq!(r.fixes(), 1, "{msg}: {}", r.text());
        every_problem_says_how(&r);
        assert_eq!(fs::read(&f).unwrap(), before, "doctor leaves the log alone");
    }
}

#[test]
fn a_cut_output_or_a_payload_that_changed_shape_is_a_warning() {
    let msgs = [
        "engine stdout is not a JSON object: dropped",
        "engine output key foo dropped",
        "decision block dropped on SessionStart",
        "additionalContext dropped: not allowed on PostToolUse",
        "block without a reason: default reason used",
        "a field was cut to keep stdout under the Muse limit",
        "payload does not fit our schema: missing field `session_id`",
        "hook_event_name is \"Notification\" but the command says Stop",
        "a message a later version of core may write",
        // only a message that begins with a "the engine did not run" phrase is a fix
        "engine output key cannot run dropped",
    ];
    for msg in msgs {
        let w = healthy("problems-warn");
        write_problems(&w, &[(30, "Stop", msg)]);
        let r = w.examine();
        must(&r, Level::Warn, "1 hook note(s) in the last 24 h");
        must(&r, Level::Warn, &format!("the last 30s ago on Stop: {msg}"));
        assert!(!r.text().contains("no problem logged"), "{}", r.text());
        assert_eq!(r.fixes(), 0, "{msg}: {}", r.text());
        every_problem_says_how(&r);
    }
}

#[test]
fn routine_lines_and_a_missing_or_empty_log_are_no_problem() {
    let w = healthy("problems-none");
    let r = w.examine();
    assert!(!r.text().contains("problems.log"), "{}", r.text());
    assert_eq!((r.fixes(), warns(&r)), (0, 0));
    write_problems(&w, &[]);
    let r = w.examine();
    must(
        &r,
        Level::Ok,
        "no problem logged by the hooks in the last 24 h (0 routine and 0 older",
    );
    write_problems(
        &w,
        &[
            (
                10,
                "Stop",
                "Stop block dropped: 2 blocks already given in this turn",
            ),
            (
                20,
                "Stop",
                "payload key brand_new_key is not in our schema (kept, passed on)",
            ),
            (
                30,
                "SessionStart",
                "gate default not set, tried again at the next session start: `torpor gate off` exited 3: refused",
            ),
        ],
    );
    let r = w.examine();
    must(
        &r,
        Level::Ok,
        "no problem logged by the hooks in the last 24 h (3 routine and 0 older",
    );
    assert_eq!((r.fixes(), warns(&r)), (0, 0), "{}", r.text());
}

#[test]
fn a_problem_older_than_a_day_is_history() {
    let down = "torpor not found: set TORPOR_BIN or put it on PATH";
    let w = healthy("problems-old");
    // exactly 24 h old still counts; a second older does not
    write_problems(&w, &[(86_400, "Stop", down)]);
    must(&w.examine(), Level::Fix, "the last 24h ago on Stop");
    write_problems(&w, &[(86_401, "Stop", down), (90_000, "Stop", "odd")]);
    let r = w.examine();
    assert_eq!((r.fixes(), warns(&r)), (0, 0), "{}", r.text());
    must(&r, Level::Ok, "(0 routine and 2 older line(s) in");
}

#[test]
fn the_newest_line_is_shown_and_each_kind_is_counted() {
    let w = healthy("problems-newest");
    write_problems(
        &w,
        &[
            (4000, "Stop", "payload has no session_id"),
            (130, "UserPromptSubmit", "cannot read stdin: late"),
            (2000, "SessionStart", "cannot read stdin: middle"),
            (50, "Stop", "engine output key foo dropped"),
            (60, "Stop", "engine output key bar dropped"),
        ],
    );
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "3 hook call(s) went through without a check in the last 24 h (the engine did not run, or the hook's input was unusable), the last 2m ago on UserPromptSubmit: cannot read stdin: late",
    );
    must(
        &r,
        Level::Warn,
        "2 hook note(s) in the last 24 h, the last 50s ago on Stop: engine output key foo dropped",
    );
}

#[test]
fn bad_lines_are_skipped_and_only_the_end_of_a_big_log_is_read() {
    let w = healthy("problems-junk");
    let f = write_problems(&w, &[(40, "Stop", "payload has no session_id")]);
    let good = fs::read(&f).unwrap();
    let mut bytes = b"garbage\n\n   \nabc Stop s1 not a time\n99999999999999999999999 Stop s1 too big\n1700000050 Stop\n".to_vec();
    bytes.extend_from_slice(b"\xff\xfe\n");
    bytes.extend_from_slice(&good);
    fs::write(&f, &bytes).unwrap();
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "1 hook call(s) went through without a check",
    );
    assert_eq!(warns(&r), 0, "{}", r.text());
    // a fresh line that lies more than 128 KiB before the end is not read: only the end counts
    let now_s = (w.t0() + 60_000) / 1000;
    let mut big = format!("{} Stop 01a10000 payload has no session_id\n", now_s - 20).into_bytes();
    let old = format!("{} Stop 01a10000 {}\n", now_s - 200_000, "x".repeat(200));
    for _ in 0..1000 {
        big.extend_from_slice(old.as_bytes());
    }
    big.extend_from_slice(&good);
    assert!(big.len() > 200 * 1024);
    fs::write(&f, &big).unwrap();
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "1 hook call(s) went through without a check",
    );
    // the line the limit cuts in two is dropped, even when its second half looks like a line of its own
    let fake = format!("{} Stop 01a10000 cannot read stdin: cut\n", now_s - 10);
    let routine = |pad: usize| {
        format!(
            "{} Stop 01a10000 Stop block dropped: {}\n",
            now_s - 5,
            "y".repeat(pad)
        )
    };
    let cap = doctor::problems::PROBLEMS_READ as usize;
    let rest = cap - fake.len() - routine(0).len();
    let start = format!("{} Stop 01a10000 start of a long line ", now_s - 300);
    let mut cut = start.as_bytes().to_vec();
    cut.extend_from_slice(fake.as_bytes());
    cut.extend_from_slice(routine(rest).as_bytes());
    assert_eq!(
        cut.len(),
        start.len() + cap,
        "the limit cuts exactly where `fake` starts"
    );
    fs::write(&f, &cut).unwrap();
    let r = w.examine();
    assert_eq!((r.fixes(), warns(&r)), (0, 0), "{}", r.text());
    must(&r, Level::Ok, "(1 routine and 0 older line(s) in");
}

#[test]
fn only_the_log_of_the_vault_the_hooks_write_to_is_read() {
    let w = healthy("problems-vault");
    let other = w.t.join("other-vault");
    let f = liveness::shim_dir(&other).join("problems.log");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    let now_s = (w.t0() + 60_000) / 1000;
    fs::write(
        &f,
        format!("{} Stop 01a10000 payload has no session_id\n", now_s - 5),
    )
    .unwrap();
    let r = w.examine();
    assert_eq!((r.fixes(), warns(&r)), (0, 0), "{}", r.text());
}

#[test]
fn a_stop_that_went_through_without_the_engine_is_not_a_live_gate() {
    // healthy(): the Stop heartbeat is 2 s after the settings were written, doctor's "now" is 60 s after
    let down = "torpor not found: set TORPOR_BIN or put it on PATH";
    let w = healthy("problems-gate");
    must(&w.examine(), Level::Ok, "gate live: Stop ran");
    // in the same second as the Stop heartbeat or later, whatever the event: a call went through without the engine
    for (ago, event) in [(58, "Stop"), (30, "UserPromptSubmit"), (0, "SessionStart")] {
        write_problems(&w, &[(ago, event, down)]);
        let r = w.examine();
        must(
            &r,
            Level::Fix,
            "gate not live: a hook call at or after the last Stop (58s ago, session s-main) went through without the engine",
        );
        mustnt(&r, Level::Ok, "gate live");
        must(
            &r,
            Level::Fix,
            "1 hook call(s) went through without a check",
        );
        every_problem_says_how(&r);
    }
    // a failure from before that Stop: the engine is back, the gate is live (the failure is still reported)
    write_problems(&w, &[(59, "Stop", down)]);
    let r = w.examine();
    must(&r, Level::Ok, "gate live: Stop ran");
    must(
        &r,
        Level::Fix,
        "1 hook call(s) went through without a check",
    );
    // a cut output is a note, not a gate that is down
    write_problems(&w, &[(10, "Stop", "engine output key foo dropped")]);
    must(&w.examine(), Level::Ok, "gate live: Stop ran");
}

#[test]
fn a_log_with_many_lines_and_no_prompt_is_a_fix() {
    let w = World::new("drift");
    let sid = "01a00000-0000-7000-8000-000000000004";
    let text: String = (0..250).map(|_| common::filler_line(10)).collect();
    common::put_log(&w.sessions(), sid, &text);
    w.beat(sid, true, "UserPromptSubmit", 1000);
    w.beat(sid, true, "Stop", 2000);
    let paths = transcript::Paths {
        vault: w.vault.clone(),
        sessions: w.sessions(),
    };
    transcript::translate(&paths, sid).unwrap();
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "hold no prompt: its log format may have changed",
    );
    every_problem_says_how(&r);
}

#[test]
fn lines_of_muses_log_that_could_not_be_read_are_a_warning() {
    let w = World::new("badlines");
    let sid = "s-bad";
    let mut b = LogBuilder::new(sid);
    b.accepted("chat", "main");
    let mut raw = b.text().into_bytes();
    raw.extend_from_slice(b"{\"payload_type\":\"runtime.user_intent.accepted\" broken\n");
    common::put_log(&w.sessions(), sid, &String::from_utf8(raw).unwrap());
    w.beat(sid, true, "UserPromptSubmit", 1000);
    w.beat(sid, true, "Stop", 2000);
    let paths = transcript::Paths {
        vault: w.vault.clone(),
        sessions: w.sessions(),
    };
    transcript::translate(&paths, sid).unwrap();
    let r = w.examine();
    must(
        &r,
        Level::Warn,
        "1 line(s) of Muse's log for session s-bad had an unexpected shape",
    );
    mustnt(&r, Level::Fix, "hold no prompt");
}

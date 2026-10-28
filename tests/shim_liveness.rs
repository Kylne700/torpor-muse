//! Heartbeats and the turn counter (src/liveness.rs).
// Each test crate compiles these src files on its own: what only the binary or another test crate calls looks unused.
#[allow(dead_code)]
#[path = "../src/liveness.rs"]
mod liveness;
#[allow(dead_code)]
#[path = "../src/util.rs"]
mod util;

#[path = "shim_common.rs"]
mod common;

use common::Tmp;
use liveness::{
    Beat, beat_in, gate_live, is_main, last_by_event, read_beats, safe_id, sessions, turns,
    unpaired,
};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn seq(names: &[&str]) -> Vec<Beat> {
    names
        .iter()
        .enumerate()
        .map(|(i, e)| Beat {
            t: i as u64,
            e: e.to_string(),
            m: true,
        })
        .collect()
}

fn mode(p: &Path) -> u32 {
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

#[test]
fn a_beat_is_one_small_line_per_event() {
    let t = Tmp::new("beat");
    beat_in(&t.0, "Stop", "s1", true, 1234).unwrap();
    beat_in(&t.0, "UserPromptSubmit", "s1", false, 1300).unwrap();
    let raw = std::fs::read_to_string(t.join("muse/beats/s1.jsonl")).unwrap();
    assert_eq!(
        raw,
        "{\"t\":1234,\"e\":\"Stop\",\"m\":true}\n{\"t\":1300,\"e\":\"UserPromptSubmit\",\"m\":false}\n"
    );
    let b = read_beats(&t.0, "s1");
    assert_eq!((b[0].t, b[0].e.as_str(), b[0].m), (1234, "Stop", true));
    assert_eq!(
        (b[1].t, b[1].e.as_str(), b[1].m),
        (1300, "UserPromptSubmit", false)
    );
    assert!(read_beats(&t.0, "nothing-here").is_empty());
}

#[test]
fn the_files_are_private() {
    let t = Tmp::new("modes");
    beat_in(&t.0, "Stop", "s1", true, 1).unwrap();
    assert_eq!(mode(&t.join("muse")), 0o700);
    assert_eq!(mode(&t.join("muse/beats")), 0o700);
    assert_eq!(mode(&t.join("muse/beats/s1.jsonl")), 0o600);
}

#[test]
fn a_session_id_that_is_not_a_plain_name_writes_nothing() {
    let t = Tmp::new("unsafe");
    for bad in ["../x", "", "a/b", "a b", ".", "..", "a.b", &"x".repeat(81)] {
        assert!(!safe_id(bad), "{bad:?}");
        assert!(beat_in(&t.0, "Stop", bad, true, 1).is_err(), "{bad:?}");
    }
    assert!(!t.join("muse").exists());
    for good in [
        "01a179b0-ac39-7f73-87ac-e586fdf21c04",
        "a_b-C9",
        &"x".repeat(80),
    ] {
        assert!(safe_id(good), "{good:?}");
    }
}

#[test]
fn an_event_name_cannot_break_the_line() {
    let t = Tmp::new("event");
    beat_in(&t.0, "St\"op\n}{", "s1", true, 5).unwrap();
    beat_in(&t.0, &"E".repeat(100), "s1", true, 6).unwrap();
    let b = read_beats(&t.0, "s1");
    assert_eq!(b.len(), 2);
    assert_eq!(b[0].e, "Stop");
    assert_eq!(b[1].e.len(), 40);
}

#[test]
fn parallel_hooks_do_not_tear_each_others_lines() {
    let t = Tmp::new("parallel");
    std::thread::scope(|s| {
        for n in 0..8 {
            let t = &t;
            s.spawn(move || {
                for i in 0..200 {
                    beat_in(&t.0, "PreToolUse", "s1", true, n * 1000 + i).unwrap();
                }
            });
        }
    });
    let raw = std::fs::read_to_string(t.join("muse/beats/s1.jsonl")).unwrap();
    assert_eq!(raw.lines().count(), 1600);
    assert_eq!(read_beats(&t.0, "s1").len(), 1600);
}

#[test]
fn a_busy_session_keeps_its_newest_beats_in_two_files() {
    let t = Tmp::new("rotate");
    for i in 0..8000 {
        beat_in(&t.0, "PostToolUse", "s1", true, i).unwrap();
    }
    assert!(
        t.join("muse/beats/s1.jsonl.1").is_file(),
        "rotated at 256 KiB"
    );
    let all = read_beats(&t.0, "s1");
    assert_eq!(all.len(), 8000);
    assert!(
        all.windows(2).all(|w| w[0].t < w[1].t),
        "the older half is read first"
    );
    assert!(
        std::fs::metadata(t.join("muse/beats/s1.jsonl"))
            .unwrap()
            .len()
            < 256 * 1024
    );
    assert_eq!(
        sessions(&t.0),
        ["s1"],
        "the rotated half is not a session of its own"
    );
}

#[test]
fn observer_sessions_are_counted_apart() {
    let t = Tmp::new("observers");
    for (e, n) in [
        ("SessionStart", 1),
        ("UserPromptSubmit", 2),
        ("Stop", 2),
        ("PreToolUse", 3),
    ] {
        for _ in 0..n {
            beat_in(&t.0, e, "main-1", true, 100).unwrap();
        }
    }
    for (e, n) in [
        ("SubagentStart", 1),
        ("PreToolUse", 5),
        ("PostToolUse", 5),
        ("SubagentStop", 1),
    ] {
        for _ in 0..n {
            beat_in(&t.0, e, "child-1", false, 200).unwrap();
        }
    }
    let (main, obs) = last_by_event(&t.0, 10);
    let names = |m: &liveness::Last| m.keys().cloned().collect::<Vec<_>>();
    assert_eq!(
        names(&main),
        ["PreToolUse", "SessionStart", "Stop", "UserPromptSubmit"]
    );
    assert_eq!(
        names(&obs),
        ["PostToolUse", "PreToolUse", "SubagentStart", "SubagentStop"]
    );
    assert_eq!(main["PreToolUse"], (100, "main-1".to_string(), 3));
    assert_eq!(obs["PreToolUse"], (200, "child-1".to_string(), 5));
    assert!(is_main(&read_beats(&t.0, "main-1")));
    assert!(!is_main(&read_beats(&t.0, "child-1")));
    assert!(!is_main(&[]));
}

#[test]
fn a_finished_turn_proves_the_gate_and_an_open_one_does_not() {
    let done = turns(&seq(&[
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "Stop",
    ]));
    assert_eq!(
        (
            done.opened,
            done.closed,
            done.open,
            done.no_stop,
            done.extra_stops
        ),
        (1, 1, false, 0, 0)
    );
    assert!(gate_live(&seq(&["UserPromptSubmit", "Stop"])));
    // The first prompt is still being answered: no Stop yet, nothing proven.
    let running = seq(&["SessionStart", "UserPromptSubmit"]);
    assert!(turns(&running).open && !gate_live(&running));
    // A Stop from the previous turn is enough while the next turn runs.
    assert!(gate_live(&seq(&[
        "UserPromptSubmit",
        "Stop",
        "UserPromptSubmit"
    ])));
    // A Stop with no prompt before it is not a finished turn.
    assert!(!gate_live(&seq(&["SessionStart", "Stop"])));
}

#[test]
fn a_turn_whose_stop_never_arrived_is_flagged() {
    let t = turns(&seq(&["UserPromptSubmit", "UserPromptSubmit", "Stop"]));
    assert_eq!((t.opened, t.closed, t.no_stop), (2, 1, 1));
    let t = turns(&seq(&["UserPromptSubmit", "SessionEnd"]));
    assert_eq!((t.no_stop, t.closed, t.open), (1, 0, false));
    let t = turns(&seq(&[
        "UserPromptSubmit",
        "PreToolUse",
        "SessionEnd",
        "SessionStart",
        "UserPromptSubmit",
    ]));
    assert_eq!((t.no_stop, t.open), (1, true));
}

#[test]
fn a_stop_after_a_blocking_stop_is_a_continuation_not_a_fault() {
    // Muse runs the turn again after a Stop hook blocks it, so several Stops follow one prompt.
    let t = turns(&seq(&["UserPromptSubmit", "Stop", "Stop", "Stop"]));
    assert_eq!((t.closed, t.no_stop, t.extra_stops), (1, 0, 2));
}

#[test]
fn a_stop_failure_closes_the_turn() {
    let t = turns(&seq(&[
        "UserPromptSubmit",
        "StopFailure",
        "UserPromptSubmit",
        "Stop",
    ]));
    assert_eq!((t.opened, t.closed, t.no_stop), (2, 2, 0));
}

#[test]
fn events_that_arrived_without_their_pair_are_listed() {
    let all = |_: &str| true;
    let none = unpaired(
        &seq(&[
            "PreToolUse",
            "PostToolUse",
            "PreToolUse",
            "PostToolUseFailure",
        ]),
        all,
    );
    assert!(none.is_empty());
    // One opening more than closings may still be running.
    assert!(unpaired(&seq(&["PreToolUse", "PostToolUse", "PreToolUse"]), all).is_empty());
    let lost = unpaired(
        &seq(&["PreToolUse", "PreToolUse", "PreToolUse", "PostToolUse"]),
        all,
    );
    assert_eq!(
        lost,
        ["2 PreToolUse without PostToolUse/PostToolUseFailure"]
    );
    let extra = unpaired(&seq(&["PostToolUse", "PostToolUse", "PreToolUse"]), all);
    assert_eq!(
        extra,
        ["1 PostToolUse/PostToolUseFailure without PreToolUse"]
    );
    let sub = unpaired(
        &seq(&["SubagentStart", "SubagentStart", "SubagentStart"]),
        all,
    );
    assert_eq!(sub, ["3 SubagentStart without SubagentStop"]);
}

#[test]
fn an_event_nobody_listens_to_proves_nothing() {
    // The hooks are not all registered: a missing beat is then no fault.
    let only_pre = |e: &str| e == "PreToolUse";
    assert!(unpaired(&seq(&["PreToolUse", "PreToolUse", "PreToolUse"]), only_pre).is_empty());
    assert!(unpaired(&seq(&["PreToolUse"; 4]), |_| false).is_empty());
}

// ---------------------------------------------------------------- a failure never reaches Muse

/// Runs only as the child of `a_write_error_never_fails_the_hook`.
#[test]
fn beat_in_a_child_process() {
    if std::env::var_os("SHIM_BEAT_CHILD").is_none() {
        return;
    }
    println!("<<<");
    liveness::beat("Stop", "s1", true);
    println!(">>>");
}

#[test]
fn a_write_error_never_fails_the_hook() {
    let t = Tmp::new("failure");
    let blocker = t.join("a-file-where-the-vault-should-be");
    std::fs::write(&blocker, "x").unwrap();
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "beat_in_a_child_process",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("SHIM_BEAT_CHILD", "1")
        .env("TORPOR_HOME", &blocker)
        .env("HOME", t.join("home"))
        .output()
        .unwrap();
    let (stdout, stderr) = (
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(out.status.success(), "{stdout}\n{stderr}");
    let during = stdout
        .split("<<<\n")
        .nth(1)
        .and_then(|s| s.split(">>>\n").next())
        .unwrap();
    assert_eq!(
        during, "",
        "beat() wrote to stdout, where Muse reads the hook's answer"
    );
    assert!(
        stderr.contains("torpor-muse: beat Stop s1:"),
        "the error is not lost: {stderr}"
    );
    // The direct calls say the same: an error value, never a panic.
    assert!(beat_in(&blocker, "Stop", "s1", true, 1).is_err());
    liveness::log_error(&blocker, "still no panic");
}

#[test]
fn the_error_log_is_cut_when_it_passes_64_kib() {
    let t = Tmp::new("errors");
    let msg = format!("a failure\nwith a second line {}", "z".repeat(1000));
    for _ in 0..100 {
        liveness::log_error(&t.0, &msg);
    }
    let f = t.join("muse/shim-errors.log");
    let size = std::fs::metadata(&f).unwrap().len();
    assert!(size < 70 * 1024 && size > 0, "{size}");
    let raw = std::fs::read_to_string(&f).unwrap();
    assert!(raw.lines().count() < 100, "it was cut at least once");
    let first = raw.lines().next().unwrap();
    let (ms, rest) = first.split_once(' ').unwrap();
    assert!(ms.parse::<u64>().unwrap() > 1_600_000_000_000);
    assert!(
        rest.starts_with("a failure with a second line"),
        "one line per error"
    );
    assert_eq!(mode(&f), 0o600);
}

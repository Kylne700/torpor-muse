//! `torpor-muse doctor` against made-up heartbeats, Muse session logs and XDG_DATA_HOME.
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

use common::{LogBuilder, append, fixture, sid_of};
use doctor::Level;
use doctor_common::{
    T0, World, every_problem_says_how, hook, install, must, mustnt, set_mtime, suggest,
};
use std::fs;

#[test]
fn the_gate_is_live_only_after_a_finished_turn() {
    let w = World::new("gate");
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "gate not proven live: no heartbeat from a main session",
    );
    assert_eq!(doctor::finish(&r), 1);

    w.beat("s1", true, "SessionStart", 500);
    w.beat("s1", true, "UserPromptSubmit", 1000);
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "session s1 has 1 prompt(s) and no finished turn",
    );
    mustnt(&r, Level::Ok, "gate live");
    every_problem_says_how(&r);

    w.beat("s1", true, "Stop", 2000);
    let r = w.examine();
    must(&r, Level::Ok, "gate live: Stop ran");
    mustnt(&r, Level::Fix, "gate not proven live");

    // the next prompt is still being answered: the Stop of the turn before still proves it
    w.beat("s1", true, "UserPromptSubmit", 3000);
    must(&w.examine(), Level::Ok, "gate live: Stop ran");
}

#[test]
fn a_stop_from_before_the_settings_changed_proves_nothing() {
    let w = World::new("older");
    w.beat("s1", true, "UserPromptSubmit", 1000);
    w.beat("s1", true, "Stop", 2000);
    must(&w.examine(), Level::Ok, "gate live");
    set_mtime(&w.settings, T0 + 30_000); // init rewrote the settings after that turn
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "gate not proven live since the settings changed",
    );
    mustnt(&r, Level::Ok, "gate live");
}

#[test]
fn the_probes_own_heartbeats_do_not_prove_the_gate() {
    let w = World::new("probebeats");
    w.beat(liveness::PROBE_SID, true, "UserPromptSubmit", 1000);
    w.beat(liveness::PROBE_SID, true, "Stop", 2000);
    must(&w.examine(), Level::Fix, "no heartbeat from a main session");
}

#[test]
fn observer_sessions_prove_nothing_and_are_listed_apart() {
    let w = World::new("observers");
    for (e, dt) in [
        ("SubagentStart", 100),
        ("PreToolUse", 200),
        ("PreToolUse", 300),
        ("PostToolUse", 400),
        ("Stop", 500),
    ] {
        w.beat("child-1", false, e, dt);
    }
    let r = w.examine();
    must(&r, Level::Fix, "no heartbeat from a main session");
    must(&r, Level::Info, "observer (child) sessions, counted apart:");
    must(&r, Level::Info, "PreToolUse 2x");
    mustnt(&r, Level::Info, "last heartbeat per event, main sessions");
    // a main session next to them proves the gate; the observers stay apart
    w.beat("main-1", true, "UserPromptSubmit", 1000);
    w.beat("main-1", true, "Stop", 2000);
    let r = w.examine();
    must(&r, Level::Ok, "gate live");
    must(&r, Level::Info, "observer (child) sessions, counted apart:");
    must(&r, Level::Info, "last heartbeat per event, main sessions:");
}

#[test]
fn a_turn_without_stop_is_a_warning_and_a_repeated_stop_is_normal() {
    let w = World::new("turns");
    for (e, dt) in [
        ("UserPromptSubmit", 1000),
        ("UserPromptSubmit", 2000),
        ("Stop", 3000),
        ("Stop", 4000),
        ("Stop", 5000),
    ] {
        w.beat("s1", true, e, dt);
    }
    let r = w.examine();
    must(
        &r,
        Level::Warn,
        "1 turn(s) of session s1 ended without a Stop",
    );
    must(&r, Level::Info, "2 extra Stop(s) in session s1");
    must(&r, Level::Info, "when it double-checks its answer"); // dogfood1 n2: no Stop blocked there
    must(&r, Level::Ok, "gate live");
    every_problem_says_how(&r);
    assert_eq!(r.fixes(), 0, "{}", r.text());
    assert_eq!(doctor::finish(&r), 0, "a warning alone does not fail");
}

#[test]
fn events_without_their_pair_are_a_warning_only_when_both_hooks_exist() {
    let w = World::new("pairs");
    let all = [
        ("SessionStart", 10),
        ("UserPromptSubmit", 10),
        ("Stop", 35),
        ("PreToolUse", 6),
        ("PostToolUse", 6),
        ("PostToolUseFailure", 6),
    ];
    w.write_json(&w.settings_value(&all));
    for (e, dt) in [
        ("UserPromptSubmit", 1000),
        ("PreToolUse", 1100),
        ("PreToolUse", 1200),
        ("PreToolUse", 1300),
        ("PostToolUse", 1400),
        ("Stop", 2000),
    ] {
        w.beat("s1", true, e, dt);
    }
    let r = w.examine();
    must(
        &r,
        Level::Warn,
        "2 PreToolUse without PostToolUse/PostToolUseFailure",
    );
    must(&r, Level::Warn, "or the turn was stopped while a tool ran"); // dogfood1 n7: often no rejected output
    every_problem_says_how(&r);
    // with no hook for the second half of the pair there is nothing to compare
    w.write_json(&w.settings_value(&all[..4]));
    mustnt(&w.examine(), Level::Warn, "events without their pair");
}

#[test]
fn errors_the_hooks_logged_are_shown() {
    let w = World::new("errors");
    liveness::log_error(&w.vault, "transcript Stop s1: no such log");
    let r = w.examine();
    must(&r, Level::Warn, "1 error(s) logged by the hooks");
    must(&r, Level::Warn, "transcript Stop s1: no such log");
    must(&r, Level::Warn, "shim-errors.log");
    every_problem_says_how(&r);
}

#[test]
fn only_the_tested_muse_version_is_ok() {
    let w = World::new("version");
    must(&w.examine(), Level::Info, "no Muse session log under");
    let log = |sid: &str, semver: &str| {
        let mut b = LogBuilder::new(sid);
        b.metadata(semver);
        b.text()
    };
    w.put(&log("01a00000-0000-7000-8000-000000000001", "1.4.13"));
    must(
        &w.examine(),
        Level::Ok,
        "Muse Code 1.4.13 (the version these hooks were tested with)",
    );
    // the newest log decides
    w.put(&log("01a00000-0000-7000-8000-000000000002", "1.4.14"));
    let r = w.examine();
    must(
        &r,
        Level::Warn,
        "Muse Code 1.4.14: the hooks and the log translation were tested only with 1.4.13",
    );
    mustnt(&r, Level::Ok, "Muse Code 1.4.13");
    every_problem_says_how(&r);
}

#[test]
fn an_output_muse_rejected_is_found_in_its_log() {
    let w = World::new("rejected");
    let text = fixture("rejected", &w.home.to_string_lossy());
    let log = w.put(&text);
    let r = w.examine();
    must(&r, Level::Fix, "Muse rejected a hook in");
    must(&r, Level::Fix, "Stop failed");
    must(
        &r,
        Level::Fix,
        "unsupported `stop_reason` in output of Stop hook output",
    );
    must(&r, Level::Fix, "saw: {\"stop_reason\":\"x\"}");
    must(&r, Level::Fix, "seen only in Muse's session log");
    must(&r, Level::Fix, &log.display().to_string());
    every_problem_says_how(&r);
    assert_eq!(doctor::finish(&r), 1);

    // a later run of the same hook that went through clears it
    let key = format!("{}:stop:def-971b0d3a108b3a3b", w.settings.display());
    let mut b = LogBuilder::new(&sid_of(&text));
    b.terminal("run-9", &key, "Stop", "completed", None, "");
    append(&log, &b.text());
    mustnt(&w.examine(), Level::Fix, "Muse rejected");
}

#[test]
fn a_hook_that_timed_out_is_found_and_other_statuses_are_not() {
    let w = World::new("timedout");
    let key = |ev: &str| format!("{}:{ev}:def-1", w.settings.display());
    let mut b = LogBuilder::new("01a00000-0000-7000-8000-000000000005");
    b.terminal(
        "run-1",
        &key("user_prompt_submit"),
        "UserPromptSubmit",
        "timed_out",
        Some("hook timed out after 1s"),
        "",
    );
    b.terminal("run-1", &key("stop"), "Stop", "blocked", None, "");
    b.terminal(
        "run-1",
        &key("session_start"),
        "SessionStart",
        "cancelled",
        None,
        "",
    );
    w.put(&b.text());
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "UserPromptSubmit timed_out (hook timed out after 1s)",
    );
    must(&r, Level::Fix, "raise the timeout of that hook");
    mustnt(&r, Level::Fix, "Stop blocked");
    mustnt(&r, Level::Fix, "SessionStart cancelled");
}

#[test]
fn a_rejected_hook_of_another_settings_file_is_not_ours() {
    let w = World::new("elsewhere");
    let mut b = LogBuilder::new("01a00000-0000-7000-8000-000000000006");
    b.terminal(
        "run-1",
        "/somewhere/else/settings.json:stop:def-1",
        "Stop",
        "failed",
        Some("x"),
        "",
    );
    w.put(&b.text());
    mustnt(&w.examine(), Level::Fix, "Muse rejected");
}

#[test]
fn a_prompt_that_never_reached_the_prompt_hook_is_a_fix() {
    let w = World::new("noups");
    let key = |ev: &str| format!("{}:{ev}:def-1", w.settings.display());
    let sid = "01a00000-0000-7000-8000-000000000003";
    let mut b = LogBuilder::new(sid);
    let intent = b.accepted("chat", "main");
    b.terminal(&intent, &key("stop"), "Stop", "completed", None, "");
    let text = b.text();
    w.put(&text);
    let r = w.examine();
    must(&r, Level::Fix, "UserPromptSubmit hook never ran");
    every_problem_says_how(&r);

    // a replayed prompt of a forked session has an id of another kind: no news
    let mut replayed = intent.clone();
    replayed.replace_range(14..15, "5");
    w.put(&text.replace(&intent, &replayed));
    mustnt(&w.examine(), Level::Fix, "UserPromptSubmit hook never ran");

    // the hook ran for that prompt
    b.terminal(
        &intent,
        &key("user_prompt_submit"),
        "UserPromptSubmit",
        "completed",
        None,
        "",
    );
    w.put(&b.text());
    mustnt(&w.examine(), Level::Fix, "UserPromptSubmit hook never ran");

    // only plain chat prompts are held to it: skills and plugin commands are not claimed to reach the hook
    for kind in ["skill_invocation", "plugin_command"] {
        let mut b = LogBuilder::new("01a00000-0000-7000-8000-000000000007");
        let intent = b.accepted(kind, "main");
        b.terminal(&intent, &key("stop"), "Stop", "completed", None, "");
        w.put(&b.text());
        mustnt(&w.examine(), Level::Fix, "UserPromptSubmit hook never ran");
    }
}

#[test]
fn xdg_data_home_is_seen_by_muse_but_not_by_the_hooks() {
    let w = World::new("xdg");
    let x = w.t.join("xdg-data");
    let mut c = w.ctx();
    c.xdg_data = Some(x.clone());
    c.logs = x.join("muse/sessions");
    // the hooks look in $HOME/.local/share/muse/sessions, which is not there
    let r = doctor::examine(&c);
    must(&r, Level::Fix, "the hooks cannot find any session log");
    must(&r, Level::Fix, "ln -s");
    must(&r, Level::Fix, &x.join("muse").display().to_string());
    every_problem_says_how(&r);
    // both exist: Muse writes to one, the hooks read the other
    fs::create_dir_all(w.sessions()).unwrap();
    let r = doctor::examine(&c);
    must(&r, Level::Warn, "both places exist");
    mustnt(&r, Level::Fix, "XDG_DATA_HOME");
    // XDG_DATA_HOME that names the same place is fine
    c.xdg_data = Some(w.home.join(".local/share"));
    c.logs = w.sessions();
    mustnt(&doctor::examine(&c), Level::Warn, "XDG_DATA_HOME");
}

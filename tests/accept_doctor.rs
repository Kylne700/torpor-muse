//! Regression tests for defects the S3a-accept run found in `torpor-muse doctor` (e2e/RESULTS.md, check A8).
//! doctor.rs is compiled in with the same stubs as tests/shim_doctor.rs; only its look at Muse's session logs is used.
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

/// doctor.rs reads the engine's time limit per event from hook.rs; one event is enough here.
mod hook {
    pub struct Ev {
        pub name: &'static str,
        pub engine_ms: u64,
    }
    pub const EVENTS: &[Ev] = &[Ev {
        name: "Stop",
        engine_ms: 31_500,
    }];
}

/// doctor.rs asks install.rs where Muse's settings file is and why `init` would refuse it; these tests hand doctor the
/// path directly and leave the refusal to tests/core_install.rs and the unit tests.
#[allow(dead_code)]
mod install {
    pub fn settings_path() -> std::path::PathBuf {
        std::path::PathBuf::from("settings.json")
    }
    pub fn refusal(_: &std::path::Path) -> Option<String> {
        None
    }
}

/// doctor.rs asks suggest.rs whether the check-writer has a key; here it always has.
#[allow(dead_code)]
mod suggest {
    pub fn writer_off(_: bool) -> Option<String> {
        None
    }
}

#[path = "shim_common.rs"]
mod common;

use common::{LogBuilder, Tmp, put_log};
use doctor::{Ctx, Level};
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

/// LogBuilder stamps record number `n` with `BASE_US + n * 1000` microseconds.
const BASE_US: u64 = 1_793_038_390_000_000;

fn settings_changed_at(settings: &Path, us: u64) {
    fs::File::open(settings)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_micros(us))
        .unwrap();
}

/// A settings file (no torpor-muse hook in it: doctor's look at the logs does not need one) and one Muse log in which
/// record 2 is a failed UserPromptSubmit hook and record 12 a completed one under another definition (another key).
/// `ran_again`: whether the second record is there at all.
fn doctor_says(name: &str, changed_us: u64, ran_again: bool) -> doctor::Report {
    let t = Tmp::new(name);
    let (home, sessions) = (t.join("home"), t.join("home/sessions"));
    let settings = home.join(".config/muse/settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, r#"{"schema_version":1}"#).unwrap();
    settings_changed_at(&settings, changed_us);
    let key = |def: &str| format!("{}:user_prompt_submit:def-{def}", settings.display());
    let mut b = LogBuilder::new("01a179b0-0000-7000-8000-000000000001");
    b.metadata("1.4.13");
    b.terminal(
        "run-1",
        &key("6e0e0bb945156b40"),
        "UserPromptSubmit",
        "failed",
        Some("unsupported `torpor_drill_poison` in output"),
        r#"{"torpor_drill_poison":1}"#,
    );
    for _ in 0..9 {
        b.cbu(
            "run-1",
            "runtime_hook",
            "filler: keeps the two hook records 10 ms apart",
        );
    }
    if ran_again {
        b.terminal(
            "run-2",
            &key("589b55faab57df25"),
            "UserPromptSubmit",
            "completed",
            None,
            "",
        );
    }
    put_log(&sessions, &common::sid_of(&b.text()), &b.text());
    doctor::examine(&Ctx {
        settings,
        home: home.clone(),
        logs: sessions.clone(),
        hook_logs: sessions,
        xdg_data: None,
        env_vault: Some(t.join("vault")),
        shell: None,
        temps: Vec::new(),
        cwd: t.join("home"),
        now_ms: (BASE_US / 1000) + 60_000,
        probe_timeout: Duration::from_secs(5),
    })
}

const REJECTED: &str = "Muse rejected a hook";

fn rejected(r: &doctor::Report) -> bool {
    r.0.iter()
        .any(|(l, s)| *l == Level::Fix && s.contains(REJECTED))
}

#[test]
fn a_hook_that_failed_before_it_was_repaired_is_no_fix_once_the_event_ran_fine_again() {
    // found in A8: after the poisoned command was put back, doctor kept printing the old failure, because the failed key
    // (a hash of the old command) never runs again
    let r = doctor_says("repaired", BASE_US + 7_000, true);
    assert!(!rejected(&r), "{}", r.text());
}

#[test]
fn a_failed_hook_stays_a_fix_when_the_settings_did_not_change_after_it() {
    // another hook of the same event ran fine later, but this one is still in the file and still failing
    let r = doctor_says("unchanged", BASE_US + 500, true);
    assert!(rejected(&r), "{}", r.text());
}

#[test]
fn a_failed_hook_stays_a_fix_until_its_event_has_run_since_the_settings_changed() {
    let r = doctor_says("not_yet", BASE_US + 7_000, false);
    assert!(rejected(&r), "{}", r.text());
}

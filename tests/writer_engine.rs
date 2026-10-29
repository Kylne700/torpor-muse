//! The check-writer with the real Torpor engine (TORPOR_BIN) behind a mock model: the engine lints the check,
//! qualifies it on real saved versions of a synthetic app, and saves it. Skipped, with a message, without an
//! engine binary. Owner: S3b-writer.
mod writer_common;

use serde_json::json;
use writer_common::*;

const PRAISE: &str = "login works now, alice can sign in and then see her account";

/// Linux: what a stopped run leaves behind is found by its working folder (`/proc/<pid>/cwd`).
#[cfg(target_os = "linux")]
mod left_behind {
    use std::path::{Path, PathBuf};

    /// The live processes whose working folder is inside `dir`, as (pid, session id).
    pub fn inside(dir: &Path) -> Vec<(i32, i32)> {
        let Ok(dir) = dir.canonicalize() else {
            return Vec::new();
        };
        let procs = std::fs::read_dir("/proc").into_iter().flatten().flatten();
        procs
            .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
            .filter(|pid| {
                std::fs::read_link(format!("/proc/{pid}/cwd")).is_ok_and(|c| c.starts_with(&dir))
            })
            // SAFETY: getsid(2) only reads.
            .map(|pid| (pid, unsafe { libc::getsid(pid) }))
            .collect()
    }

    /// Kills what is still inside the sandbox when the test ends, by exact pid, each folder read again just before.
    pub struct Guard(pub PathBuf);

    impl Drop for Guard {
        fn drop(&mut self) {
            for (pid, _) in inside(&self.0) {
                // SAFETY: SIGKILL to a pid whose working folder was just read as inside this test's own sandbox.
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    }
}

fn with_engine(tag: &str) -> Option<Sandbox> {
    let mut sb = Sandbox::new(tag);
    sb.engine = Some(real_engine()?);
    Some(sb)
}

/// The run the Muse hook starts after the praise prompt, with the engine's hooks already fed.
fn hook_run(sb: &Sandbox, base: &str) -> Ran {
    sb.suggest(
        Some(base),
        "-",
        &[
            ("TORPOR_MUSE_FROM_HOOK", "s1"),
            ("TORPOR_MUSE_WORDS", PRAISE),
        ],
    )
}

/// S2 on F4b: with the user's gate off (what the first Muse session in a folder sets) the engine still says "In shadow for
/// now ... starts to stop it", and the line the user reads must not. The test above keeps the engine's own words with the
/// gate on, as a new store has it.
#[test]
fn with_the_gate_off_the_proven_line_says_what_the_ai_is_told_and_not_a_stop() {
    let Some(sb) = with_engine("gate-off") else {
        return;
    };
    sb.engine_history(PRAISE);
    let (code, out, err) = sb.torpor(&["gate", "off"], None);
    assert_eq!(code, 0, "{out}{err}");
    let (_, now, _) = sb.torpor(&["gate"], None);
    assert!(now.starts_with("The gate is off"), "{now}");
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = hook_run(&sb, &mock.base());
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 0, "{ctx}");
    assert!(
        r.out.starts_with("proven: Promise #1 ") && r.out.contains(" Proof: passed "),
        "{ctx}"
    );
    assert!(
        !r.out.contains("In shadow") && !r.out.contains("starts to stop"),
        "{ctx}"
    );
    assert!(
        r.out.contains("it does not stop the AI") && r.out.contains("`torpor gate on`"),
        "{ctx}"
    );
    assert_eq!(sb.result()["outcome"], "proven", "{ctx}");
}

#[test]
fn a_praise_becomes_a_proven_check_that_catches_a_break_and_ignores_a_harmless_edit() {
    let Some(sb) = with_engine("e2e") else { return };
    sb.engine_history(PRAISE);
    let mut answer = good_answer();
    answer["reason"] = json!("a wrong password is not tried");
    let mock = Mock::ok(&sb.dir, &answer);
    let r = hook_run(&sb, &mock.base());
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 0, "{ctx}");
    assert!(
        r.out.starts_with("proven: ") && r.out.contains("In shadow for now"),
        "{ctx}"
    );
    assert_eq!(sb.result()["outcome"], "proven", "{ctx}");

    let (_, list, _) = sb.torpor(&["promise", "list"], None);
    assert!(
        list.contains("login works") && list.contains("your words, turn 2"),
        "{list}"
    );
    // what the check leaves out is on the Why line the engine prints (L3 N4)
    let (_, show, _) = sb.torpor(&["promise", "show", "1"], None);
    assert!(
        show.contains("Why: login works for alice Not covered: a wrong password is not tried"),
        "{show}"
    );

    // it passes on the app as it is
    let (_, out, _) = sb.torpor(&["check", "1"], None);
    assert!(out.contains("PASS") && !out.contains("FAIL"), "{out}");

    // a break is caught, a harmless edit is not flagged
    let file = sb.ws.join("server.js");
    let app = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, app.replace("!== body.password", "=== body.password")).unwrap();
    let (_, out, _) = sb.torpor(&["check", "1"], None);
    assert!(out.contains("FAIL"), "the break was not caught: {out}");
    std::fs::write(
        &file,
        app.replace("Small notes for small teams.", "Notes, kept small."),
    )
    .unwrap();
    let (_, out, _) = sb.torpor(&["check", "1"], None);
    assert!(out.contains("PASS") && !out.contains("FAIL"), "{out}");
    std::fs::write(&file, &app).unwrap();

    // the same praise again does not call the model a second time
    let again = hook_run(&sb, &mock.base());
    assert!(again.out.starts_with("skipped: "), "{}", again.out);
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn the_engines_own_lint_words_are_quoted_back_once_then_the_check_stays_advisory() {
    let Some(sb) = with_engine("lint") else {
        return;
    };
    sb.engine_history(PRAISE);
    let mut bad = good_answer();
    bad["steps"][0]["path"] = json!("http://evil.example/api/login");
    let mock = Mock::ok(&sb.dir, &bad);
    let r = hook_run(&sb, &mock.base());
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 3, "{ctx}");
    assert!(r.out.starts_with("advisory: "), "{ctx}");
    assert_eq!(mock.requests().len(), 2, "{ctx}");
    let second = mock.input(1);
    let at = second.find("<<<DATA runner_message").expect(&ctx);
    assert!(
        second[at..].contains("Rejected"),
        "the engine's words: {second}"
    );
    let (_, list, _) = sb.torpor(&["promise", "list"], None);
    assert!(!list.contains("login works"), "nothing was saved: {list}");
}

#[test]
fn a_run_stopped_midway_leaves_at_most_a_proposed_row_and_a_rerun_does_not_pile_on() {
    let Some(sb) = with_engine("killed") else {
        return;
    };
    #[cfg(target_os = "linux")]
    let _clean = left_behind::Guard(sb.dir.clone()); // a failing run leaves nothing behind
    sb.engine_history(PRAISE);
    let mock = Mock::ok(&sb.dir, &good_answer());
    // the proof takes about twenty seconds on this app; with 12 s the engine call gets about 7 and is stopped midway
    let r = sb.suggest(
        Some(&mock.base()),
        "-",
        &[
            ("TORPOR_MUSE_FROM_HOOK", "s1"),
            ("TORPOR_MUSE_WORDS", PRAISE),
            ("TORPOR_MUSE_HARD_TIMEOUT_S", "12"),
        ],
    );
    let ctx = format!("out={:?} err={:?} secs={}", r.out, r.err, r.secs);
    assert_eq!(r.code, 3, "{ctx}");
    assert!(r.out.contains("did not finish in time"), "{ctx}");
    assert_eq!(sb.result()["outcome"], "advisory", "{ctx}");
    // The engine starts the app under test in a process group of its own, and stops it only when it exits itself. Stopped
    // from outside it leaves that app running: nothing may still run with its working folder in this sandbox.
    #[cfg(target_os = "linux")]
    {
        let mut left = left_behind::inside(&sb.dir);
        for _ in 0..60 {
            if left.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            left = left_behind::inside(&sb.dir);
        }
        assert!(
            left.is_empty(),
            "still running in the sandbox after the stopped run (pid, session): {left:?}"
        );
    }
    // the stopped add may leave a `proposed` row (the engine drops those itself after 15 minutes); never a saved check
    let (_, list, _) = sb.torpor(&["promise", "list"], None);
    let rows: Vec<&str> = list.lines().filter(|l| l.contains("login works")).collect();
    assert!(rows.iter().all(|l| l.contains(" · proposed · ")), "{list}");

    // the engine refuses the same check while that row stands ("already has this check"): the rerun stays out of it
    let again = hook_run(&sb, &mock.base());
    assert!(again.out.starts_with("skipped: "), "{}", again.out);
    assert_eq!(mock.requests().len(), 1, "the model was asked again");
}

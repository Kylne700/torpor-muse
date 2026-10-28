//! The whole path of a praise: Muse's hook (the real `torpor-muse hook UserPromptSubmit`) gives the prompt to the
//! engine, starts the detached `suggest`, and returns; the run asks a mock model and the real engine proves the check.
//! Skipped, with a message, without an engine binary (TORPOR_BIN). Owner: S3b-writer.
mod writer_common;

use serde_json::json;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use writer_common::*;

const PRAISE: &str = "login works now, alice can sign in and then see her account";

fn private(path: &std::path::Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

/// The real `torpor-muse hook UserPromptSubmit` for `prompt` of session `sid`, in the sandbox project; waits for its
/// pipes to close.
fn hook(
    sb: &Sandbox,
    engine: &std::path::Path,
    base: &str,
    sid: &str,
    prompt: &str,
) -> std::process::Output {
    let payload = json!({"session_id": sid, "cwd": sb.ws, "hook_event_name": "UserPromptSubmit",
        "prompt": prompt, "turn_id": "t2"});
    let mut hook = Command::new(env!("CARGO_BIN_EXE_torpor-muse"))
        .args(["hook", "UserPromptSubmit"])
        .current_dir(&sb.ws)
        .env_clear()
        .env("PATH", path_var())
        .env("HOME", sb.home())
        .env("TORPOR_HOME", sb.th())
        .env("TORPOR_BIN", engine)
        .env("TORPOR_MUSE_STATE", sb.state())
        .env("TORPOR_MUSE_API_BASE", base)
        .envs(std::env::var("TMPDIR").map(|t| ("TMPDIR", t)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    hook.stdin
        .take()
        .unwrap()
        .write_all(payload.to_string().as_bytes())
        .unwrap();
    hook.wait_with_output().unwrap()
}

#[test]
fn the_hook_returns_at_once_and_the_started_run_ends_with_a_proven_check() {
    let Some(engine) = real_engine() else { return };
    let mut sb = Sandbox::new("hookpath");
    sb.engine = Some(engine.clone());
    sb.engine_first_turn();

    // what the user sets up for the trigger: a config that names a private key file (the hook has no key of its own)
    let key = sb.dir.join("meta.key");
    private(&key, FAKE_KEY);
    private(
        &sb.home().join(".config/torpor-muse/config"),
        &format!("key_file = {}\n", key.display()),
    );

    let mock = Mock::ok(&sb.dir, &good_answer());
    let t0 = Instant::now();
    let o = hook(&sb, &engine, &mock.base(), "s1", PRAISE);
    // output() waits for both pipes to close: a run that kept them open would hold this call for the whole proof
    let took = t0.elapsed();
    let ctx = format!(
        "hook out={:?} err={:?}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(o.status.code(), Some(0), "{ctx}");
    eprintln!("HOOK returned after {took:?}");
    assert!(took < Duration::from_secs(8), "the hook took {took:?}");

    let mut result = sb.result();
    while result["outcome"].is_null() && t0.elapsed() < Duration::from_secs(240) {
        std::thread::sleep(Duration::from_millis(500));
        result = sb.result();
    }
    assert_eq!(result["outcome"], "proven", "{ctx}; result={result}");
    std::thread::sleep(Duration::from_millis(500)); // the run lets go of its lock just after the result file

    let requests = mock.requests();
    assert_eq!(requests.len(), 1, "{ctx}");
    let auth = requests[0]["headers"]["authorization"].as_str().unwrap();
    assert!(
        auth.contains(&format!("len={}", "Bearer ".len() + FAKE_KEY.len())),
        "the key from the config's key file was not sent: {auth}"
    );
    let (_, list, _) = sb.torpor(&["promise", "list"], None);
    assert!(
        list.contains("login works") && list.contains("your words, turn 2"),
        "{list}"
    );
    assert!(!sb.private_text().contains(FAKE_KEY), "the key was written");
}

#[test]
fn a_run_that_failed_is_told_to_the_user_at_the_next_prompt_of_the_session_once() {
    let Some(engine) = real_engine() else { return };
    let mut sb = Sandbox::new("hookfail");
    sb.engine = Some(engine.clone());
    sb.engine_first_turn();
    let key = sb.dir.join("meta.key");
    private(&key, FAKE_KEY);
    private(
        &sb.home().join(".config/torpor-muse/config"),
        &format!("key_file = {}\n", key.display()),
    );

    let mock = Mock::start(&sb.dir, "billing-402", 0, &good_answer().to_string());
    let t0 = Instant::now();
    let first = hook(&sb, &engine, &mock.base(), "s1", PRAISE);
    assert_eq!(first.status.code(), Some(0));
    let said = |o: &std::process::Output| String::from_utf8_lossy(&o.stdout).to_string();
    // (this first prompt carries the notice of what the check-writer sends; it is no news about an earlier run)
    assert!(
        !said(&first).contains("earlier"),
        "nothing has failed yet: {}",
        said(&first)
    );

    let mut result = sb.result();
    while result["outcome"].is_null() && t0.elapsed() < Duration::from_secs(240) {
        std::thread::sleep(Duration::from_millis(500));
        result = sb.result();
    }
    assert_eq!(result["code"], "key", "{result}");
    std::thread::sleep(Duration::from_millis(500)); // the run lets go of its lock just after the result file

    // a prompt too short for a new run: it only carries the news
    let second = said(&hook(&sb, &engine, &mock.base(), "s1", "ok"));
    assert!(
        second.contains("check-writer wrote no check")
            && second.contains("Save points and undo are not affected"),
        "{second}"
    );
    let third = said(&hook(&sb, &engine, &mock.base(), "s1", "ok"));
    assert!(!third.contains("check-writer"), "told twice: {third}");
}

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
    let payload = json!({"session_id": "s1", "cwd": sb.ws, "hook_event_name": "UserPromptSubmit",
        "prompt": PRAISE, "turn_id": "t2"});
    let t0 = Instant::now();
    let mut hook = Command::new(env!("CARGO_BIN_EXE_torpor-muse"))
        .args(["hook", "UserPromptSubmit"])
        .current_dir(&sb.ws)
        .env_clear()
        .env("PATH", path_var())
        .env("HOME", sb.home())
        .env("TORPOR_HOME", sb.th())
        .env("TORPOR_BIN", &engine)
        .env("TORPOR_MUSE_STATE", sb.state())
        .env("TORPOR_MUSE_API_BASE", mock.base())
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
    // output() waits for both pipes to close: a run that kept them open would hold this call for the whole proof
    let o = hook.wait_with_output().unwrap();
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

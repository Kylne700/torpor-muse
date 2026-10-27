//! `torpor-muse hook <Event>` as Muse runs it: the real binary, a cleared environment, a throw-away TORPOR_HOME.
//! Most tests use a fake engine that logs what it is given; the last two run the pinned real engine (TORPOR_BIN).
//! Payloads are the redacted G0 recordings in tests/fixtures/core (`<CWD>` is filled in per test).
//! Owner: S3a-core.

use serde_json::{Map, Value, json};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const EXE: &str = env!("CARGO_BIN_EXE_torpor-muse");
const SID: &str = "00000000-0000-7000-8000-000000000001";
const TURN: &str = "00000000-0000-7000-8000-000000000002";

fn pinned() -> &'static str {
    include_str!("../engine.lock")
        .lines()
        .find_map(|l| l.strip_prefix("version "))
        .expect("engine.lock has a version line")
        .trim()
}

/// A stand-in engine (POSIX sh). Per event, files next to it steer it: `sleep.<E>` (seconds to hang), `err.<E>`
/// (stderr), `out.<E>` (stdout), `code.<E>` (exit code). It keeps every payload as `in.<E>.<n>`, every `version` call
/// as a line of `version.calls`, and the pids of what it started as `pid.<E>` and `child.<E>`.
fn write_fake(dir: &Path, version: &str) -> PathBuf {
    let script = format!(
        r#"#!/bin/sh
d=$(dirname "$0")
case "$1" in
version) echo x >> "$d/version.calls"; echo 'torpor {version}'; exit 0 ;;
hook)
  c=$(cat "$d/count.$2" 2>/dev/null || echo 0); c=$((c + 1)); echo $c > "$d/count.$2"
  cat > "$d/in.$2.$c"
  if [ -f "$d/sleep.$2" ]; then
    sleep "$(cat "$d/sleep.$2")" &
    echo $! > "$d/child.$2"; echo $$ > "$d/pid.$2"
    wait
  fi
  [ -f "$d/err.$2" ] && cat "$d/err.$2" >&2
  [ -f "$d/out.$2" ] && cat "$d/out.$2"
  [ -f "$d/code.$2" ] && exit "$(cat "$d/code.$2")"
  exit 0 ;;
esac
exit 64
"#
    );
    let p = dir.join("torpor");
    fs::write(&p, script).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p
}

struct Run {
    code: i32,
    out: String,
    err: String,
    took: Duration,
}

struct Rig {
    dir: PathBuf,
    cwd: PathBuf,
    fake: PathBuf,
    engine: PathBuf,
}

impl Rig {
    /// Fake engine of the pinned version. The one-time "check-writer is off" note (there is no key file here) is marked
    /// as shown already, so it does not turn up in every test; `unseeded` is the rig that still gets it.
    fn new(name: &str) -> Rig {
        let r = Rig::unseeded(name);
        let muse = r.store().join("muse");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&muse)
            .unwrap();
        fs::write(muse.join("seen-keys"), "UserPromptSubmit.writer-off\n").unwrap();
        r
    }

    fn unseeded(name: &str) -> Rig {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("core-hook-{name}"));
        let _ = fs::remove_dir_all(&dir);
        let (cwd, fake) = (dir.join("proj"), dir.join("fake"));
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(&fake).unwrap();
        let engine = write_fake(&fake, pinned());
        Rig {
            dir,
            cwd,
            fake,
            engine,
        }
    }

    /// The pinned real engine from TORPOR_BIN (cargo runs tests from the package root, so a relative path works).
    fn real(name: &str) -> Rig {
        let mut r = Rig::new(name);
        let bin = std::env::var_os("TORPOR_BIN")
            .filter(|b| !b.is_empty())
            .expect(
                "set TORPOR_BIN to the pinned engine (engine.lock) to run the real-engine tests",
            );
        r.engine = fs::canonicalize(bin).expect("TORPOR_BIN does not exist");
        r
    }

    fn store(&self) -> PathBuf {
        self.dir.join("store")
    }

    /// What Muse gives a hook: `$SHELL -c` with a cleared environment, in the project folder. TORPOR_* is what the
    /// installed command bakes in.
    fn command(&self, exe: &str, args: &[&str]) -> Command {
        let mut c = Command::new(exe);
        c.args(args)
            .current_dir(&self.cwd)
            .env_clear()
            .env("HOME", self.dir.join("home"))
            .env("PATH", "/usr/bin:/bin")
            .env("TORPOR_HOME", self.store())
            .env("TORPOR_BIN", &self.engine);
        c
    }

    fn hook_bytes(&self, event: &str, stdin: &[u8]) -> Run {
        let t0 = Instant::now();
        let mut child = self
            .command(EXE, &["hook", event])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut pipe = child.stdin.take().unwrap();
        let data = stdin.to_vec();
        let writer = std::thread::spawn(move || {
            let _ = pipe.write_all(&data); // a hook that stops reading early is not our test's failure
        });
        let o = child.wait_with_output().unwrap();
        let _ = writer.join();
        Run {
            code: o.status.code().unwrap_or(-1),
            out: String::from_utf8_lossy(&o.stdout)
                .trim_end_matches('\n')
                .to_owned(),
            err: String::from_utf8_lossy(&o.stderr).into_owned(),
            took: t0.elapsed(),
        }
    }

    fn hook(&self, event: &str, payload: &Value) -> Run {
        self.hook_bytes(event, payload.to_string().as_bytes())
    }

    /// `hook`, and the hook must exit 0 with stdout Muse accepts.
    fn hook_ok(&self, event: &str, payload: &Value) -> Run {
        let run = self.hook(event, payload);
        assert_eq!(run.code, 0, "{event}: {}", run.err);
        assert_muse_output(event, &run.out);
        run
    }

    fn fixture(&self, name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/core")
            .join(format!("{name}.json"));
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text.replace("<CWD>", self.cwd.to_str().unwrap())).unwrap()
    }

    /// The payloads the fake engine got for `hook <event>`, oldest first.
    fn got(&self, event: &str) -> Vec<Value> {
        (1..)
            .map_while(|n| fs::read_to_string(self.fake.join(format!("in.{event}.{n}"))).ok())
            .map(|t| serde_json::from_str(&t).expect("the engine got JSON"))
            .collect()
    }

    fn set(&self, file: &str, text: &str) {
        fs::write(self.fake.join(file), text).unwrap();
    }

    fn problems(&self) -> String {
        fs::read_to_string(self.store().join("muse/problems.log")).unwrap_or_default()
    }

    fn version_calls(&self) -> usize {
        fs::read_to_string(self.fake.join("version.calls")).map_or(0, |t| t.lines().count())
    }
}

/// Muse's closed output schema (research/59 D5) as seen from outside: nothing on stdout, or one JSON object that
/// starts with `{`, uses only the closed camelCase keys, stays under 12 KiB, and follows the per-event rules.
fn assert_muse_output(event: &str, out: &str) {
    if out.is_empty() {
        return;
    }
    assert!(
        out.starts_with('{'),
        "stdout must start with a brace: {out:.200}"
    );
    assert!(out.len() < 12 * 1024, "{event}: {} bytes", out.len());
    let v: Value =
        serde_json::from_str(out).unwrap_or_else(|e| panic!("{event}: not JSON ({e}): {out:.200}"));
    let top = [
        "decision",
        "reason",
        "continue",
        "stopReason",
        "suppressOutput",
        "systemMessage",
        "hookSpecificOutput",
    ];
    let inner = [
        "hookEventName",
        "additionalContext",
        "permissionDecision",
        "permissionDecisionReason",
        "updatedInput",
        "selectedSkills",
        "decision",
        "updatedMCPToolOutput",
    ];
    let o = v.as_object().unwrap();
    assert!(
        o.keys().all(|k| top.contains(&k.as_str())),
        "{event}: unknown key in {out:.300}"
    );
    if let Some(m) = o.get("systemMessage") {
        assert!(
            m.as_str().unwrap().chars().count() < 1000,
            "{event}: systemMessage too long"
        );
    }
    if o.contains_key("decision") {
        assert_eq!(event, "Stop", "only a Stop may block");
        assert!(
            !o["reason"].as_str().unwrap_or("").trim().is_empty(),
            "a block needs a reason"
        );
    }
    if let Some(h) = o.get("hookSpecificOutput") {
        let h = h.as_object().unwrap();
        assert!(
            h.keys().all(|k| inner.contains(&k.as_str())),
            "{event}: unknown inner key in {out:.300}"
        );
        assert_eq!(h["hookEventName"], event);
        if h.contains_key("additionalContext") {
            let ok = [
                "SessionStart",
                "SubagentStart",
                "UserPromptSubmit",
                "PreToolUse",
                "PostToolUse",
                "PostToolUseFailure",
            ];
            assert!(ok.contains(&event), "{event} takes no additionalContext");
        }
        if let Some(d) = h.get("permissionDecision") {
            assert_eq!(d, "deny", "Muse cannot ask");
            assert!(
                !h["permissionDecisionReason"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .is_empty()
            );
        }
    }
}

/// The payload as the engine should see it: the fixture plus the documented changes and nothing else (research/60
/// AD1, research/59 D11). `transcript_path` is left out on both sides: another module may fill it in.
fn expected_for_engine(event: &str, fixture: &Value) -> Value {
    let mut p: Map<String, Value> = fixture.as_object().unwrap().clone();
    p.remove("transcript_path");
    if let Some(t) = p.get("turn_id").cloned() {
        p.insert("prompt_id".into(), t);
    }
    if matches!(event, "PreToolUse" | "PostToolUse" | "PostToolUseFailure") {
        let tool = p["tool_name"].as_str().unwrap().to_owned();
        let name = match tool.as_str() {
            "bash" => "Bash",
            "edit_file" => "Edit",
            "write_file" => "Write",
            "read_file" => "Read",
            "search" => "Grep",
            other => panic!("no engine name for {other}"),
        };
        p.insert("tool_name".into(), name.into());
        p.remove("tool_response");
        if name != "Bash" {
            for k in ["content", "find", "replace"] {
                p["tool_input"].as_object_mut().unwrap().remove(k);
            }
        } else if event == "PostToolUseFailure" {
            let e: Value = serde_json::from_str(p["error"].as_str().unwrap()).unwrap();
            let text = [&e["output"], &e["terminal_reason"]]
                .into_iter()
                .filter_map(Value::as_str)
                .find(|s| !s.is_empty())
                .unwrap_or("");
            p.insert(
                "error".into(),
                format!("Exit code {}\n{text}", e["exit_code"]).into(),
            );
        }
    }
    Value::Object(p)
}

fn without_transcript(mut v: Value) -> Value {
    v.as_object_mut().unwrap().remove("transcript_path");
    v
}

fn alive(pid: &str) -> bool {
    let o = Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&o.stdout);
    !stat.trim().is_empty() && !stat.trim().starts_with('Z')
}

// ---------------------------------------------------------------- events

/// Every event research/59 lists: (event, fixture, the engine is called). The four with no fixture were never seen
/// firing in G0; they get a minimal payload.
const EVENTS: [(&str, Option<&str>, bool); 19] = [
    ("SessionStart", Some("SessionStart"), true),
    ("UserPromptSubmit", Some("UserPromptSubmit"), true),
    ("PreToolUse", Some("PreToolUse-bash"), true),
    ("PostToolUse", Some("PostToolUse-bash"), true),
    ("PostToolUseFailure", Some("PostToolUseFailure-bash"), true),
    ("PostToolBatch", Some("PostToolBatch"), false),
    ("Stop", Some("Stop"), true),
    ("StopFailure", Some("StopFailure"), true),
    ("SessionEnd", Some("SessionEnd"), true),
    ("PreCompact", Some("PreCompact"), true),
    ("PostCompact", Some("PostCompact"), false),
    ("SubagentStart", Some("SubagentStart"), false),
    ("SubagentStop", Some("SubagentStop"), true),
    ("PreLLMCall", Some("PreLLMCall"), false),
    ("PostLLMCall", Some("PostLLMCall"), false),
    ("PermissionRequest", None, false),
    ("Notification", None, false),
    ("Interrupt", None, false),
    ("SessionFork", None, false),
];

#[test]
fn every_muse_event_is_taken_and_goes_to_the_engine_or_not_as_documented() {
    let r = Rig::new("events");
    for (event, fixture, called) in EVENTS {
        let mut p = match fixture {
            Some(f) => r.fixture(f),
            None => {
                json!({"session_id": SID, "hook_event_name": event, "cwd": r.cwd, "turn_id": TURN})
            }
        };
        if event == "SubagentStop" {
            // the G0 recording is an observer child's own event; a parent's has a child id of its own
            p["child_session_id"] = json!("00000000-0000-7000-8000-000000000009");
            p["turn_id"] = json!(TURN);
        }
        let run = r.hook_ok(event, &p);
        assert!(
            run.out.is_empty(),
            "{event}: a quiet engine means a quiet hook: {}",
            run.out
        );
        assert_eq!(r.got(event).len(), usize::from(called), "{event}");
    }
}

#[test]
fn the_engine_gets_each_recording_in_its_own_form_and_nothing_else_changed() {
    let r = Rig::new("normalise");
    let mut seen = 0;
    for (event, fixture, called) in EVENTS {
        // the tool events have one recording per tool; the others one
        let names: Vec<String> = match (event, fixture) {
            ("PreToolUse" | "PostToolUse", _) => {
                ["bash", "edit_file", "write_file", "read_file", "search"]
                    .iter()
                    .map(|t| format!("{event}-{t}"))
                    .collect()
            }
            ("PostToolUseFailure", _) => vec![
                "PostToolUseFailure-bash".into(),
                "PostToolUseFailure-bash-sandbox".into(),
                "PostToolUseFailure-read_file".into(),
            ],
            ("SessionStart", _) => ["", "-clear", "-compact", "-fork", "-resume"]
                .iter()
                .map(|s| format!("SessionStart{s}"))
                .collect(),
            ("Stop", _) => vec!["Stop".into(), "Stop-active".into()],
            (_, Some(f)) if called && event != "SubagentStop" => vec![f.to_owned()],
            _ => continue,
        };
        for name in names {
            let fx = r.fixture(&name);
            let before = r.got(event).len();
            r.hook_ok(event, &fx);
            let got = r.got(event);
            assert_eq!(got.len(), before + 1, "{name} did not reach the engine");
            assert_eq!(
                without_transcript(got[before].clone()),
                expected_for_engine(event, &fx),
                "{name}"
            );
            seen += 1;
        }
    }
    assert_eq!(
        seen,
        5 + 5 + 3 + 5 + 2 + 4,
        "Pre/PostToolUse 5 each, failures 3, SessionStart 5, Stop 2, then UserPromptSubmit/StopFailure/SessionEnd/PreCompact"
    );
    // the shape the engine reads for a bash failure (research/59 D11)
    let failure = r.got("PostToolUseFailure");
    assert!(
        failure[0]["error"]
            .as_str()
            .unwrap()
            .starts_with("Exit code 2\nls: cannot access")
    );
    assert!(
        failure[1]["error"]
            .as_str()
            .unwrap()
            .starts_with("Exit code 2\n/bin/sh: 1: cannot create")
    );
    assert_eq!(
        failure[2]["error"],
        r.fixture("PostToolUseFailure-read_file")["error"],
        "only bash is reshaped"
    );
}

#[test]
fn observer_children_are_ignored_after_the_heartbeat() {
    let r = Rig::new("children");
    // an observer's own events: turn id = session id (D10), or child id = session id
    let mut ups = r.fixture("UserPromptSubmit");
    ups["turn_id"] = ups["session_id"].clone();
    for (event, p) in [
        ("UserPromptSubmit", ups),
        ("SubagentStop", r.fixture("SubagentStop")),
        ("SubagentStart", r.fixture("SubagentStart")),
    ] {
        let run = r.hook_ok(event, &p);
        assert!(
            run.out.is_empty() && run.err.is_empty(),
            "{event}: {}{}",
            run.out,
            run.err
        );
        assert!(r.got(event).is_empty(), "{event} reached the engine");
    }
}

#[test]
fn unknown_keys_are_passed_on_and_logged_once() {
    let r = Rig::new("unknown-key");
    let mut p = r.fixture("UserPromptSubmit");
    p["brand_new_key"] = json!({"x": 1});
    for _ in 0..2 {
        r.hook_ok("UserPromptSubmit", &p);
    }
    let got = r.got("UserPromptSubmit");
    assert_eq!(got.len(), 2);
    assert_eq!(got[0]["brand_new_key"], json!({"x": 1}));
    assert_eq!(
        r.problems().matches("brand_new_key").count(),
        1,
        "{}",
        r.problems()
    );
}

#[test]
fn payloads_of_one_and_a_half_megabytes_are_delivered_whole() {
    let r = Rig::new("big");
    let mut p = r.fixture("UserPromptSubmit");
    p["prompt"] = json!("x".repeat(1_500_000));
    let run = r.hook_ok("UserPromptSubmit", &p);
    assert!(run.out.is_empty());
    assert_eq!(
        r.got("UserPromptSubmit")[0]["prompt"]
            .as_str()
            .unwrap()
            .len(),
        1_500_000
    );
    // a tool result this size is not forwarded: the engine reads none of it
    let mut p = r.fixture("PostToolUse-write_file");
    p["tool_response"]["structuredPatch"] = json!(["y".repeat(1_500_000)]);
    p["tool_input"]["content"] = json!("z".repeat(1_500_000));
    r.hook_ok("PostToolUse", &p);
    let sent = &r.got("PostToolUse")[0];
    assert!(sent.get("tool_response").is_none() && sent["tool_input"].get("content").is_none());
    assert!(sent.to_string().len() < 5_000);
}

#[test]
fn bad_input_never_fails_a_turn() {
    let r = Rig::new("bad-input");
    let cases: [(&str, &[u8]); 6] = [
        ("Stop", b"not json"),
        ("Stop", b"[1]"),
        ("Stop", b"{}"),
        ("Stop", b""),
        ("Stop", br#"{"session_id": 5}"#),
        ("NoSuchEvent", br#"{"session_id":"s"}"#),
    ];
    for (event, stdin) in cases {
        let run = r.hook_bytes(event, stdin);
        assert_eq!(run.code, 0, "{event} {stdin:?}");
        assert!(run.out.is_empty(), "{}", run.out);
    }
    let log = r.problems();
    for want in ["not a JSON object", "no session_id", "unknown event"] {
        assert!(log.contains(want), "{want:?} missing from {log}");
    }
    assert!(r.got("Stop").is_empty());
}

// ---------------------------------------------------------------- the engine goes wrong

#[test]
fn a_wrong_engine_fails_open_and_tells_the_user_once_per_session() {
    let r = Rig::new("wrong-engine");
    write_fake(&r.fake, "9.9.9");
    let first = r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    let v: Value = serde_json::from_str(&first.out).expect("the user is told");
    assert!(
        v["systemMessage"].as_str().unwrap().contains("NOT saving"),
        "{}",
        first.out
    );
    assert!(
        v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Tell the user")
    );
    // the same session is not told again, and tool events never print the notice
    assert!(
        r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"))
            .out
            .is_empty()
    );
    assert!(
        r.hook_ok("PreToolUse", &r.fixture("PreToolUse-bash"))
            .out
            .is_empty()
    );
    assert!(r.hook_ok("Stop", &r.fixture("Stop")).out.is_empty());
    // another session is
    let mut other = r.fixture("SessionStart");
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000aa");
    assert!(!r.hook_ok("SessionStart", &other).out.is_empty());
    assert!(
        r.problems().matches("9.9.9").count() >= 5,
        "every failure is logged: {}",
        r.problems()
    );
    assert!(
        r.got("SessionStart").is_empty(),
        "a wrong engine is never run"
    );
}

#[test]
fn a_missing_engine_fails_open_too() {
    let mut r = Rig::new("no-engine");
    r.engine = r.dir.join("nowhere/torpor");
    let run = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    assert!(run.out.contains("NOT saving"), "{}", run.out);
    assert!(r.problems().contains("nowhere"));
}

#[test]
fn an_engine_that_exits_with_an_error_does_not_stop_the_turn() {
    let r = Rig::new("engine-error");
    r.set("code.UserPromptSubmit", "3");
    r.set("err.UserPromptSubmit", "boom: database is locked");
    let run = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    assert!(
        run.out.contains("NOT saving"),
        "the user hears of it: {}",
        run.out
    );
    let log = r.problems();
    assert!(
        log.contains("exited 3") && log.contains("boom: database is locked"),
        "{log}"
    );
}

#[test]
fn the_engine_version_is_asked_once_not_on_every_hook() {
    let r = Rig::new("version-cache");
    for _ in 0..3 {
        r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    }
    assert_eq!(r.version_calls(), 1);
    // a different binary at the same path is asked again (size and mtime changed)
    write_fake(&r.fake, &format!("{} ", pinned()));
    for _ in 0..3 {
        r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    }
    assert_eq!(r.version_calls(), 2);
}

#[test]
fn a_stuck_engine_is_killed_with_everything_it_started_and_the_turn_goes_on() {
    let r = Rig::new("stuck");
    r.set("sleep.PreCompact", "30");
    let run = r.hook_ok("PreCompact", &r.fixture("PreCompact"));
    assert!(
        run.took >= Duration::from_millis(2_000),
        "killed before its deadline: {:?}",
        run.took
    );
    assert!(
        run.took < Duration::from_secs(10),
        "the hook hung: {:?}",
        run.took
    );
    assert!(run.out.is_empty());
    assert!(r.problems().contains("timed out"), "{}", r.problems());
    for f in ["pid.PreCompact", "child.PreCompact"] {
        let pid = fs::read_to_string(r.fake.join(f))
            .unwrap()
            .trim()
            .to_owned();
        let t0 = Instant::now();
        while alive(&pid) && t0.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(!alive(&pid), "{f}: process {pid} is still running");
    }
}

// ---------------------------------------------------------------- what Muse is allowed to see

#[test]
fn whatever_the_engine_prints_muse_only_gets_its_closed_schema_within_the_size_limit() {
    let r = Rig::new("schema");
    let huge = "\u{e9}\n".repeat(20_000);
    let ctx = |event: &str, text: &str| json!({"hookSpecificOutput": {"hookEventName": event, "additionalContext": text}});
    let cases = [
        // snake_case and unknown keys; additionalContext and a stray hookEventName on Stop
        (
            "Stop",
            "Stop",
            json!({"decision": "block", "reason": "R", "system_message": "x", "stopReason": "y", "hookSpecificOutput": {"hookEventName": "Stop", "additionalContext": "no"}}),
        ),
        // only Stop may block; the oversized text is cut to fit
        (
            "UserPromptSubmit",
            "UserPromptSubmit",
            json!({"decision": "block", "reason": "r", "systemMessage": "s".repeat(4000), "hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": huge}}),
        ),
        // the engine's failure form says PostToolUse
        (
            "PostToolUseFailure",
            "PostToolUseFailure-bash",
            ctx("PostToolUse", "seen"),
        ),
        (
            "PreCompact",
            "PreCompact",
            ctx("PreCompact", "not allowed here"),
        ),
        // an engine question becomes a deny: Muse hooks cannot ask
        (
            "PreToolUse",
            "PreToolUse-bash",
            json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": "it deletes. Approve only if you meant it."}}),
        ),
    ];
    for (event, fixture, engine_out) in cases {
        r.set(&format!("out.{event}"), &engine_out.to_string());
        let run = r.hook_ok(event, &r.fixture(fixture));
        match event {
            "Stop" => assert_eq!(run.out, r#"{"decision":"block","reason":"R"}"#),
            "UserPromptSubmit" => {
                let v: Value = serde_json::from_str(&run.out).unwrap();
                assert!(v.get("decision").is_none(), "{event} cannot block");
                let ctx = v["hookSpecificOutput"]["additionalContext"]
                    .as_str()
                    .unwrap();
                assert!(ctx.starts_with("[Torpor note"), "{:.200}", ctx);
                assert!(
                    ctx.len() > 6000,
                    "most of the text must survive the cut: {}",
                    ctx.len()
                );
            }
            "PostToolUseFailure" => assert!(
                run.out.contains(r#""hookEventName":"PostToolUseFailure""#),
                "{}",
                run.out
            ),
            "PreCompact" => assert!(run.out.is_empty(), "{}", run.out),
            _ => {
                assert!(
                    run.out.contains(r#""permissionDecision":"deny""#),
                    "{}",
                    run.out
                );
                assert!(
                    run.out.contains("Muse hooks cannot ask") && !run.out.contains("Approve"),
                    "{}",
                    run.out
                );
            }
        }
    }
    // not JSON, or not an object: nothing is printed and the turn goes on
    for junk in ["hello", "[1]", "{\"decision\":", "   "] {
        r.set("out.Stop", junk);
        assert!(
            r.hook_ok("Stop", &r.fixture("Stop")).out.is_empty(),
            "{junk:?}"
        );
    }
    assert!(r.problems().contains("dropped"));
}

#[test]
fn torpor_blocks_the_end_of_a_turn_at_most_twice() {
    let r = Rig::new("stop-cap");
    let block = r#"{"decision":"block","reason":"prove X first"}"#;
    r.set("out.Stop", block);
    let stop = r.fixture("Stop");
    let outs: Vec<String> = (0..4).map(|_| r.hook_ok("Stop", &stop).out).collect();
    assert_eq!(
        outs,
        [block, block, "", ""],
        "Muse allows 8 blocks; the engine's cap is 2 and ours is the same"
    );
    assert!(r.problems().contains("Stop block dropped"));
    // a new turn is a new count
    let mut next = stop.clone();
    next["turn_id"] = json!("00000000-0000-7000-8000-000000000003");
    assert_eq!(r.hook_ok("Stop", &next).out, block);
}

#[test]
fn state_notes_carry_a_sequence_number_so_the_newest_replaces_the_older() {
    let r = Rig::new("sequence");
    let note = |text: &str| {
        json!({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": text}}).to_string()
    };
    for (i, text) in ["state A", "state B"].iter().enumerate() {
        r.set("out.UserPromptSubmit", &note(text));
        let run = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
        let ctx = serde_json::from_str::<Value>(&run.out).unwrap()["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            ctx.starts_with(&format!(
                "[Torpor note #{}: the newest Torpor note replaces all earlier ones]",
                i + 1
            )),
            "{ctx}"
        );
        assert!(ctx.ends_with(text));
    }
    // another session counts for itself
    let mut other = r.fixture("UserPromptSubmit");
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000bb");
    other["turn_id"] = json!("00000000-0000-7000-8000-0000000000bc");
    assert!(
        r.hook_ok("UserPromptSubmit", &other)
            .out
            .contains("[Torpor note #1:")
    );
}

// ---------------------------------------------------------------- the check-writer's note, and text made safe

fn note_parts(out: &str) -> (String, String) {
    let v: Value = serde_json::from_str(out).expect("the user is told");
    let text = |p: &str| {
        v.pointer(p)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    (
        text("/systemMessage"),
        text("/hookSpecificOutput/additionalContext"),
    )
}

#[test]
fn the_first_prompt_says_once_that_the_check_writer_is_off_and_how_to_turn_it_on() {
    let r = Rig::unseeded("writer-note");
    let first = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    let (said, ctx) = note_parts(&first.out);
    assert!(
        said.contains("check-writer is off") && said.contains("key_file"),
        "{said}"
    );
    assert!(said.contains("chmod 600"), "{said}");
    assert!(
        ctx.contains("Tell the user so") && ctx.contains("torpor-muse doctor"),
        "{ctx}"
    );
    // once for this state folder: the same session, and another one, hear nothing more
    let again = r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    assert!(again.out.is_empty(), "{}", again.out);
    let mut other = r.fixture("UserPromptSubmit");
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000aa");
    assert!(r.hook_ok("UserPromptSubmit", &other).out.is_empty());
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
        said.starts_with("saved\n") && said.contains("check-writer is off"),
        "{said}"
    );
    assert!(
        ctx.contains("engine says hello") && ctx.contains("Tell the user so"),
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

// ---------------------------------------------------------------- the real engine

/// One scripted turn through the hooks, like Muse does it: a file is created with write_file and deleted by a bash
/// command in the same turn. `pre_bash` = whether the hook sees that bash command before it runs.
fn turn_with_a_created_and_deleted_file(r: &Rig, pre_bash: bool) {
    let created = r.cwd.join("created.txt");
    fs::write(r.cwd.join("base.txt"), "keep\n").unwrap();
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));

    r.hook_ok("PreToolUse", &r.fixture("PreToolUse-write_file"));
    fs::write(&created, "new file\n").unwrap();
    r.hook_ok("PostToolUse", &r.fixture("PostToolUse-write_file"));

    let (mut pre, mut post) = (r.fixture("PreToolUse-bash"), r.fixture("PostToolUse-bash"));
    for p in [&mut pre, &mut post] {
        p["tool_input"]["command"] = json!("rm created.txt");
        p["tool_use_id"] = json!("call_rm_1");
    }
    if pre_bash {
        r.hook_ok("PreToolUse", &pre);
    }
    fs::remove_file(&created).unwrap();
    r.hook_ok("PostToolUse", &post);
    r.hook_ok("Stop", &r.fixture("Stop"));
}

fn engine_cli(r: &Rig, args: &[&str]) -> (i32, String) {
    engine_cli_stdin(r, args, "")
}

/// The engine's own command line, run the way a person runs it (nothing from Claude Code in the environment).
fn engine_cli_stdin(r: &Rig, args: &[&str], stdin: &str) -> (i32, String) {
    let mut child = r
        .command(r.engine.to_str().unwrap(), args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let o = child.wait_with_output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    (o.status.code().unwrap_or(-1), text)
}

#[test]
fn real_engine_a_file_made_and_deleted_in_one_turn_comes_back_only_with_the_pre_image() {
    // with the hook seeing the delete before it runs, `torpor back 1` brings the file back
    let r = Rig::real("real-restore");
    turn_with_a_created_and_deleted_file(&r, true);
    assert!(!r.cwd.join("created.txt").exists());
    let (code, text) = engine_cli(&r, &["back", "1"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(
        fs::read_to_string(r.cwd.join("created.txt")).unwrap(),
        "new file\n",
        "{text}"
    );
    assert!(
        text.contains("before rm created.txt"),
        "the save point is named after the command: {text}"
    );
    // negative control: the same turn without the PreToolUse of the delete has nothing to bring back
    let c = Rig::real("real-restore-control");
    turn_with_a_created_and_deleted_file(&c, false);
    let (_, text) = engine_cli(&c, &["back", "1"]);
    assert!(
        !c.cwd.join("created.txt").exists(),
        "no pre-image, no file: {text}"
    );
}

/// AD3 (research/60): a promise the AI adds through its bash tool must not count as the user's words. The engine knows
/// who ran `torpor keep` only from the row PreToolUse(Bash) leaves (or CLAUDECODE, which Muse does not set), so the same
/// promise made without that row is the control: it has to reach the stronger tier, or the test proves nothing.
/// (The engine reads the turn's origin from the transcript; with no transcript the origin is unknown. Once the shim
/// supplies one that says "human", the engine counts an AI-run keep in that turn as the user's words: see the report.)
#[test]
fn real_engine_an_ai_run_keep_stays_advisory_and_the_same_keep_from_the_user_gets_shadow() {
    const WORDS: &str = "the proof page must answer";
    const SPEC: &str = r#"{"v":1,"name":"proof page answers","why":"the proof page must answer","start":{"cmd":"python3 -m http.server {port} --bind 127.0.0.1","ready":"/","timeout_s":90},"env":{},"steps":[{"http":{"method":"GET","path":"/proof.html"},"expect":{"status":200},"why":"the proof page answers"}],"unverifiable":false}"#;
    // turn 1 leaves a save point without the page; the page is then made by hand, outside any turn, so the check is provable
    let rig = |name: &str| {
        let r = Rig::real(name);
        r.hook_ok("SessionStart", &r.fixture("SessionStart"));
        r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
        r.hook_ok("Stop", &r.fixture("Stop"));
        fs::write(r.cwd.join("proof.html"), "<p>proof</p>\n").unwrap();
        r
    };
    let keep = |r: &Rig| {
        let (code, text) = engine_cli_stdin(r, &["keep", WORDS, "--check", "-"], SPEC);
        assert_eq!(code, 0, "{text}");
        assert!(
            text.contains("failed 2 of 2 on the start of turn 1"),
            "the check must be provable: {text}"
        );
    };

    let user = rig("real-ad3-user");
    keep(&user);
    let (_, list) = engine_cli(&user, &["promise", "list"]);
    assert!(
        list.contains("· shadow ·") && list.contains("your words"),
        "{list}"
    );

    let ai = rig("real-ad3-ai");
    let in_turn_2 = |mut p: Value| {
        p["turn_id"] = json!("00000000-0000-7000-8000-000000000012");
        p
    };
    ai.hook_ok(
        "UserPromptSubmit",
        &in_turn_2(ai.fixture("UserPromptSubmit")),
    );
    let mut pre = in_turn_2(ai.fixture("PreToolUse-bash"));
    pre["tool_input"]["command"] = json!(format!("torpor keep \"{WORDS}\" --check - < spec.json"));
    ai.hook_ok("PreToolUse", &pre);
    keep(&ai);
    let stop = ai.hook_ok("Stop", &in_turn_2(ai.fixture("Stop")));
    assert!(stop.out.contains("proposed by the AI"), "{}", stop.out);
    let (_, list) = engine_cli(&ai, &["promise", "list"]);
    assert!(
        list.contains("· advisory ·") && list.contains("the AI"),
        "{list}"
    );
}

#[test]
fn real_engine_every_reply_in_a_session_is_valid_muse_output() {
    let r = Rig::real("real-session");
    turn_with_a_created_and_deleted_file(&r, true); // asserts every reply on the way
    for (event, fixture) in [
        ("PostToolUseFailure", "PostToolUseFailure-bash"),
        ("PostToolUseFailure", "PostToolUseFailure-bash-sandbox"),
        ("PreCompact", "PreCompact"),
        ("SessionStart", "SessionStart-compact"),
        ("StopFailure", "StopFailure"),
        ("SessionEnd", "SessionEnd"),
    ] {
        let run = r.hook_ok(event, &r.fixture(fixture));
        assert!(
            run.err.is_empty() || !run.err.contains("panicked"),
            "{event}: {}",
            run.err
        );
    }
    // a command that deletes data outside this computer: Muse cannot ask, so it is denied with a reason
    let mut p = r.fixture("PreToolUse-bash");
    p["tool_input"]["command"] = json!("git push --force origin main");
    p["permission_mode"] = json!("default");
    let run = r.hook_ok("PreToolUse", &p);
    assert!(
        run.out.contains(r#""permissionDecision":"deny""#),
        "never an allow, never an ask: {}",
        run.out
    );
    assert!(!run.out.contains("Approve only"), "{}", run.out);
}

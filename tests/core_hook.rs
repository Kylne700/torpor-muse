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
/// as a line of `version.calls`, and the pids of what it started as `pid.<E>` and `child.<E>`. `status --json`,
/// `show N --json` and `back --id P ...` print `out.status`, `out.show.N` and `out.back.P` when those exist; `gate ...`
/// exits `code.gate` (0 when there is no such file); every call that is not `hook` or `version` is a line of `calls.log`.
fn write_fake(dir: &Path, version: &str) -> PathBuf {
    let script = format!(
        r#"#!/bin/sh
d=$(dirname "$0")
case "$1" in hook|version) ;; *) echo "$*" >> "$d/calls.log" ;; esac
case "$1" in
version) echo x >> "$d/version.calls"; echo 'torpor {version}'; exit 0 ;;
status) [ -f "$d/out.status" ] && cat "$d/out.status" && exit 0 ;;
show) [ -f "$d/out.show.$2" ] && cat "$d/out.show.$2" && exit 0 ;;
back) [ -f "$d/out.back.$3" ] && cat "$d/out.back.$3" && exit 0 ;;
gate) [ -f "$d/code.gate" ] && exit "$(cat "$d/code.gate")"; exit 0 ;;
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
        self.hook_env(event, stdin, &[])
    }

    /// `hook_bytes` with more variables in the hook's environment.
    fn hook_env(&self, event: &str, stdin: &[u8], env: &[(&str, &str)]) -> Run {
        let t0 = Instant::now();
        let mut child = self
            .command(EXE, &["hook", event])
            .envs(env.iter().copied())
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

    /// The engine calls the hook made besides `hook` and `version` (the arguments of each, one per entry).
    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.fake.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
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

/// Every event `init` registers: (event, fixture, the engine is called).
const EVENTS: [(&str, &str, bool); 11] = [
    ("SessionStart", "SessionStart", true),
    ("UserPromptSubmit", "UserPromptSubmit", true),
    ("PreToolUse", "PreToolUse-bash", true),
    ("PostToolUse", "PostToolUse-bash", true),
    ("PostToolUseFailure", "PostToolUseFailure-bash", true),
    ("Stop", "Stop", true),
    ("StopFailure", "StopFailure", true),
    ("SessionEnd", "SessionEnd", true),
    ("PreCompact", "PreCompact", true),
    ("SubagentStart", "SubagentStart", false),
    ("SubagentStop", "SubagentStop", true),
];

#[test]
fn every_muse_event_is_taken_and_goes_to_the_engine_or_not_as_documented() {
    let r = Rig::new("events");
    for (event, fixture, called) in EVENTS {
        let mut p = r.fixture(fixture);
        if event == "SubagentStop" {
            // the G0 recording is an observer child's own event; a parent's has a child id of its own
            p["child_session_id"] = json!("00000000-0000-7000-8000-000000000009");
            p["turn_id"] = json!(TURN);
        }
        let run = r.hook_ok(event, &p);
        if event == "SessionStart" {
            // the one exception: the model is given the guide (F4) whatever the engine says
            assert!(context_of(&run).starts_with("[Torpor guide"), "{}", run.out);
        } else {
            assert!(
                run.out.is_empty(),
                "{event}: a quiet engine means a quiet hook: {}",
                run.out
            );
        }
        assert_eq!(r.got(event).len(), usize::from(called), "{event}");
    }
}

#[test]
fn an_event_an_earlier_version_registered_is_answered_with_nothing() {
    // Settings written before this version still name PostToolBatch and PostCompact until `init` is run again.
    let r = Rig::new("retired");
    for event in ["PostToolBatch", "PostCompact"] {
        let p = json!({"session_id": SID, "hook_event_name": event, "cwd": r.cwd, "turn_id": TURN});
        let run = r.hook_ok(event, &p);
        assert!(
            run.out.is_empty() && run.err.is_empty(),
            "{event}: {}{}",
            run.out,
            run.err
        );
        assert!(r.got(event).is_empty(), "{event}: the engine was called");
    }
    assert_eq!(r.problems(), "", "nothing is logged for them");
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
            (_, f) if called && event != "SubagentStop" => vec![f.to_owned()],
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
    assert!(
        said.contains("chmod 600") && said.contains("torpor-muse doctor"),
        "{said}"
    );
    // the notice is for the user: the model's context gets nothing (an eval arm without a key sees what the others see)
    assert!(ctx.is_empty(), "{ctx}");
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

/// The first line of the engine's `torpor gate` in the rig's folder, as a person reads it ("The gate is off. ...").
fn gate_state(r: &Rig) -> String {
    let (code, text) = engine_cli(r, &["gate"]);
    assert_eq!(code, 0, "{text}");
    text.lines().next().unwrap_or("").to_owned()
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

/// The additional context of a hook's output, "" when it printed nothing.
fn context_of(run: &Run) -> String {
    if run.out.is_empty() {
        String::new()
    } else {
        note_parts(&run.out).1
    }
}

/// L3 N1 and N2: a turn that changed a database is kept for its folder (the last 5, newest first), also past
/// SessionEnd. The user gets a short line with the command first; the model gets every kept turn, each with its own
/// command, once per session: at the next prompt, or at the start of a session (startup, resume, compact) in that folder.
#[test]
fn database_turns_reach_the_model_after_an_exit_and_in_later_sessions() {
    let r = Rig::new("db-kept");
    let db_turn = |n: u32, p: u32, files: &[&str]| {
        r.set(
            "out.status",
            &json!({"saving": true, "last_turn": {"n": n}}).to_string(),
        );
        let files: Vec<Value> = files
            .iter()
            .map(|f| json!({"path": f, "class": "tracked", "change": "deleted"}))
            .collect();
        let show = json!({"turn": {"n": n, "start_point": p}, "files": files});
        r.set(&format!("out.show.{n}"), &show.to_string());
        note_parts(&stop_run(&r, n).out).0
    };
    let ups = || context_of(&r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit")));
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    assert_eq!(ups(), "");
    let said = db_turn(3, 9, &["notes.db"]);
    assert_eq!(
        said,
        "Torpor: turn 3 changed notes.db. Get it back: stop the app, then torpor back --id 9 --with-data"
    );
    // exit, then resume (the same session id): the start context has it, and the next prompt does not repeat it
    r.hook_ok("SessionEnd", &r.fixture("SessionEnd"));
    let ctx = context_of(&r.hook_ok("SessionStart", &r.fixture("SessionStart-resume")));
    for part in [
        "Turn 3 changed notes.db",
        "stop the app",
        "every other file",
        "even if the app made a fresh database since",
        "only when the user asks",
    ] {
        assert!(ctx.contains(part), "{part}: {ctx}");
    }
    assert!(
        ctx.ends_with(&format!(
            "The command: {} back --id 9 --with-data",
            r.engine.display()
        )),
        "{ctx}"
    );
    assert_eq!(ups(), "");
    // a second database turn: both are listed, the newer first, each with its own command
    let said = db_turn(5, 12, &["data/app.sqlite"]);
    assert_eq!(
        said,
        "Torpor: turn 5 changed data/app.s... Get it back: stop the app, then torpor back --id 12 --with-data",
        "100 characters: the name is cut, the command stays whole"
    );
    let ctx = ups();
    let (five, three) = (ctx.find("Turn 5 changed"), ctx.find("Turn 3 changed"));
    assert!(five.is_some() && three.is_some() && five < three, "{ctx}");
    assert!(
        ctx.contains("back --id 12 --with-data") && ctx.contains("back --id 9 --with-data"),
        "{ctx}"
    );
    assert_eq!(ups(), "", "once per session");
    // an exit with no new start: the next prompt of the session gets them again
    r.hook_ok("SessionEnd", &r.fixture("SessionEnd"));
    assert!(ups().contains("Turn 5 changed"));
    // compaction, and a new session in the same folder
    assert!(
        context_of(&r.hook_ok("SessionStart", &r.fixture("SessionStart-compact")))
            .contains("Turn 3 changed")
    );
    let mut other = r.fixture("SessionStart");
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000bb");
    let ctx = context_of(&r.hook_ok("SessionStart", &other));
    assert!(
        ctx.contains("Turn 5 changed") && ctx.contains("Turn 3 changed"),
        "{ctx}"
    );
    // long names are cut, never the command; only the last 5 turns stay
    let said = db_turn(
        7,
        20,
        &["data/a-database-file-with-a-very-long-name.sqlite3", "b.db"],
    );
    assert!(
        said.chars().count() <= 100
            && said.starts_with("Torpor: turn 7 changed data/a")
            && said.contains("...")
            && said.ends_with(". Get it back: stop the app, then torpor back --id 20 --with-data"),
        "{said}"
    );
    for n in [8, 10, 11] {
        db_turn(n, n + 20, &["notes.db"]);
    }
    let mut fresh = r.fixture("SessionStart");
    fresh["session_id"] = json!("00000000-0000-7000-8000-0000000000cc");
    let ctx = context_of(&r.hook_ok("SessionStart", &fresh));
    assert!(
        ctx.contains("Turn 11 changed")
            && ctx.contains("Turn 5 changed")
            && !ctx.contains("Turn 3 changed"),
        "{ctx}"
    );
}

/// A Stop after a turn that deleted `files` (database files) and began at save point `p`, with the fake engine's
/// `status --json` and `show N --json` answers set to match: the turn is then kept for its folder (L3 N1).
fn db_stop(r: &Rig, n: u32, p: u32, files: &[String]) -> String {
    r.set(
        "out.status",
        &json!({"saving": true, "last_turn": {"n": n}}).to_string(),
    );
    let rows: Vec<Value> = files
        .iter()
        .map(|f| json!({"path": f, "class": "tracked", "change": "deleted"}))
        .collect();
    let show = json!({"turn": {"n": n, "start_point": p}, "files": rows});
    r.set(&format!("out.show.{n}"), &show.to_string());
    note_parts(&stop_run(r, 100 + n).out).0 // not 1: the session's own id is a subagent's turn
}

const FIVE_SOURCES: [&str; 5] = [
    "SessionStart",
    "SessionStart-clear",
    "SessionStart-compact",
    "SessionStart-fork",
    "SessionStart-resume",
];

/// F4 item 1: every session start (startup, resume, clear, compact, fork) gives the model one short guide: the plain
/// words a user says, the Torpor command for each (a dry run first), and the commands that are the user's own.
#[test]
fn the_model_gets_the_guide_at_every_session_start_and_nowhere_else() {
    let r = Rig::new("guide");
    let engine = r.engine.display().to_string();
    for name in FIVE_SOURCES {
        let run = r.hook_ok("SessionStart", &r.fixture(name));
        let (said, ctx) = note_parts(&run.out);
        assert_eq!(said, "", "{name}: nothing for the user");
        assert!(
            ctx.starts_with("[Torpor guide: stays valid] "),
            "{name}: {ctx}"
        );
        assert_eq!(
            ctx.matches(&engine).count(),
            1,
            "the engine's path, once: {ctx}"
        );
        assert!(
            ctx.chars().count() - engine.len() <= 640,
            "about 700 characters with a path of 60: {}",
            ctx.chars().count()
        );
        for want in [
            "torpor log -n 5, torpor undo N --dry-run, torpor undo N.",
            "torpor back --turn K or --id P",
            "torpor find-break --promise ID (or --check -)",
            "the database note's command, app stopped first",
            "Never run off, on, gate, retain, forget, promise ok/drop/block, runner install",
        ] {
            assert!(ctx.contains(want), "{name}: {want:?} missing: {ctx}");
        }
    }
    // the engine's own startup line stays for the user, and its own context stays in front of the guide
    r.set(
        "out.SessionStart",
        r#"{"systemMessage":"Torpor is saving this folder at the start and end of every turn.","hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"engine words"}}"#,
    );
    let (said, ctx) = note_parts(&r.hook_ok("SessionStart", &r.fixture("SessionStart")).out);
    assert_eq!(
        said,
        "Torpor is saving this folder at the start and end of every turn."
    );
    assert!(
        ctx.starts_with("[Torpor note")
            && ctx.contains("engine words\n\n[Torpor guide: stays valid]"),
        "{ctx}"
    );
    // not at a prompt, not at a stop, not for a subagent's own session
    for event in ["UserPromptSubmit", "Stop", "PreCompact"] {
        let run = r.hook_ok(event, &r.fixture(event));
        assert!(!run.out.contains("Torpor guide"), "{event}: {}", run.out);
    }
    let mut child = r.fixture("SessionStart");
    child["turn_id"] = child["session_id"].clone();
    assert!(r.hook_ok("SessionStart", &child).out.is_empty());
}

/// F4 item 1, the worst case: the engine's context is as long as Muse takes, a systemMessage is near its limit, five
/// database turns have the longest file lists the note allows. Muse drops an answer of 12 KiB or more, so the engine's
/// text is cut, never the database note (the user's data) and not the guide.
#[test]
fn guide_and_database_note_fit_with_the_longest_engine_text_by_cutting_the_engines() {
    let r = Rig::new("guide-fit");
    let files: Vec<String> = (0..30)
        .map(|i| format!("data/a-very-long-database-file-name-{i}.sqlite3"))
        .collect();
    let turns = [(1, 2), (3, 4), (5, 6), (7, 8), (9, 10)];
    for (n, p) in turns {
        db_stop(&r, n, p, &files);
    }
    // quotes, backslashes, tabs, newlines and multibyte text: the encoded text is longer than the raw one
    let nasty = "\"quoted\" \\path\\ \t tab \u{e9}\u{20ac}\u{1f642} line\n";
    let engine = |chars: usize| {
        let context: String = nasty.chars().cycle().take(chars).collect();
        json!({"systemMessage": "\u{e9}".repeat(990), "hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": context}})
            .to_string()
    };
    let mut chars = 3000;
    while engine(chars + 50).len() < 11_800 {
        chars += 50;
    }
    r.set("out.SessionStart", &engine(chars));
    for name in FIVE_SOURCES {
        let run = r.hook_ok("SessionStart", &r.fixture(name)); // under 12 KiB, closed keys
        let (said, ctx) = note_parts(&run.out);
        assert!(
            said.starts_with("\u{e9}\u{e9}\u{e9}") && said.chars().count() > 900,
            "{name}"
        );
        assert!(
            ctx.contains("\n[cut]\n\n[Torpor guide: stays valid] "),
            "{name}: the engine's text is cut"
        );
        for (_, p) in turns {
            assert!(
                ctx.contains(&format!("back --id {p} --with-data")),
                "{name}: {p}"
            );
        }
        assert!(
            ctx.ends_with("back --id 2 --with-data"),
            "{name}: the database note is last"
        );
        assert!(run.out.len() < 12 * 1024, "{name}: {} bytes", run.out.len());
    }
}

/// F4 item 1, the commands that are the user's own: under Muse the shell does not carry the marker the engine's command
/// line looks for, so what stops the model is the engine's PreToolUse answer, passed through. The commands the guide
/// does name pass.
#[test]
fn real_engine_refuses_the_users_own_commands_to_the_model_and_passes_the_guides_commands() {
    let r = Rig::real("only-you");
    fs::write(r.cwd.join("base.txt"), "keep\n").unwrap();
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    let engine = r.engine.display().to_string();
    let ask = |command: &str| -> Value {
        let mut p = r.fixture("PreToolUse-bash");
        p["tool_input"]["command"] = json!(command);
        let run = r.hook_ok("PreToolUse", &p);
        if run.out.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&run.out).unwrap()
        }
    };
    let users = [
        "torpor gate on",
        "torpor gate off",
        "torpor off",
        "torpor on",
        "torpor retain 90",
        "torpor forget",
        "torpor promise ok 1",
        "torpor promise drop 1",
        "torpor promise block 1",
        "torpor runner install",
        "torpor undo 1 --force",
        "cd /tmp && torpor gate on",
        &format!("{engine} gate on"),
    ];
    for command in users {
        let v = ask(command);
        let h = &v["hookSpecificOutput"];
        assert_eq!(h["permissionDecision"], "deny", "{command}: {v}");
        let why = h["permissionDecisionReason"].as_str().unwrap();
        assert!(
            why.contains("only the user can do this"),
            "{command}: {why}"
        );
    }
    let guides = [
        "torpor log -n 5",
        "torpor undo 1 --dry-run",
        "torpor undo 1",
        "torpor back --turn 1 --dry-run",
        "torpor back --id 2 --with-data --dry-run",
        "torpor find-break --promise 1",
        "torpor find-break --check -",
        "torpor check 1",
        "torpor promise dispute 1 \"the check is outdated\"",
        &format!("{engine} undo 1 --dry-run"),
    ];
    for command in guides {
        let v = ask(command);
        assert!(
            v["hookSpecificOutput"]["permissionDecision"].is_null(),
            "{command}: {v}"
        );
    }
}

/// L3 N7: Muse shows `systemMessage` on one line and drops its newlines, so the engine's lines ran together. They are
/// joined with " — " now; what the model reads keeps its newlines.
#[test]
fn the_engine_lines_stay_apart_on_the_one_line_muse_shows() {
    let r = Rig::new("one-line");
    r.set(
        "out.Stop",
        r#"{"systemMessage":"Torpor: save point 4. undo it: torpor undo 3\nTorpor: new promise #1 \"x\" (guard only), from your words at turn 2. Drop it: torpor promise drop 1"}"#,
    );
    let said = note_parts(&r.hook_ok("Stop", &r.fixture("Stop")).out).0;
    assert_eq!(
        said,
        "Torpor: save point 4. undo it: torpor undo 3 — Torpor: new promise #1 \"x\" (guard only), from your words at turn 2. Drop it: torpor promise drop 1"
    );
    r.set(
        "out.UserPromptSubmit",
        r#"{"systemMessage":"one\ntwo","hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"c1\nc2"}}"#,
    );
    let (said, ctx) = note_parts(
        &r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"))
            .out,
    );
    assert_eq!(said, "one — two");
    assert!(ctx.ends_with("]\nc1\nc2"), "{ctx}");
}

/// L3 N4: what a check the writer saved does not cover (its `gaps.json`) goes, short, into the engine's line that
/// announces the promise, and in full to the model of the session that asked for it, once, at its next prompt.
#[test]
fn what_a_new_promise_does_not_cover_reaches_the_user_and_the_model() {
    use std::os::unix::ffi::OsStrExt;
    let r = Rig::new("gap-note");
    let root = fs::canonicalize(&r.cwd).unwrap();
    let h = root
        .as_os_str()
        .as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
    let ws = r.store().join(format!("muse/suggest/ws/{h:016x}"));
    fs::create_dir_all(&ws).unwrap();
    let gap = "editing a note, and that the notes survive a restart of the server";
    let gaps = json!([{"promise": 2, "session": SID, "gap": gap}, {"promise": 1, "session": SID, "gap": "older"}]);
    fs::write(ws.join("gaps.json"), gaps.to_string()).unwrap();
    r.set(
        "out.Stop",
        &json!({"systemMessage": "Torpor: save point 4. undo it: torpor undo 3\nTorpor: new promise #2 \"Adding (and listing) notes works\" (guard only), from your words at turn 2. Drop it: torpor promise drop 2"}).to_string(),
    );
    let said = note_parts(&r.hook_ok("Stop", &r.fixture("Stop")).out).0;
    assert!(
        said.contains(&format!("\"Adding (and listing) notes works\" (guard only; not covered: {gap}), from your words at turn 2. Drop it: torpor promise drop 2")),
        "{said}"
    );
    let ctx = context_of(&r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit")));
    assert!(
        ctx.contains(&format!("#2 does not cover: {gap}"))
            && ctx.contains("#1 does not cover: older"),
        "{ctx}"
    );
    let again = context_of(&r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit")));
    assert!(!again.contains("does not cover"), "once: {again}");
    // another session in the folder did not ask for these checks: its model is not told
    let mut other = r.fixture("UserPromptSubmit");
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000aa");
    assert!(!context_of(&r.hook_ok("UserPromptSubmit", &other)).contains("does not cover"));
}

/// dogfood1 M1: a turn whose shell command rewrote a database (the app made a new, empty one). `torpor undo N` and a
/// plain `torpor back` leave database content as it is, so the Stop line names the command that does put the data back,
/// the next prompt gives the model that command with the engine's path, and each, run exactly as printed, brings the
/// old bytes back. A turn that changed no database keeps today's text.
#[test]
fn real_engine_a_turn_that_rewrote_a_database_names_the_command_that_puts_it_back() {
    let sh = |r: &Rig, cmd: &str, path: &str| {
        let o = r
            .command("/bin/sh", &["-c", cmd])
            .env("PATH", path)
            .env_remove("TORPOR_BIN")
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        (o.status.code().unwrap_or(-1), text)
    };
    let turn = |r: &Rig, file: &str| {
        r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
        let (mut pre, mut post) = (r.fixture("PreToolUse-bash"), r.fixture("PostToolUse-bash"));
        for p in [&mut pre, &mut post] {
            p["tool_input"]["command"] = json!(format!("rm -f {file} && node server.js"));
            p["tool_use_id"] = json!("call_rm_db");
        }
        r.hook_ok("PreToolUse", &pre);
        fs::write(r.cwd.join(file), b"SQLite format 3\0").unwrap();
        r.hook_ok("PostToolUse", &post);
        note_parts(&r.hook_ok("Stop", &r.fixture("Stop")).out).0
    };
    let next = |r: &Rig, n: u32| {
        let mut f = r.fixture("UserPromptSubmit");
        f["turn_id"] = json!(format!("00000000-0000-7000-8000-00000000000{n}"));
        f
    };
    let old = b"SQLite format 3\0the user's notes".to_vec();

    let r = Rig::real("real-db");
    let db = r.cwd.join("notes.db");
    fs::write(&db, &old).unwrap();
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    let said = turn(&r, "notes.db");
    assert!(
        said.contains("undo it: torpor undo 1"),
        "the engine's line stays: {said}"
    );
    // L3 N2: the database line comes first and ends with the command within 100 characters (the status bar is short)
    let first = said.split(" — ").next().unwrap_or("");
    let user = first.split_once("then ").map_or("", |(_, c)| c).to_owned();
    assert!(
        first.starts_with("Torpor: turn 1 changed notes.db. Get it back: stop the app, then ")
            && first.chars().count() <= 100
            && user.starts_with("torpor back --id ")
            && user.ends_with(" --with-data"),
        "{said}"
    );
    // the user's terminal finds the engine as `torpor` on its PATH
    let path = format!("{}:/usr/bin:/bin", r.engine.parent().unwrap().display());
    let (code, text) = sh(&r, &user, &path);
    assert_eq!((code, fs::read(&db).unwrap()), (0, old.clone()), "{text}");

    fs::write(&db, b"SQLite format 3\0").unwrap(); // broken again, for the model's copy of the command
    let (_, ctx) = note_parts(&r.hook_ok("UserPromptSubmit", &next(&r, 3)).out);
    let model = ctx
        .rsplit_once("The command: ")
        .map_or("", |(_, c)| c)
        .to_owned();
    assert_eq!(
        model,
        format!("{}{}", r.engine.display(), &user["torpor".len()..]),
        "{ctx}"
    );
    assert!(ctx.contains("every other file"), "{ctx}");
    let (code, text) = sh(&r, &model, "/usr/bin:/bin");
    assert_eq!((code, fs::read(&db).unwrap()), (0, old), "{text}");
    let again = r.hook_ok("UserPromptSubmit", &next(&r, 4)).out;
    assert!(
        !again.contains("--with-data"),
        "the model is told once: {again}"
    );
    // with saving off the engine's last turn stays the database one: its line must not come back at every Stop
    assert_eq!(engine_cli(&r, &["off"]).0, 0);
    let stop = r.hook_ok("Stop", &r.fixture("Stop")).out;
    assert!(!stop.contains("--with-data"), "{stop}");

    // control: the same turn on a file that is not a database keeps today's text, and the model hears nothing
    let c = Rig::real("real-db-control");
    fs::write(c.cwd.join("notes.txt"), &b"the user's notes"[..]).unwrap();
    c.hook_ok("SessionStart", &c.fixture("SessionStart"));
    let said = turn(&c, "notes.txt");
    assert!(
        said.starts_with("Torpor: save point ") && !said.contains('\n'),
        "{said}"
    );
    assert!(
        !said.contains("--with-data") && !said.contains("database"),
        "{said}"
    );
    let out = c.hook_ok("UserPromptSubmit", &next(&c, 3)).out;
    assert!(out.is_empty(), "{out}");
}

/// The drill's D07 setup: an app that holds a database open in WAL mode with 300 rows in its log (made with python's
/// sqlite3 module). A line on its stdin closes it, which writes the log into the database file and removes the `-wal`
/// and `-shm` files, as an app that stops cleanly does. Killed when the test ends or fails.
struct BusyApp(std::process::Child);

impl BusyApp {
    fn start(db: &Path) -> BusyApp {
        const SRC: &str = r#"
import sqlite3, sys
con = sqlite3.connect(sys.argv[1], isolation_level=None)
con.execute("PRAGMA journal_mode=WAL")
con.execute("PRAGMA wal_autocheckpoint=0")
con.execute("create table t(id integer primary key, v text)")
con.execute("begin")
for i in range(300):
    con.execute("insert into t(v) values (?)", ("row%d" % i,))
con.execute("commit")
print("ready", flush=True)
sys.stdin.readline()
con.close()
"#;
        let mut child = Command::new("python3")
            .args(["-I", "-c", SRC])
            .arg(db)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3 is needed for the database tests");
        let mut said = String::new();
        std::io::BufRead::read_line(
            &mut std::io::BufReader::new(child.stdout.as_mut().unwrap()),
            &mut said,
        )
        .unwrap();
        assert_eq!(said.trim(), "ready");
        BusyApp(child)
    }

    fn stop(mut self) {
        self.0.stdin.take().unwrap().write_all(b"\n").unwrap();
        assert!(self.0.wait().unwrap().success());
    }
}

impl Drop for BusyApp {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn rows_in(db: &Path) -> String {
    let o = Command::new("python3")
        .args(["-I", "-c", "import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute('select count(*) from t').fetchone()[0])"])
        .arg(db)
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_owned()
}

/// F4 item 4 (drill D07). The engine saves no database while its app holds a log that is not empty, and says nothing
/// about it when a command deletes that database, so `torpor back --with-data` could not bring it back ("its database
/// was busy"). The hook now denies such a command with a reason (Muse runs on); once the app has stopped the very same
/// command goes through, the engine then saves the file, and `back --with-data` brings the 300 rows back.
#[test]
fn real_engine_a_command_that_would_delete_a_database_in_use_is_denied_until_its_app_stops() {
    const RM: &str = "rm -f data/app.db data/app.db-wal data/app.db-shm";
    let r = Rig::real("real-busy-db");
    let db = r.cwd.join("data/app.db");
    fs::create_dir(r.cwd.join("data")).unwrap();
    fs::write(r.cwd.join("base.txt"), "keep\n").unwrap();
    let app = BusyApp::start(&db);
    let wal = r.cwd.join("data/app.db-wal");
    assert!(
        fs::metadata(&wal).unwrap().len() > 0,
        "the app's log is not empty"
    );

    let turn2 = |mut p: Value| {
        p["turn_id"] = json!("00000000-0000-7000-8000-000000000012");
        p
    };
    let pre = |cmd: &str, id: &str| {
        let mut p = turn2(r.fixture("PreToolUse-bash"));
        p["tool_input"]["command"] = json!(cmd);
        p["tool_use_id"] = json!(id);
        p
    };
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    r.hook_ok("Stop", &r.fixture("Stop"));
    r.hook_ok("UserPromptSubmit", &turn2(r.fixture("UserPromptSubmit")));

    // while the app runs: the delete is denied, by file names and by folder, with a reason the model can act on
    for cmd in [RM, "rm -rf data"] {
        let out = r.hook_ok("PreToolUse", &pre(cmd, "call_rm_busy")).out;
        let v: Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{cmd}: {e}: {out:?}"));
        let (d, why) = (
            v["hookSpecificOutput"]["permissionDecision"].as_str(),
            v["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap_or(""),
        );
        assert_eq!(d, Some("deny"), "{cmd}: {out}");
        for w in [
            "not run",
            "data/app.db",
            "in use",
            "Stop the app",
            "run the command again",
            "ask the user",
        ] {
            assert!(why.contains(w), "{cmd}: {w:?} is missing from {why}");
        }
    }
    // commands that name no busy database, and read-only uses of it, are not stopped
    for cmd in [
        "ls data",
        "cat data/app.db > /dev/null",
        "rm -f missing.txt",
        "cp data/app.db /tmp/copy-of-app.db.bak",
    ] {
        let out = r.hook_ok("PreToolUse", &pre(cmd, "call_other")).out;
        assert!(!out.contains("permissionDecision"), "{cmd}: {out}");
    }
    assert!(
        db.exists() && wal.exists(),
        "the hook never touches the files"
    );

    // the app stops (its log goes into the database file); the same command now goes through
    app.stop();
    assert!(db.exists() && !wal.exists());
    assert_eq!(rows_in(&db), "300");
    let out = r.hook_ok("PreToolUse", &pre(RM, "call_rm_free")).out;
    assert!(!out.contains("permissionDecision"), "{out}");
    for s in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{s}", db.display()));
    }
    let mut post = turn2(r.fixture("PostToolUse-bash"));
    post["tool_input"]["command"] = json!(RM);
    post["tool_use_id"] = json!("call_rm_free");
    r.hook_ok("PostToolUse", &post);
    let last_stop = r.hook_ok("Stop", &turn2(r.fixture("Stop"))).out;
    assert!(!db.exists());

    // `back --with-data` to the save point the engine took when the command went through brings the 300 rows back;
    // the one it took while the app still ran (the denied call) had nothing to bring back: that is what the deny saves
    let (code, shown) = engine_cli(&r, &["show", "2", "--json"]);
    assert_eq!(code, 0, "{shown}");
    let shown: Value = serde_json::from_str(&shown).unwrap();
    let pres: Vec<u64> = shown["points"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["kind"].as_str().is_some_and(|k| k.starts_with("pre_")))
        .filter_map(|p| p["id"].as_u64())
        .collect();
    assert!(pres.len() >= 2, "{shown}");
    let (early, late) = (pres[0].to_string(), pres[pres.len() - 1].to_string());
    // F4 item 3: that very Stop told the model (a block) and the user (the line) the save point that has the 300 rows, not
    // the turn's start (where the app was running and the engine saved no database)
    let told: Value = serde_json::from_str(&last_stop).unwrap_or(Value::Null);
    let cmd = format!("back --id {late} --with-data");
    let why = blocked(&told).unwrap_or_else(|| panic!("no block: {last_stop}"));
    assert!(
        why.contains("This turn changed or deleted data/app.db (database files).")
            && why.contains(&format!("`{} {cmd}`", r.engine.display())),
        "{why}"
    );
    assert!(
        said(&told).starts_with(&format!(
            "Torpor: turn 2 changed data/app.db. Get it back: stop the app, then torpor {cmd}"
        )),
        "{last_stop}"
    );
    let (_, text) = engine_cli(&r, &["back", "--id", &early, "--with-data", "--dry-run"]);
    assert!(text.contains("its database was busy"), "{text}");
    let (code, text) = engine_cli(&r, &["back", "--id", &late, "--with-data"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(rows_in(&db), "300", "{text}");

    // the drill's own D07: the writer was killed (SIGKILL), no app runs, its log is still not empty. The engine saves
    // such a database no more than a live one, so the delete is denied and the reason names the way out (the next
    // tests walk it); with saving off (nothing is saved anyway) nothing is denied
    let stale = r.cwd.join("data/stale.db");
    drop(BusyApp::start(&stale));
    assert!(
        fs::metadata(format!("{}-wal", stale.display()))
            .unwrap()
            .len()
            > 0
    );
    let out = r
        .hook_ok("PreToolUse", &pre("rm data/stale.db", "call_rm_stale"))
        .out;
    assert!(
        out.contains("permissionDecision")
            && out.contains("a crash left its files behind")
            && out.contains("SELECT count(*) FROM sqlite_master")
            && out.contains("If it is refused again, ask the user")
            && out.contains("in their own terminal")
            && !out.contains("stays refused"),
        "{out}"
    );
    assert_eq!(engine_cli(&r, &["off"]).0, 0);
    let out = r
        .hook_ok("PreToolUse", &pre("rm data/stale.db", "call_rm_stale_off"))
        .out;
    assert!(!out.contains("permissionDecision"), "{out}");
}

// ------------------------------------------------ F4c item 10: the way out of a database a crash left behind

/// The state a database is left in, made with python's sqlite3 module (the library real apps use); the table `t` has
/// 5 committed rows. `wal`: the app was killed with them only in its log. `hot`: killed in the middle of a transaction
/// (a small cache pushes the unfinished pages into the file), so a journal waits to be rolled back. `persist` and
/// `truncate`: closed cleanly in a journal mode that keeps the `-journal` file (zero header, empty). `app`: an app
/// still runs and holds the database open (the one thing that is returned, so the test keeps it alive).
fn leftover(db: &Path, kind: &str) -> Option<BusyApp> {
    const SRC: &str = r#"
import os, signal, sqlite3, sys
kind, db = sys.argv[1], sys.argv[2]
con = sqlite3.connect(db, isolation_level=None)
if kind == "wal":
    con.execute("PRAGMA journal_mode=WAL")
    con.execute("PRAGMA wal_autocheckpoint=0")
elif kind != "hot":
    con.execute("PRAGMA journal_mode=" + kind)
con.execute("create table t(id integer primary key, v text)")
con.execute("begin")
for i in range(5):
    con.execute("insert into t(v) values (?)", ("row%d" % i,))
con.execute("commit")
if kind == "hot":
    con.execute("PRAGMA cache_size=5")
    con.execute("begin")
    for i in range(3000):
        con.execute("insert into t(v) values (?)", ("x" * 500,))
if kind in ("wal", "hot"):
    os.kill(os.getpid(), signal.SIGKILL)  # the app dies: no close, no checkpoint, no rollback
con.close()
"#;
    if kind == "app" {
        return Some(BusyApp::start(db));
    }
    let o = Command::new("python3")
        .args(["-I", "-c", SRC, kind])
        .arg(db)
        .output()
        .expect("python3 is needed for the database tests");
    assert_eq!(
        o.status.success(),
        !matches!(kind, "wal" | "hot"),
        "{kind}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    None
}

/// The commands the deny text offers. The `sqlite3` program is not on every machine (not on the one these were written
/// on), so its form is run only where it is installed.
fn exit_forms() -> Vec<&'static str> {
    let mut forms = vec!["python3 -c"];
    if Command::new("sqlite3").arg("-version").output().is_ok() {
        forms.push("sqlite3 ");
    }
    forms
}

fn second_turn(mut p: Value) -> Value {
    p["turn_id"] = json!("00000000-0000-7000-8000-000000000012");
    p
}

/// A Bash call in the second turn: the reason when the hook denies it, None when it goes through.
fn denial(r: &Rig, cmd: &str, id: &str) -> Option<String> {
    let mut p = second_turn(r.fixture("PreToolUse-bash"));
    p["tool_input"]["command"] = json!(cmd);
    p["tool_use_id"] = json!(id);
    let out = r.hook_ok("PreToolUse", &p).out;
    let v: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
    let h = &v["hookSpecificOutput"];
    match h["permissionDecision"].as_str() {
        Some("deny") => Some(
            h["permissionDecisionReason"]
                .as_str()
                .unwrap_or("")
                .to_owned(),
        ),
        other => {
            assert!(other.is_none(), "{cmd}: {out}");
            None
        }
    }
}

/// What the model meets after the app is gone, with the real engine and real SQLite files: `rm data/app.db` is denied,
/// the model runs the command the text names (`form`: the start of the one it picks), and tries the delete again.
/// `wal` and `hot` are cleared by that one read, so the delete goes through with the engine holding a copy; the other
/// kinds are not, so the delete stays denied and the text sends the model to the user.
fn exit_scenario(kind: &str, form: &str) {
    let r = Rig::real(&format!(
        "real-exit-{kind}-{}",
        form.split_whitespace().next().unwrap()
    ));
    let db = r.cwd.join("data/app.db");
    fs::create_dir(r.cwd.join("data")).unwrap();
    fs::write(r.cwd.join("base.txt"), "keep\n").unwrap();
    let _app = leftover(&db, kind);
    let side = |s: &str| PathBuf::from(format!("{}{s}", db.display()));
    let (wal, shm, jrn) = (side("-wal"), side("-shm"), side("-journal"));
    // the state is what the guard calls busy (so a missing state fails here, not later)
    match kind {
        "wal" | "app" => assert!(fs::metadata(&wal).unwrap().len() > 0 && shm.exists()),
        "hot" => assert_eq!(
            fs::read(&jrn).unwrap()[..8],
            [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7],
            "a journal that waits to be rolled back"
        ),
        "persist" => assert_eq!(fs::read(&jrn).unwrap()[..8], [0; 8]),
        "truncate" => assert_eq!(fs::metadata(&jrn).unwrap().len(), 0),
        _ => unreachable!("{kind}"),
    }
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    r.hook_ok("Stop", &r.fixture("Stop"));
    r.hook_ok(
        "UserPromptSubmit",
        &second_turn(r.fixture("UserPromptSubmit")),
    );

    let why = denial(&r, "rm data/app.db", "call_rm_1")
        .unwrap_or_else(|| panic!("{kind}: the delete is not denied"));
    assert!(
        why.contains("a crash left its files behind")
            && why.contains("If it is refused again, ask the user")
            && !why.contains("stays refused"),
        "{why}"
    );
    // the way out is in the text; with `<db>` filled in it goes through the hook (neither the guard nor the engine
    // denies it) and runs
    let exit = why
        .split('`')
        .skip(1)
        .step_by(2)
        .find(|c| c.starts_with(form))
        .unwrap_or_else(|| panic!("no {form:?} command in {why}"))
        .replace("<db>", "data/app.db");
    assert_eq!(denial(&r, &exit, "call_exit"), None, "{exit}");
    let run = Command::new("sh")
        .args(["-c", &exit])
        .current_dir(&r.cwd)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{exit}: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let clears = matches!(kind, "wal" | "hot");
    let left: Vec<&PathBuf> = [&wal, &shm, &jrn]
        .into_iter()
        .filter(|p| p.exists())
        .collect();
    assert!(db.exists(), "the read never removes the database");
    assert_eq!(
        left.is_empty(),
        clears,
        "{kind}: left after the read: {left:?}"
    );
    // every committed row is there; the unfinished transaction of the `hot` kind is the one thing rolled back
    assert_eq!(rows_in(&db), if kind == "app" { "300" } else { "5" });

    let retry = denial(&r, "rm data/app.db", "call_rm_2");
    if !clears {
        let again = retry.unwrap_or_else(|| panic!("{kind}: the delete went through"));
        assert!(again.contains("ask the user"), "{again}");
        return;
    }
    assert_eq!(
        retry, None,
        "{kind}: the files are gone, the same delete goes through"
    );
    fs::remove_file(&db).unwrap();
    let mut post = second_turn(r.fixture("PostToolUse-bash"));
    post["tool_input"]["command"] = json!("rm data/app.db");
    post["tool_use_id"] = json!("call_rm_2");
    r.hook_ok("PostToolUse", &post);
    let last_stop = r.hook_ok("Stop", &second_turn(r.fixture("Stop"))).out;

    // the engine's own record, as in D07: the save point taken while the files were still there has no copy of the
    // database ("its database was busy"), the one taken at the retried delete has it, and `back` returns its 5 rows
    let (code, shown) = engine_cli(&r, &["show", "2", "--json"]);
    assert_eq!(code, 0, "{shown}");
    let shown: Value = serde_json::from_str(&shown).unwrap();
    let pres: Vec<u64> = shown["points"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["kind"].as_str().is_some_and(|k| k.starts_with("pre_")))
        .filter_map(|p| p["id"].as_u64())
        .collect();
    assert!(pres.len() >= 2, "{shown}");
    let (early, late) = (pres[0].to_string(), pres[pres.len() - 1].to_string());
    let told: Value = serde_json::from_str(&last_stop).unwrap_or(Value::Null);
    assert!(
        blocked(&told).is_some_and(|w| w.contains(&format!("back --id {late} --with-data"))),
        "the Stop names the save point that has the copy: {last_stop}"
    );
    let (_, text) = engine_cli(&r, &["back", "--id", &early, "--with-data", "--dry-run"]);
    assert!(text.contains("its database was busy"), "{text}");
    let (_, text) = engine_cli(&r, &["back", "--id", &late, "--with-data", "--dry-run"]);
    assert!(!text.contains("busy"), "{text}");
    let (code, text) = engine_cli(&r, &["back", "--id", &late, "--with-data"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(rows_in(&db), "5", "{text}");
}

/// F4c item 10. A killed app leaves a `-wal` with rows only in it. The deny text names one read; it folds the log into
/// the file and removes `-wal` and `-shm`, the same delete then goes through, and the engine keeps a copy with the rows.
#[test]
fn real_engine_one_read_clears_the_files_a_killed_app_left_and_the_delete_then_goes_through_saved()
{
    for form in exit_forms() {
        exit_scenario("wal", form);
    }
}

/// F4c item 10. An app killed in the middle of a transaction leaves a hot `-journal`. The same read rolls back only the
/// unfinished transaction (the 5 committed rows stay), removes the journal, and the delete then goes through saved.
#[test]
fn real_engine_one_read_rolls_back_a_hot_journal_and_the_delete_then_goes_through_saved() {
    for form in exit_forms() {
        exit_scenario("hot", form);
    }
}

/// F4c item 10, the open limit. A `-journal` that a clean close left behind in PERSIST mode (its header is zeroed)
/// survives the read, so the delete stays denied; the text says to ask the user.
#[test]
fn real_engine_a_persist_journal_a_clean_close_left_survives_the_read_and_the_delete_stays_denied()
{
    for form in exit_forms() {
        exit_scenario("persist", form);
    }
}

/// F4c item 10, the open limit, as above for TRUNCATE mode (the journal file is empty).
#[test]
fn real_engine_a_truncate_journal_a_clean_close_left_survives_the_read_and_the_delete_stays_denied()
{
    for form in exit_forms() {
        exit_scenario("truncate", form);
    }
}

/// F4c item 10. While another program still has the database open (the app runs), the read leaves its log and the
/// retried delete stays denied: the guard is right to refuse it.
#[test]
fn real_engine_the_read_leaves_the_log_of_a_database_an_app_still_has_open_and_the_delete_stays_denied()
 {
    for form in exit_forms() {
        exit_scenario("app", form);
    }
}

// ---------------------------------------------------------------- F4c: no false alarms from the hands-free parts

/// The F4c corpus (pre-registered: no case is added, dropped or edited). The folder is a git repo with a busy `dev.db`
/// (its app runs, as in D07) and the files the commands name; the hook only looks at the commands, none is run. Rows:
/// id, command, the folder's .gitignore. The first list must go through, the second must be denied.
const IGNORED: &str = "dev.db";
const QUIET_ALLOW: [(&str, &str, &str); 14] = [
    ("A1", "find . -name .DS_Store -delete", IGNORED),
    ("A2", "find . -name '*.pyc' -delete", IGNORED),
    (
        "A3",
        "find . -type d -name __pycache__ -exec rm -rf {} +",
        IGNORED,
    ),
    ("A4", "git clean -fd", "dev.db*"),
    ("A5", "git clean -fdx -e 'dev.db*'", IGNORED),
    ("A6", "rm app.{log,txt}", IGNORED),
    ("A7", "cp -t backup/ dev.db", IGNORED),
    ("A8", "cat > reset.sh <<'EOF'\nrm dev.db\nEOF", IGNORED),
    ("A9", "mv src/old.ts src/new.ts", IGNORED),
    ("A10", "rm -rf node_modules", IGNORED),
    ("A11", "find src -name '*.orig' -delete", IGNORED),
    ("A12", "git rm --cached dev.db", IGNORED),
    ("A13", "cp dev.db dev.db.bak", IGNORED),
    ("A14", "rm -f dev.db.bak", IGNORED),
];
const QUIET_DENY: [(&str, &str, &str); 12] = [
    ("D1", "rm dev.db", IGNORED),
    ("D2", "rm -f ./dev.db*", IGNORED),
    ("D3", "find . -name '*.db' -delete", IGNORED),
    ("D4", "find . -delete", IGNORED),
    ("D5", "git clean -fdx", IGNORED),
    ("D6", "mv dev.db old.db", IGNORED),
    ("D7", "cp other.db dev.db", IGNORED),
    ("D8", ": > dev.db", IGNORED),
    ("D9", "sh -c 'rm dev.db'", IGNORED),
    ("D10", "cd data && rm ../dev.db", IGNORED),
    ("D11", "git clean -fd", IGNORED),
    ("D12", "git clean -fdx -e dev.db", IGNORED),
];

/// The corpus folder; its app holds `dev.db` open as long as the returned value lives.
fn quiet_folder(r: &Rig) -> BusyApp {
    assert!(
        r.command("git", &["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
    for d in ["data", "src", "backup", "node_modules/pkg"] {
        fs::create_dir_all(r.cwd.join(d)).unwrap();
    }
    for f in [
        "src/old.ts",
        "other.db",
        "dev.db.bak",
        "app.log",
        "app.txt",
        "node_modules/pkg/index.js",
    ] {
        fs::write(r.cwd.join(f), "x\n").unwrap();
    }
    let app = BusyApp::start(&r.cwd.join("dev.db"));
    assert!(fs::metadata(r.cwd.join("dev.db-wal")).unwrap().len() > 0);
    app
}

/// The rows of `list` whose verdict is not the wanted one (`want_deny`), each as `id command`.
fn quiet_wrong(name: &str, list: &[(&str, &str, &str)], want_deny: bool) -> Vec<String> {
    let r = Rig::real(name);
    let _app = quiet_folder(&r);
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    let mut wrong = Vec::new();
    for (id, cmd, ignored) in list {
        fs::write(r.cwd.join(".gitignore"), format!("{ignored}\n")).unwrap();
        // A6 only means something when a busy `app.db` is there for `app.*` to hit (the expanded braces must not)
        let app = (*id == "A6").then(|| BusyApp::start(&r.cwd.join("app.db")));
        let mut p = r.fixture("PreToolUse-bash");
        p["tool_input"]["command"] = json!(cmd);
        p["tool_use_id"] = json!(format!("call_{id}"));
        let out = r.hook_ok("PreToolUse", &p).out;
        let denied = serde_json::from_str::<Value>(&out)
            .is_ok_and(|v| v["hookSpecificOutput"]["permissionDecision"] == "deny");
        if denied != want_deny {
            wrong.push(format!("{id} {cmd}"));
        }
        if app.is_some() {
            drop(app);
            for s in ["", "-wal", "-shm"] {
                let _ = fs::remove_file(r.cwd.join(format!("app.db{s}")));
            }
        }
    }
    wrong
}

/// F4c: commands that cannot hurt the busy database are not stopped (a false alarm teaches the model to ask for approval).
#[test]
fn corpus_commands_that_cannot_hurt_a_busy_database_are_not_denied() {
    let wrong = quiet_wrong("quiet-allow", &QUIET_ALLOW, false);
    assert!(wrong.is_empty(), "{} false denies: {wrong:#?}", wrong.len());
}

/// F4c: and every way of losing it is still denied, `git clean` included (it deletes the -wal and -shm files too).
#[test]
fn corpus_commands_that_would_hurt_a_busy_database_are_denied() {
    let wrong = quiet_wrong("quiet-deny", &QUIET_DENY, true);
    assert!(
        wrong.is_empty(),
        "{} missed denies: {wrong:#?}",
        wrong.len()
    );
}

/// F4c Stop cases. S1 S3 S4 S5 with the fake engine saying what the turn did; S2 (a rename) with the real engine.
#[test]
fn corpus_the_stop_block_comes_only_for_what_is_deleted_or_lost() {
    let mut wrong: Vec<String> = Vec::new();
    let r = Rig::new("quiet-stop");
    started(&r);
    // S1: a migration changed dev.db: the user's line stays, the model is not stopped for it
    fs::write(r.cwd.join("dev.db"), b"x").unwrap();
    engine_turn(&r, 5, 30, &[31], &[("dev.db", "shell", "changed")]);
    engine_stop(&r, &[SAVED]);
    let v = stop_of(&r, 2);
    if blocked(&v).is_some() || !said(&v).contains("turn 5 changed dev.db") {
        wrong.push(format!("S1 {v}"));
    }
    // S3: a deleted file is still told
    engine_turn(&r, 6, 40, &[], &[("notes.md", "shell", "deleted")]);
    let v = stop_of(&r, 3);
    if !blocked(&v).is_some_and(|w| w.contains("This turn deleted notes.md")) {
        wrong.push(format!("S3 {v}"));
    }
    // S5: Muse then runs the turn on and stops again (stop_hook_active): that Stop asks the engine nothing
    let calls = r.calls().len();
    let mut again = r.fixture("Stop-active");
    again["turn_id"] = json!("00000000-0000-7000-8000-000000000003");
    let v = serde_json::from_str::<Value>(&r.hook_ok("Stop", &again).out).unwrap_or(Value::Null);
    if r.calls().len() != calls || blocked(&v).is_some() {
        wrong.push(format!("S5 {} engine calls: {v}", r.calls().len() - calls));
    }
    // S4 (D07): the database is deleted after its app stopped, or while it ran: both stay blocked
    engine_turn(&r, 7, 50, &[51], &[("dev.db", "shell", "deleted")]);
    let v = stop_of(&r, 4);
    if !blocked(&v).is_some_and(|w| w.contains("changed or deleted dev.db (database files)")) {
        wrong.push(format!("S4 deleted {v}"));
    }
    fs::remove_file(r.cwd.join("dev.db")).unwrap();
    engine_turn(&r, 8, 60, &[61], &[("dev.db", "shell", "not saved")]);
    let v = stop_of(&r, 5);
    if !blocked(&v).is_some_and(|w| w.contains("could not save dev.db")) {
        wrong.push(format!("S4 in use {v}"));
    }
    // S2: the real engine lists a rename as a deleted file and a new one
    let r = Rig::real("quiet-rename");
    fs::create_dir(r.cwd.join("src")).unwrap();
    fs::write(r.cwd.join("src/old.ts"), "export const a = 1;\n".repeat(12)).unwrap();
    started(&r);
    r.hook_ok("Stop", &r.fixture("Stop"));
    let v = real_turn(&r, 3, "mv src/old.ts src/new.ts", || {
        fs::rename(r.cwd.join("src/old.ts"), r.cwd.join("src/new.ts")).unwrap()
    });
    if blocked(&v).is_some_and(|w| w.contains("deleted")) {
        let (_, st) = engine_cli(&r, &["status", "--json"]);
        let n = serde_json::from_str::<Value>(&st)
            .map_or(0, |s| s["last_turn"]["n"].as_u64().unwrap_or(0));
        let (_, rows) = engine_cli(&r, &["show", &n.to_string(), "--json"]);
        let rows = serde_json::from_str::<Value>(&rows).map_or(rows, |s| s["files"].to_string());
        wrong.push(format!("S2 {v} rows {rows}"));
    }
    assert!(
        wrong.is_empty(),
        "{} wrong Stop cases: {wrong:#?}",
        wrong.len()
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

// ---------------------------------------------------------------- F4b: the gate is off until the user turns it on

/// The engine's promise gate is on in every folder it saves. The user chose hands-free (a broken check is told to the AI
/// before it says Done), so the first SessionStart in a saved folder switches it off there, once. After that the gate is the
/// user's: a `torpor gate on` typed in their own terminal stays on through every later session start, whatever its source.
/// The marker that keeps it so is a file in the store, which the engine's own guard refuses the model.
#[test]
fn real_engine_the_gate_is_off_in_a_new_folder_and_a_gate_on_of_the_users_stays_on() {
    let r = Rig::real("gate-default");
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    let state = gate_state(&r);
    assert!(state.starts_with("The gate is off"), "{state}");

    // the user turns it on in their own terminal (no CLAUDECODE there)
    let (code, text) = engine_cli(&r, &["gate", "on"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(gate_state(&r), "The gate is on.");
    for source in [
        "SessionStart",
        "SessionStart-resume",
        "SessionStart-clear",
        "SessionStart-compact",
        "SessionStart-fork",
    ] {
        r.hook_ok("SessionStart", &r.fixture(source));
        assert_eq!(gate_state(&r), "The gate is on.", "{source}");
    }

    // the model cannot take the marker away: the engine refuses a command on a file of its store
    let marker = fs::read_dir(r.store().join("muse/gate"))
        .unwrap()
        .next()
        .expect("one marker for the folder")
        .unwrap()
        .path();
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    let mut p = r.fixture("PreToolUse-bash");
    p["tool_input"]["command"] = json!(format!("rm {}", marker.display()));
    let run = r.hook_ok("PreToolUse", &p);
    assert!(
        run.out.contains(r#""permissionDecision":"deny""#),
        "{}",
        run.out
    );
    assert!(marker.exists());
}

/// A hook's environment may carry CLAUDECODE (Muse started from inside Claude Code): the engine refuses `gate off` then, as
/// it does for the model. Our `gate off` call does not carry it, so the default is still set.
#[test]
fn real_engine_the_gate_default_is_set_although_the_hooks_environment_has_claudecode() {
    let r = Rig::real("gate-default-cc");
    let payload = r.fixture("SessionStart").to_string();
    let run = r.hook_env("SessionStart", payload.as_bytes(), &[("CLAUDECODE", "1")]);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(r.problems().is_empty(), "{}", r.problems());
    let state = gate_state(&r);
    assert!(state.starts_with("The gate is off"), "{state}");
}

/// The gate default with a stand-in engine: `gate off` runs once per folder and only where the engine says it is saving. A
/// `gate off` that fails is one line in problems.log and not a word to the model; nothing is marked, so the next session
/// start tries again.
#[test]
fn the_gate_default_runs_once_per_folder_only_where_saving_and_a_failure_is_one_log_line() {
    let r = Rig::new("gate-default-fake");
    let off_calls = || r.calls().iter().filter(|c| *c == "gate off").count();
    let start = |source: &str| r.hook_ok("SessionStart", &r.fixture(source));

    // `torpor off` was typed in this folder: nothing is run
    r.set("out.status", r#"{"saving":false,"off":true}"#);
    start("SessionStart");
    assert_eq!(off_calls(), 0);

    // saving, but `gate off` fails: one log line, nothing for the model, no marker
    r.set("out.status", r#"{"saving":true}"#);
    r.set("code.gate", "1");
    let run = start("SessionStart");
    assert_eq!(off_calls(), 1);
    let log = r.problems();
    assert!(
        log.lines().count() == 1 && log.contains("gate default not set"),
        "{log}"
    );
    assert!(
        !run.out.contains("gate default") && !run.out.contains("not set"),
        "{}",
        run.out
    );

    // tried again at the next start; it works now, and is not run again in that folder
    fs::remove_file(r.fake.join("code.gate")).unwrap();
    start("SessionStart-resume");
    assert_eq!(off_calls(), 2);
    for source in ["SessionStart", "SessionStart-clear", "SessionStart-compact"] {
        start(source);
    }
    assert_eq!(off_calls(), 2, "once per folder");
    assert_eq!(r.problems().lines().count(), 1, "{}", r.problems());

    // another folder is its own
    let other = r.dir.join("other");
    fs::create_dir_all(&other).unwrap();
    let mut p = r.fixture("SessionStart");
    p["cwd"] = json!(other.to_str().unwrap());
    r.hook_ok("SessionStart", &p);
    assert_eq!(off_calls(), 3);
}

/// `gate off` first, the marker second: when the marker cannot be written the gate is off all the same, there is one line in
/// problems.log, and the next start runs `gate off` again (the other order would leave a marker with the gate still on, and
/// the user's choice would never apply). Once the marker can be written it is the last time.
#[test]
fn the_gate_default_is_set_before_it_is_marked_and_a_marker_that_cannot_be_written_is_one_log_line()
{
    let r = Rig::new("gate-default-mark");
    let off_calls = || r.calls().iter().filter(|c| *c == "gate off").count();
    r.set("out.status", r#"{"saving":true}"#);
    let muse = r.store().join("muse");
    fs::create_dir_all(&muse).unwrap();
    fs::write(
        muse.join("gate"),
        "a file where the marker folder should be",
    )
    .unwrap();

    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    assert_eq!(off_calls(), 1);
    let log = r.problems();
    assert!(
        log.lines().count() == 1 && log.contains("not remembered"),
        "{log}"
    );

    r.hook_ok("SessionStart", &r.fixture("SessionStart-resume"));
    assert_eq!(off_calls(), 2, "not marked: tried again");

    fs::remove_file(muse.join("gate")).unwrap();
    r.hook_ok("SessionStart", &r.fixture("SessionStart-clear"));
    assert_eq!(off_calls(), 3);
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    assert_eq!(off_calls(), 3, "marked now");
}

// ---------------------------------------------------------------- F4 items 2 and 3: the one Stop block of a turn

/// The engine's own Stop lines (gate.rs `fail_line`; the first is its save line), as it prints them: apart on lines.
const SAVED: &str =
    "Torpor: save point 31 · turn 5 changed 1 file (1 edit) · undo it: torpor undo 5";
const SHADOW_FAIL: &str = "Torpor: promise #1 (proof page answers) failed after this turn (3 of 3 runs; it passed on the version from the end of turn 4). It would have stopped the AI, but it is in shadow mode. torpor promise show 1";
const ADVISORY_FAIL: &str = "Torpor: promise #2 (login works) failed after this turn (3 of 3 runs; it passed before). The AI was not stopped. torpor promise show 2";

fn engine_stop(r: &Rig, lines: &[&str]) {
    r.set(
        "out.Stop",
        &json!({"systemMessage": lines.join("\n")}).to_string(),
    );
}

/// The fake engine's `status --json` and `show N --json` for turn `n`: it began at save point `start`, took the save
/// points `pre` before its commands and changed `rows` (path, class, change).
fn engine_turn(r: &Rig, n: u32, start: u32, pre: &[u32], rows: &[(&str, &str, &str)]) {
    r.set(
        "out.status",
        &json!({"saving": true, "last_turn": {"n": n}}).to_string(),
    );
    let files: Vec<Value> = rows
        .iter()
        .map(|(path, class, change)| json!({"path": path, "class": class, "change": change}))
        .collect();
    let mut points = vec![json!({"id": start, "kind": "start"})];
    points.extend(pre.iter().map(|id| json!({"id": id, "kind": "pre_bash"})));
    let show = json!({"turn": {"n": n, "start_point": start}, "files": files, "points": points});
    r.set(&format!("out.show.{n}"), &show.to_string());
}

/// The Stop of the turn whose id ends in `t` (2 is the fixture's own). A test that plays several turns gives each its own
/// id, as Muse does: one turn id is one turn, and its Stop after the one block says nothing more (F4c).
fn stop_run(r: &Rig, t: u32) -> Run {
    let mut p = r.fixture("Stop");
    p["turn_id"] = json!(format!("00000000-0000-7000-8000-{t:012}"));
    r.hook_ok("Stop", &p)
}

/// What the Stop of turn `t` printed, Null when nothing.
fn stop_of(r: &Rig, t: u32) -> Value {
    serde_json::from_str(&stop_run(r, t).out).unwrap_or(Value::Null)
}

/// The reason of a Stop block, None when the output is not one.
fn blocked(v: &Value) -> Option<&str> {
    (v["decision"] == "block").then(|| v["reason"].as_str().unwrap_or(""))
}

fn said(v: &Value) -> &str {
    v["systemMessage"].as_str().unwrap_or("")
}

fn started(r: &Rig) {
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    // the SessionStart's own `status --json` look (F4b, the gate default) is not what the tests after it count
    let _ = fs::remove_file(r.fake.join("calls.log"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
}

/// F4 item 2. A promise that failed after the turn (the engine's own fail line, shadow or advisory) is told to the model in
/// ONE Stop block per turn: the promise, its words, the turn that broke it (or how to find it), and what to do.
#[test]
fn a_check_that_broke_is_told_to_the_model_in_one_stop_block_per_turn() {
    let r = Rig::new("f4-broke");
    let e = r.engine.display().to_string();
    started(&r);
    engine_turn(&r, 5, 30, &[], &[("app.js", "edit", "changed")]);
    engine_stop(&r, &[SAVED, SHADOW_FAIL]);
    let first = stop_of(&r, 2);
    let why = blocked(&first).unwrap_or_else(|| panic!("no block: {first}"));
    let has = |why: &str, text: &str| assert!(why.contains(text), "{text}: {why}");
    for text in [
        "[Torpor] Before you finish:",
        "Promise #1 \"proof page answers\" (turn 5 broke it) failed after this turn.",
        "Fix the app, not the check, and do not special-case test values.",
        &format!("Run `{e} check 1` before you say it is fixed."),
        &format!("run `{e} promise dispute 1 \"<why>\"` and tell the user."),
    ] {
        has(why, text);
    }
    assert!(
        !why.contains("find-break") && !why.contains("deleted"),
        "{why}"
    );
    // the user's line is the engine's own, with the failure in it
    assert!(
        said(&first).starts_with("Torpor: save point 31")
            && said(&first).contains("promise #1 (proof page answers) failed after this turn"),
        "{first}"
    );
    assert_eq!(
        r.calls(),
        ["status --json", "show 5 --json"],
        "the two calls a Stop makes, no more"
    );
    // the next Stop of the same turn goes through, and the user's line still says it fails
    let again = stop_of(&r, 2);
    assert!(blocked(&again).is_none(), "{again}");
    assert!(said(&again).contains("failed after this turn"), "{again}");
    // the next turn is told again; this check passed before, so no one turn is named: the model is told how to find it
    engine_turn(&r, 6, 40, &[], &[]);
    engine_stop(&r, &[ADVISORY_FAIL]);
    let next = stop_of(&r, 3);
    let why = blocked(&next).unwrap_or_else(|| panic!("no block: {next}"));
    has(
        why,
        &format!(
            "Promise #2 \"login works\" (Torpor cannot name one turn that broke it: run `{e} find-break --promise 2`) failed after this turn."
        ),
    );
    has(
        why,
        &format!("Run `{e} check 2` before you say it is fixed."),
    );
    assert!(said(&next).contains("The AI was not stopped."), "{next}");
}

#[test]
fn several_broken_checks_are_one_block_and_a_turn_is_named_only_when_the_engines_record_says_so() {
    let r = Rig::new("f4-several");
    let e = r.engine.display().to_string();
    started(&r);
    engine_turn(&r, 5, 30, &[], &[]);
    let kept_now = "Torpor: promise #3 (kept this turn) failed after this turn (3 of 3 runs; it passed on the kept version (save point 31)). The AI was not stopped. torpor promise show 3";
    let kept_old = "Torpor: promise #4 (kept before) failed after this turn (3 of 3 runs; it passed on the kept version (save point 12)). The AI was not stopped. torpor promise show 4";
    let start_5 = "Torpor: promise #5 (same turn) failed after this turn (3 of 3 runs; it passed on the version from the start of turn 5). The AI was not stopped. torpor promise show 5";
    engine_stop(&r, &[SAVED, SHADOW_FAIL, kept_old, kept_now, start_5]);
    let v = stop_of(&r, 2);
    let why = blocked(&v).unwrap_or_else(|| panic!("no block: {v}"));
    let want = format!(
        "Promises #1 \"proof page answers\" (turn 5 broke it), #4 \"kept before\" (Torpor cannot name one turn that broke it: run `{e} find-break --promise 4`), #3 \"kept this turn\" (turn 5 broke it) and 1 more failed after this turn."
    );
    assert!(why.contains(&want), "{why}");
    assert!(
        why.contains(&format!("Run `{e} check ID` before you say it is fixed."))
            && why.contains("ID is the number after #"),
        "{why}"
    );
    assert_eq!(why.matches("[Torpor] Before you finish").count(), 1);
}

/// What is not a broken check, a deleted file or a database: no block, and nothing but the two engine calls of every Stop.
#[test]
fn a_turn_that_broke_nothing_and_risked_nothing_gets_no_block() {
    let r = Rig::new("f4-quiet");
    started(&r);
    // the engine's other promise lines (gate.rs): flaky, set aside, stopped for it already, checks cut short
    let others = [
        "Torpor: promise #2 (api answers) passed and failed on the same version, so it is flaky and advisory only. torpor promise show 2",
        "Torpor: promise #2 (api answers) fails on the version that passed before, so its check cannot be trusted. It is set aside. torpor promise show 2",
        "Torpor: promise #2 (api answers) still fails. The AI was stopped for it already and nothing changed since, so the turn is allowed to end. torpor promise show 2",
        "Torpor: promise checks were cut short at the last 3 turn ends. torpor status",
        "Torpor: promise #3 failed after turn 5. Run torpor check 3.",
        "Torpor: new promise #4 \"proof page answers\" (shadow), from your words at turn 1. Drop it: torpor promise drop 4",
    ];
    engine_turn(&r, 5, 30, &[], &[]);
    for (i, line) in others.iter().enumerate() {
        engine_stop(&r, &[SAVED, line]);
        let v = stop_of(&r, 10 + i as u32);
        assert!(blocked(&v).is_none(), "{line}: {v}");
        assert!(said(&v).starts_with("Torpor: save point 31"), "{v}");
    }
    assert_eq!(
        r.calls().len(),
        2 * others.len(),
        "a turn that changed no file costs the two calls of every Stop"
    );
    // a turn that only added or edited files, or whose changes are not the AI's: nothing to say
    engine_turn(
        &r,
        6,
        40,
        &[41],
        &[
            ("new.txt", "shell", "new"),
            ("app.js", "edit", "changed"),
            ("run.sh", "shell", "mode"),
            ("theirs.txt", "outside", "deleted"),
            ("shared.txt", "shared", "deleted"),
            ("theirs.db", "outside", "changed"),
            ("shared.db", "shared", "changed"),
            ("fresh.db", "shell", "new"),
        ],
    );
    engine_stop(&r, &[SAVED]);
    let v = stop_of(&r, 20);
    assert!(blocked(&v).is_none(), "{v}");
    // the engine blocked this turn itself: its block goes out as it is, with no second one and no calls of ours
    let before = r.calls().len();
    let theirs = json!({"decision": "block", "reason": "Torpor: promise #1 \"x\" fails now. Fix the app, not the check.", "systemMessage": format!("{SAVED}\n{SHADOW_FAIL}")});
    r.set("out.Stop", &theirs.to_string());
    let v = stop_of(&r, 21);
    let why = blocked(&v).unwrap_or_else(|| panic!("the engine's block is kept: {v}"));
    assert!(
        why.contains("fails now") && !why.contains("Before you finish"),
        "{why}"
    );
    assert_eq!(r.calls().len(), before, "{:?}", r.calls());
    // saving is off: the engine's last turn is an old one and nothing is said about it
    r.set(
        "out.status",
        &json!({"saving": false, "last_turn": {"n": 5}}).to_string(),
    );
    engine_stop(&r, &[SAVED, SHADOW_FAIL]);
    let v = stop_of(&r, 22);
    assert!(blocked(&v).is_none(), "{v}");
    assert_eq!(r.calls().last().map(String::as_str), Some("status --json"));
    // a subagent's Stop (its turn id is its session id) never reaches the engine
    let before = r.got("Stop").len();
    let mut child = r.fixture("Stop");
    child["turn_id"] = child["session_id"].clone();
    assert_eq!(r.hook_ok("Stop", &child).out, "");
    assert_eq!(r.got("Stop").len(), before);
}

/// F4 item 3. A turn that deleted a file that existed before it, or changed or deleted a database, gets the same one Stop
/// block (merged with a broken check's): say it in one plain sentence, and a database names the command that brings it back.
#[test]
fn a_turn_that_deleted_a_file_or_changed_a_database_is_told_in_the_same_one_block() {
    let r = Rig::new("f4-risky");
    let e = r.engine.display().to_string();
    started(&r);
    let has = |why: &str, text: &str| assert!(why.contains(text), "{text}: {why}");
    // a file that existed before the turn is gone; the new and the edited ones are not named
    engine_turn(
        &r,
        5,
        30,
        &[],
        &[
            ("old.txt", "shell", "deleted"),
            ("new.txt", "shell", "new"),
            ("app.js", "edit", "changed"),
        ],
    );
    engine_stop(&r, &[SAVED]);
    let v = stop_of(&r, 2);
    let why = blocked(&v).unwrap_or_else(|| panic!("no block: {v}"));
    has(
        why,
        "This turn deleted old.txt (files that existed before this turn). Tell the user in one plain sentence what was deleted or changed and that they can get it back by asking you. Do not undo anything unless they ask.",
    );
    for not in ["new.txt", "app.js", "back --id", "Promise", "database"] {
        assert!(!why.contains(not), "{not}: {why}");
    }
    assert!(said(&v).starts_with("Torpor: save point 31"), "{v}");
    assert!(blocked(&stop_of(&r, 2)).is_none(), "once per turn");
    // a broken check and a deleted file in one turn: one reason with both
    engine_turn(&r, 6, 40, &[], &[("a.txt", "shell", "deleted")]);
    engine_stop(&r, &[SAVED, SHADOW_FAIL]);
    let v = stop_of(&r, 3);
    let why = blocked(&v).unwrap_or_else(|| panic!("no block: {v}"));
    assert_eq!(
        why.matches("[Torpor] Before you finish").count(),
        1,
        "{why}"
    );
    let (check, gone) = (why.find("Promise #1"), why.find("This turn deleted a.txt"));
    assert!(check.is_some() && gone.is_some() && check < gone, "{why}");

    // a database that was only changed (a migration, a write) is not lost: L3's line tells the user, the model is not stopped
    engine_turn(&r, 7, 50, &[51], &[("data/app.db", "shell", "changed")]);
    engine_stop(&r, &[SAVED]);
    let before = r.calls().len();
    let v = stop_of(&r, 4);
    assert!(blocked(&v).is_none(), "{v}");
    assert_eq!(r.calls().len() - before, 2, "{:?}", r.calls());
    assert!(
        said(&v).starts_with("Torpor: turn 7 changed data/app.db. Get it back: stop the app, then torpor back --id 50 --with-data — Torpor: save point 31"),
        "L3's line stays first: {v}"
    );

    // a database that is gone: the block gives the command with the turn's first save point; no dry run was needed for it
    engine_turn(&r, 8, 50, &[51], &[("data/app.db", "shell", "deleted")]);
    engine_stop(&r, &[SAVED]);
    let before = r.calls().len();
    let v = stop_of(&r, 4);
    let why = blocked(&v).unwrap_or_else(|| panic!("no block: {v}"));
    has(
        why,
        "This turn changed or deleted data/app.db (database files). Tell the user in one plain sentence",
    );
    has(
        why,
        &format!("stop the app that uses it first, then run `{e} back --id 50 --with-data`"),
    );
    has(why, "Run it only if the user asks.");
    assert_eq!(r.calls().len() - before, 2, "{:?}", r.calls());
    assert!(
        said(&v).starts_with("Torpor: turn 8 changed data/app.db. Get it back: stop the app, then torpor back --id 50 --with-data — Torpor: save point 31"),
        "L3's line stays first: {v}"
    );

    // a database that was in use when the turn began and is gone now has no copy at the start of the turn: the latest save
    // point the turn took that has one is named (asked with a dry run, the latest first)
    engine_turn(
        &r,
        9,
        60,
        &[61, 62],
        &[("data/busy.db", "shell", "not saved")],
    );
    let busy = "Nothing needed changing.\nNot touched: data/busy.db (nothing to bring back, its database was busy).";
    r.set(
        "out.back.62",
        &format!("Would go back to save point 62 (before rm).\n{busy}"),
    );
    r.set("out.back.61", "Would go back to save point 61 (before rm).\nWould change 1 file: 1 brought back (data/busy.db).\nNothing was changed (dry run).");
    let v = stop_of(&r, 5);
    let why = blocked(&v).unwrap_or_else(|| panic!("no block: {v}"));
    has(why, &format!("run `{e} back --id 61 --with-data`"));
    assert!(!why.contains("could not save"), "{why}");
    assert!(
        said(&v).starts_with("Torpor: turn 9 changed data/busy.db. Get it back: stop the app, then torpor back --id 61 --with-data — "),
        "{v}"
    );
    let calls = r.calls();
    assert_eq!(
        calls[calls.len() - 2..],
        [
            "back --id 62 --with-data --dry-run",
            "back --id 61 --with-data --dry-run"
        ]
    );
    // none of its save points has it: no command is promised, the model is told to say so, and the line is honest
    engine_turn(&r, 10, 70, &[71], &[("data/busy.db", "shell", "not saved")]);
    let v = stop_of(&r, 6);
    let why = blocked(&v).unwrap_or_else(|| panic!("no block: {v}"));
    has(
        why,
        "Torpor could not save data/busy.db (it was in use), so it may not be able to bring it back: tell the user so.",
    );
    assert!(!why.contains("back --id"), "{why}");
    assert!(
        said(&v).starts_with(
            "Torpor: turn 10 changed data/busy.db. Torpor has no copy: it was in use. — "
        ),
        "{v}"
    );
    // only the turn with a command is kept for the folder's later sessions (L3 N1)
    let mut other = r.fixture("SessionStart");
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000dd");
    let ctx = context_of(&r.hook_ok("SessionStart", &other));
    assert!(
        ctx.contains("Turn 9 changed data/busy.db") && ctx.contains("back --id 61 --with-data"),
        "{ctx}"
    );
    assert!(!ctx.contains("Turn 10 changed"), "{ctx}");
    // a database that is still there and was in use at both ends: the line (L3) stays, the block has nothing to say
    fs::create_dir_all(r.cwd.join("data")).unwrap();
    fs::write(r.cwd.join("data/use.db"), b"x").unwrap();
    engine_turn(&r, 11, 80, &[], &[("data/use.db", "shell", "not saved")]);
    let v = stop_of(&r, 7);
    assert!(blocked(&v).is_none(), "{v}");
    assert!(
        said(&v).starts_with("Torpor: turn 11 changed data/use.db. Get it back: "),
        "{v}"
    );
}

/// Our block and the engine's share the two blocks Muse gets per turn (`allow_block`).
#[test]
fn our_block_counts_against_the_same_two_blocks_per_turn_as_the_engines() {
    let r = Rig::new("f4-cap");
    started(&r);
    engine_turn(&r, 5, 30, &[], &[]);
    let theirs = r#"{"decision":"block","reason":"prove X first"}"#;
    r.set("out.Stop", theirs);
    assert_eq!(blocked(&stop_of(&r, 2)), Some("prove X first"));
    engine_stop(&r, &[SAVED, SHADOW_FAIL]);
    let second = stop_of(&r, 2);
    assert!(
        blocked(&second).is_some_and(|w| w.contains("[Torpor] Before you finish")),
        "{second}"
    );
    assert!(
        blocked(&stop_of(&r, 2)).is_none(),
        "the third is let through"
    );
    // two engine blocks use the turn's share up: ours is dropped, and the turn ends
    r.set("out.Stop", theirs);
    assert!(blocked(&stop_of(&r, 3)).is_some() && blocked(&stop_of(&r, 3)).is_some());
    engine_stop(&r, &[SAVED, SHADOW_FAIL]);
    let v = stop_of(&r, 3);
    assert!(blocked(&v).is_none(), "{v}");
    assert!(said(&v).contains("failed after this turn"), "{v}");
}

/// One real-engine turn, the way Muse runs it: the prompt, one bash command (the hook sees it before it runs; `effect`
/// is what the command does to the files) and the Stop, whose output comes back (Null when it printed nothing).
fn real_turn(r: &Rig, t: u32, cmd: &str, effect: impl FnOnce()) -> Value {
    let at = |mut p: Value| {
        p["turn_id"] = json!(format!("00000000-0000-7000-8000-{t:012}"));
        p
    };
    r.hook_ok("UserPromptSubmit", &at(r.fixture("UserPromptSubmit")));
    let (mut pre, mut post) = (
        at(r.fixture("PreToolUse-bash")),
        at(r.fixture("PostToolUse-bash")),
    );
    for p in [&mut pre, &mut post] {
        p["tool_input"]["command"] = json!(cmd);
        p["tool_use_id"] = json!(format!("call_{t}"));
    }
    r.hook_ok("PreToolUse", &pre);
    effect();
    r.hook_ok("PostToolUse", &post);
    stop_of(r, t)
}

const PROOF_WORDS: &str = "the proof page must answer";
const PROOF_SPEC: &str = r#"{"v":1,"name":"proof page answers","why":"the proof page must answer","start":{"cmd":"python3 -m http.server {port} --bind 127.0.0.1","ready":"/","timeout_s":90},"env":{},"steps":[{"http":{"method":"GET","path":"/proof.html"},"expect":{"status":200,"body_contains":"proof"},"why":"the proof page answers"}],"unverifiable":false}"#;

/// The first turn of a real-engine rig (it leaves the save point without the page), then the page made by hand.
fn proof_rig(name: &str) -> Rig {
    let r = Rig::real(name);
    fs::write(r.cwd.join("base.txt"), "keep\n").unwrap();
    started(&r);
    r.hook_ok("Stop", &r.fixture("Stop"));
    fs::write(r.cwd.join("proof.html"), "<p>proof</p>\n").unwrap();
    r
}

fn set_page(r: &Rig, text: &str) -> impl FnOnce() {
    let (page, text) = (r.cwd.join("proof.html"), format!("<p>{text}</p>\n"));
    move || fs::write(page, text).unwrap()
}

/// F4 item 2 with the real engine: a check of the user's that passed, then fails after a turn. The block names the promise
/// and the turn the engine's own record says broke it (`find-break` agrees) and goes out once per turn. With the gate
/// switched off the engine does not stop the AI and this block is all the AI hears; with it on the engine blocks by itself
/// and there is no second block. The gate is off in a new folder (F4b) and the user's switch after that: the test turns it
/// on the way a person does.
#[test]
fn real_engine_a_broken_check_is_named_with_its_turn_once_and_the_gates_own_block_is_not_doubled() {
    let r = proof_rig("real-broke");
    let e = r.engine.display().to_string();
    let (code, text) = engine_cli_stdin(&r, &["keep", PROOF_WORDS, "--check", "-"], PROOF_SPEC);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("In shadow for now"), "{text}");

    // turn 2 edits the page and it still answers: nothing to say
    let ok = real_turn(&r, 3, "echo v2 > proof.html", set_page(&r, "proof v2"));
    assert!(
        blocked(&ok).is_none() && said(&ok).starts_with("Torpor: save point"),
        "{ok}"
    );
    // turn 3 breaks it
    let bad = real_turn(&r, 4, "echo nope > proof.html", set_page(&r, "nope"));
    let why = blocked(&bad).unwrap_or_else(|| panic!("no block: {bad}"));
    for text in [
        "Promise #1 \"proof page answers\" (turn 3 broke it) failed after this turn.",
        "Fix the app, not the check, and do not special-case test values.",
        &format!("Run `{e} check 1` before you say it is fixed."),
        &format!("run `{e} promise dispute 1 \"<why>\"` and tell the user."),
    ] {
        assert!(why.contains(text), "{text}: {why}");
    }
    assert!(
        said(&bad).contains("It would have stopped the AI, but it is in shadow mode."),
        "the user's line keeps the engine's words: {bad}"
    );
    let (_, found) = engine_cli(&r, &["find-break", "--promise", "1"]);
    assert!(
        found.contains("Turn 3 broke it"),
        "the engine agrees: {found}"
    );
    let again = stop_of(&r, 4);
    assert!(
        blocked(&again).is_none() && said(&again).contains("failed after this turn"),
        "{again}"
    );

    // turn 4 fixes it; the engine now stops the AI for this promise by itself
    let fixed = real_turn(
        &r,
        5,
        "echo fixed > proof.html",
        set_page(&r, "proof fixed"),
    );
    assert!(blocked(&fixed).is_none(), "{fixed}");
    assert!(
        said(&fixed).contains("it stops the AI from now on"),
        "{fixed}"
    );
    // the gate is off (F4b): the engine says it did not stop the AI, so the one block of ours is what the AI hears
    let state = gate_state(&r);
    assert!(state.starts_with("The gate is off"), "{state}");
    let off = real_turn(&r, 6, "echo nope > proof.html", set_page(&r, "nope"));
    let why = blocked(&off).unwrap_or_else(|| panic!("no block: {off}"));
    assert!(
        why.contains("Promise #1 \"proof page answers\" (turn 5 broke it) failed after this turn."),
        "{why}"
    );
    // the engine words its own block "... fails now ..." (asserted below with the gate on); with the gate off it sends none
    assert!(!why.contains("fails now"), "{why}");
    assert!(said(&off).contains("The AI was not stopped."), "{off}");
    assert!(blocked(&stop_of(&r, 6)).is_none());
    // fixed again, gate on, broken again: the engine's own block goes out as it is; ours is not added, and the next Stop
    // of the turn (the engine lets it end) is not blocked either
    let fixed = real_turn(
        &r,
        7,
        "echo fixed > proof.html",
        set_page(&r, "proof again"),
    );
    assert!(blocked(&fixed).is_none(), "{fixed}");
    assert_eq!(engine_cli(&r, &["gate", "on"]).0, 0); // the user's own word
    let gated = real_turn(&r, 8, "echo nope > proof.html", set_page(&r, "nope"));
    let why = blocked(&gated).unwrap_or_else(|| panic!("no block: {gated}"));
    assert!(
        why.contains("fails now") && !why.contains("Before you finish"),
        "{why}"
    );
    let after = stop_of(&r, 8);
    assert!(blocked(&after).is_none(), "{after}");
    assert!(
        said(&after).contains("The AI was stopped for it already"),
        "{after}"
    );
}

/// A check the AI itself added stays advisory; its failure is told in the same way, and the engine's line says it did not
/// stop the AI.
#[test]
fn real_engine_a_check_the_ai_added_is_told_too() {
    let r = proof_rig("real-broke-ai");
    let in_turn = |t: u32, mut p: Value| {
        p["turn_id"] = json!(format!("00000000-0000-7000-8000-{t:012}"));
        p
    };
    r.hook_ok(
        "UserPromptSubmit",
        &in_turn(3, r.fixture("UserPromptSubmit")),
    );
    let mut pre = in_turn(3, r.fixture("PreToolUse-bash"));
    pre["tool_input"]["command"] = json!(format!(
        "torpor keep \"{PROOF_WORDS}\" --check - < spec.json"
    ));
    r.hook_ok("PreToolUse", &pre);
    let (code, text) = engine_cli_stdin(&r, &["keep", PROOF_WORDS, "--check", "-"], PROOF_SPEC);
    assert_eq!(code, 0, "{text}");
    let first = stop_of(&r, 3);
    assert!(said(&first).contains("proposed by the AI"), "{first}");
    assert!(blocked(&first).is_none(), "{first}");

    let bad = real_turn(&r, 4, "echo nope > proof.html", set_page(&r, "nope"));
    let why = blocked(&bad).unwrap_or_else(|| panic!("no block: {bad}"));
    assert!(
        why.contains("Promise #1 \"proof page answers\" (")
            && why.contains("failed after this turn."),
        "{why}"
    );
    assert!(said(&bad).contains("The AI was not stopped."), "{bad}");
    // told once for the turn
    assert!(blocked(&stop_of(&r, 4)).is_none());
}

/// How many files the engine's record of turn `n` lists.
fn files_in(r: &Rig, n: u32) -> usize {
    let (code, text) = engine_cli(r, &["show", &n.to_string(), "--json"]);
    assert_eq!(code, 0, "{text}");
    serde_json::from_str::<Value>(&text).unwrap()["files"]
        .as_array()
        .map_or(usize::MAX, Vec::len)
}

/// F4 item 3 with the real engine: a file that existed before the turn and was deleted in it is told in a block; adding
/// or editing files is not; neither is a turn whose only changes are the AI's own `undo` or `back` of it.
#[test]
fn real_engine_a_deleted_file_is_told_but_the_ais_own_undo_or_back_is_not() {
    let r = Rig::real("real-risky");
    let e = r.engine.display().to_string();
    fs::write(r.cwd.join("old.txt"), "old\n").unwrap();
    fs::write(r.cwd.join("other.txt"), "other\n").unwrap();
    started(&r);
    r.hook_ok("Stop", &r.fixture("Stop"));

    let gone = real_turn(&r, 3, "rm old.txt", || {
        fs::remove_file(r.cwd.join("old.txt")).unwrap()
    });
    let why = blocked(&gone).unwrap_or_else(|| panic!("no block: {gone}"));
    assert!(
        why.contains("This turn deleted old.txt (files that existed before this turn). Tell the user in one plain sentence what was deleted or changed and that they can get it back by asking you. Do not undo anything unless they ask."),
        "{why}"
    );
    assert!(
        !why.contains("back --id") && !why.contains("Promise"),
        "{why}"
    );
    assert!(said(&gone).starts_with("Torpor: save point"), "{gone}");
    assert!(blocked(&stop_of(&r, 3)).is_none(), "once per turn");

    // the AI gets it back with `undo`: the turn's record has no changed file of its own
    let undone = real_turn(&r, 4, &format!("{e} undo 2"), || {
        let (code, text) = engine_cli(&r, &["undo", "2"]);
        assert_eq!(code, 0, "{text}");
    });
    assert!(r.cwd.join("old.txt").exists());
    assert!(blocked(&undone).is_none(), "{undone}");
    assert_eq!(
        files_in(&r, 3),
        0,
        "the engine's record of the turn that undid it"
    );

    // deleted again, and put back with `back` to the save point the turn began at
    let gone = real_turn(&r, 5, "rm old.txt", || {
        fs::remove_file(r.cwd.join("old.txt")).unwrap()
    });
    assert!(blocked(&gone).is_some(), "{gone}");
    let (_, shown) = engine_cli(&r, &["show", "4", "--json"]);
    let start = serde_json::from_str::<Value>(&shown).unwrap()["turn"]["start_point"].to_string();
    let back = real_turn(&r, 6, &format!("{e} back --id {start}"), || {
        let (code, text) = engine_cli(&r, &["back", "--id", &start]);
        assert_eq!(code, 0, "{text}");
    });
    assert!(r.cwd.join("old.txt").exists());
    assert!(blocked(&back).is_none(), "{back}");
    assert_eq!(
        files_in(&r, 5),
        0,
        "the engine's record of the turn that went back"
    );

    // a turn that only adds and edits files
    let added = real_turn(&r, 7, "echo more > other.txt; echo new > new.txt", || {
        fs::write(r.cwd.join("other.txt"), "more\n").unwrap();
        fs::write(r.cwd.join("new.txt"), "new\n").unwrap();
    });
    assert!(
        blocked(&added).is_none() && said(&added).starts_with("Torpor: save point"),
        "{added}"
    );
}

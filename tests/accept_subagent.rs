//! A model-spawned subagent's tool calls reach the engine as part of its parent's turn. Found by S3a-accept (e2e/RESULTS.md,
//! A4): src/hook/handle.rs dropped every child event, so a subagent could run the store-writing command the main session was
//! refused. Real `torpor-muse hook`, a fake engine that logs what it is given, a cleared environment as Muse gives it.
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const EXE: &str = env!("CARGO_BIN_EXE_torpor-muse");
const MAIN: &str = "00000000-0000-7000-8000-000000000001";
const OTHER: &str = "00000000-0000-7000-8000-000000000003";
const TURN: &str = "00000000-0000-7000-8000-000000000002";
/// A child's session id and turn id are the same (research/59 D10), and not the parent's.
const CHILD: &str = "00000000-0000-7000-8000-0000000000c1";
const DENY: &str = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"Torpor: this file belongs to Torpor's save points."}}"#;
const BLOCK: &str = r#"{"decision":"block","reason":"keep going"}"#;

struct Rig {
    dir: PathBuf,
    cwd: PathBuf,
}

impl Rig {
    fn new(name: &str) -> Rig {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("accept-subagent-{name}"));
        let _ = fs::remove_dir_all(&dir);
        let (cwd, fake) = (dir.join("proj"), dir.join("fake"));
        fs::create_dir_all(&cwd).unwrap();
        fs::create_dir_all(&fake).unwrap();
        let version = include_str!("../engine.lock")
            .lines()
            .find_map(|l| l.strip_prefix("version "))
            .expect("engine.lock has a version line")
            .trim();
        // `<E>` files next to the script steer it: out.<E> is what it prints; every payload is kept as in.<E>.<n>
        let script = format!(
            r#"#!/bin/sh
d=$(dirname "$0")
case "$1" in
version) echo 'torpor {version}'; exit 0 ;;
hook)
  c=$(cat "$d/count.$2" 2>/dev/null || echo 0); c=$((c + 1)); echo $c > "$d/count.$2"
  cat > "$d/in.$2.$c"
  [ -f "$d/out.$2" ] && cat "$d/out.$2"
  exit 0 ;;
esac
exit 64
"#
        );
        let p = fake.join("torpor");
        fs::write(&p, script).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        Rig { dir, cwd }
    }

    fn payload(&self, event: &str, sid: &str, turn: &str, extra: Value) -> Value {
        let mut p = json!({
            "session_id": sid, "turn_id": turn, "cwd": self.cwd, "hook_event_name": event,
            "model": "m", "model_provider": "meta", "permission_mode": "bypassPermissions",
            "transcript_path": null
        });
        p.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        p
    }

    fn main_event(&self, event: &str, extra: Value) -> String {
        self.hook(event, &self.payload(event, MAIN, TURN, extra))
    }

    /// A tool event of the subagent: its own session id as turn id, in the parent's folder.
    fn child_tool(&self, event: &str) -> String {
        let tool = json!({"tool_name": "bash", "tool_use_id": "tu1", "tool_input": {"command": "rm vault/x"}});
        self.hook(event, &self.payload(event, CHILD, CHILD, tool))
    }

    fn hook(&self, event: &str, p: &Value) -> String {
        let mut c = Command::new(EXE)
            .args(["hook", event])
            .current_dir(&self.cwd)
            .env_clear()
            .env("HOME", self.dir.join("home"))
            .env("PATH", "/usr/bin:/bin")
            .env("TORPOR_HOME", self.dir.join("store"))
            .env("TORPOR_BIN", self.dir.join("fake/torpor"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        c.stdin
            .take()
            .unwrap()
            .write_all(p.to_string().as_bytes())
            .unwrap();
        let o = c.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(0), "{event}");
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    fn steer(&self, event: &str, out: &str) {
        let f = self.dir.join(format!("fake/out.{event}"));
        if out.is_empty() {
            let _ = fs::remove_file(f);
        } else {
            fs::write(f, out).unwrap();
        }
    }

    /// The payloads the engine was given for `event`, oldest first.
    fn got(&self, event: &str) -> Vec<Value> {
        (1..)
            .map(|n| self.dir.join(format!("fake/in.{event}.{n}")))
            .take_while(|f| f.exists())
            .map(|f| serde_json::from_slice(&fs::read(f).unwrap()).unwrap())
            .collect()
    }
}

#[test]
fn a_subagents_tool_calls_go_to_the_engine_under_its_parents_turn() {
    let r = Rig::new("forwarded");
    r.main_event("UserPromptSubmit", json!({"prompt": "go"}));
    r.steer("PreToolUse", DENY);
    let out = r.child_tool("PreToolUse");
    assert!(
        out.contains(r#""permissionDecision":"deny""#),
        "the engine's answer must reach Muse for the subagent's call: {out}"
    );
    r.steer("PreToolUse", "");
    r.child_tool("PostToolUse");
    r.child_tool("PostToolUseFailure");
    for event in ["PreToolUse", "PostToolUse", "PostToolUseFailure"] {
        let got = r.got(event);
        assert_eq!(got.len(), 1, "{event}");
        let p = &got[0];
        assert_eq!(p["session_id"], MAIN, "{event}: the parent's session");
        assert_eq!(p["turn_id"], TURN, "{event}");
        assert_eq!(p["prompt_id"], TURN, "{event}: the parent's turn");
        assert_eq!(p["agent_id"], CHILD, "{event}: who really made the call");
        assert_eq!(p["tool_name"], "Bash", "{event}: reshaped like a main call");
        assert_eq!(p["tool_input"]["command"], "rm vault/x", "{event}");
    }
    // doctor's turn and pair counts see the child as a child, not as a second main session
    let beats = fs::read_to_string(r.dir.join(format!("store/muse/beats/{CHILD}.jsonl"))).unwrap();
    assert!(
        beats.contains(r#""m":false"#) && !beats.contains(r#""m":true"#),
        "{beats}"
    );
}

#[test]
fn a_subagents_call_with_no_open_turn_in_its_folder_is_not_sent() {
    let r = Rig::new("no-turn");
    r.child_tool("PreToolUse");
    assert!(r.got("PreToolUse").is_empty(), "nothing is open yet");
    r.main_event("UserPromptSubmit", json!({"prompt": "go"}));
    let mut other = r.payload("PreToolUse", CHILD, CHILD, json!({"tool_name": "bash"}));
    other["cwd"] = json!(r.dir.join("elsewhere"));
    r.hook("PreToolUse", &other);
    assert!(r.got("PreToolUse").is_empty(), "another folder");
}

#[test]
fn only_a_subagents_tool_events_are_sent_everything_else_it_does_stays_heartbeat_only() {
    let r = Rig::new("lifecycle");
    r.main_event("UserPromptSubmit", json!({"prompt": "go"}));
    for event in ["SubagentStart", "SubagentStop", "UserPromptSubmit", "Stop"] {
        let extra = json!({"child_session_id": CHILD, "subagent_id": "sub1"});
        r.hook(event, &r.payload(event, CHILD, CHILD, extra));
    }
    // the one UserPromptSubmit is the parent's; the child's Stop did not close the parent's turn either
    assert_eq!(r.got("UserPromptSubmit").len(), 1);
    assert!(r.got("Stop").is_empty() && r.got("SubagentStart").is_empty());
    r.child_tool("PreToolUse");
    assert_eq!(r.got("PreToolUse").len(), 1, "the turn is still open");
}

#[test]
fn a_subagent_is_guarded_until_the_turn_really_ends() {
    let r = Rig::new("ends");
    r.main_event("UserPromptSubmit", json!({"prompt": "go"}));
    // a Stop that blocks: Muse runs the same turn on, so the subagent's calls still belong to it
    r.steer("Stop", BLOCK);
    r.main_event("Stop", json!({"stop_hook_active": false}));
    r.child_tool("PreToolUse");
    assert_eq!(r.got("PreToolUse").len(), 1, "after a blocking Stop");
    // a Stop that lets it end
    r.steer("Stop", "");
    r.main_event("Stop", json!({"stop_hook_active": true}));
    r.child_tool("PreToolUse");
    assert_eq!(r.got("PreToolUse").len(), 1, "after the Stop that ended it");
    // the next prompt opens it again; StopFailure and SessionEnd close it
    for end in ["StopFailure", "SessionEnd"] {
        r.main_event("UserPromptSubmit", json!({"prompt": "again"}));
        r.child_tool("PreToolUse");
        let n = r.got("PreToolUse").len();
        r.main_event(end, json!({}));
        r.child_tool("PreToolUse");
        assert_eq!(r.got("PreToolUse").len(), n, "after {end}");
    }
}

#[test]
fn another_sessions_end_in_the_same_folder_does_not_close_the_open_turn() {
    let r = Rig::new("two-sessions");
    r.main_event("UserPromptSubmit", json!({"prompt": "go"}));
    r.hook(
        "UserPromptSubmit",
        &r.payload(
            "UserPromptSubmit",
            OTHER,
            "t-other",
            json!({"prompt": "go"}),
        ),
    );
    // the newest turn in the folder is the other session's: the subagent is filed there
    r.child_tool("PreToolUse");
    assert_eq!(r.got("PreToolUse")[0]["session_id"], OTHER);
    // the first session ends: the other's turn stays open
    r.main_event("SessionEnd", json!({}));
    r.child_tool("PreToolUse");
    assert_eq!(r.got("PreToolUse").len(), 2);
}

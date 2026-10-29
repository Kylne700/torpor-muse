//! What the hook tests share: the real binary in a cleared environment, a fake engine, the check of Muse's output.
#![allow(dead_code)]

use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const EXE: &str = env!("CARGO_BIN_EXE_torpor-muse");
pub const SID: &str = "00000000-0000-7000-8000-000000000001";

pub fn pinned() -> &'static str {
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
pub fn write_fake(dir: &Path, version: &str) -> PathBuf {
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

pub struct Run {
    pub code: i32,
    pub out: String,
    pub err: String,
    pub took: Duration,
}

pub struct Rig {
    pub dir: PathBuf,
    pub cwd: PathBuf,
    pub fake: PathBuf,
    pub engine: PathBuf,
    /// Tell each session's first prompt that its "check-writer is off" note was heard already (`hear_no_key`).
    pub quiet: bool,
}

impl Rig {
    /// Fake engine of the pinned version. The "check-writer is off" note (there is no key file here) is marked as
    /// heard in every session before its prompt, so it does not turn up in every test; `unseeded` is the rig that
    /// still gets it.
    pub fn new(name: &str) -> Rig {
        let mut r = Rig::unseeded(name);
        r.quiet = true;
        r
    }

    pub fn unseeded(name: &str) -> Rig {
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
            quiet: false,
        }
    }

    /// Writes `no-key` into the told kinds of the session in `payload` (the real file format; `serde(default)` fills
    /// in the rest), the way a session that heard the note would have it.
    pub fn hear_no_key(&self, payload: &[u8]) {
        let sid = serde_json::from_slice::<Value>(payload)
            .ok()
            .and_then(|v| v["session_id"].as_str().map(str::to_owned));
        let Some(sid) = sid.filter(|s| s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        else {
            return;
        };
        let dir = self.store().join("muse/session");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .unwrap();
        let file = dir.join(format!("{sid}.json"));
        let mut s: Value = fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| json!({"seq": 0, "told": false, "blocks": []}));
        let mut heard = s["writer_told"].as_array().cloned().unwrap_or_default();
        if !heard.contains(&json!("no-key")) {
            heard.push(json!("no-key"));
            s["writer_told"] = Value::Array(heard);
            fs::write(&file, s.to_string()).unwrap();
        }
    }

    /// The pinned real engine from TORPOR_BIN (cargo runs tests from the package root, so a relative path works).
    pub fn real(name: &str) -> Rig {
        let mut r = Rig::new(name);
        let bin = std::env::var_os("TORPOR_BIN")
            .filter(|b| !b.is_empty())
            .expect(
                "set TORPOR_BIN to the pinned engine (engine.lock) to run the real-engine tests",
            );
        r.engine = fs::canonicalize(bin).expect("TORPOR_BIN does not exist");
        r
    }

    pub fn store(&self) -> PathBuf {
        self.dir.join("store")
    }

    /// What Muse gives a hook: `$SHELL -c` with a cleared environment, in the project folder. TORPOR_* is what the
    /// installed command bakes in.
    pub fn command(&self, exe: &str, args: &[&str]) -> Command {
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

    pub fn hook_bytes(&self, event: &str, stdin: &[u8]) -> Run {
        self.hook_env(event, stdin, &[])
    }

    /// `hook_bytes` with more variables in the hook's environment.
    pub fn hook_env(&self, event: &str, stdin: &[u8], env: &[(&str, &str)]) -> Run {
        if self.quiet && event == "UserPromptSubmit" {
            self.hear_no_key(stdin);
        }
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

    pub fn hook(&self, event: &str, payload: &Value) -> Run {
        self.hook_bytes(event, payload.to_string().as_bytes())
    }

    /// `hook`, and the hook must exit 0 with stdout Muse accepts.
    pub fn hook_ok(&self, event: &str, payload: &Value) -> Run {
        let run = self.hook(event, payload);
        assert_eq!(run.code, 0, "{event}: {}", run.err);
        assert_muse_output(event, &run.out);
        run
    }

    pub fn fixture(&self, name: &str) -> Value {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/core")
            .join(format!("{name}.json"));
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        serde_json::from_str(&text.replace("<CWD>", self.cwd.to_str().unwrap())).unwrap()
    }

    /// The payloads the fake engine got for `hook <event>`, oldest first.
    pub fn got(&self, event: &str) -> Vec<Value> {
        (1..)
            .map_while(|n| fs::read_to_string(self.fake.join(format!("in.{event}.{n}"))).ok())
            .map(|t| serde_json::from_str(&t).expect("the engine got JSON"))
            .collect()
    }

    pub fn set(&self, file: &str, text: &str) {
        fs::write(self.fake.join(file), text).unwrap();
    }

    pub fn problems(&self) -> String {
        fs::read_to_string(self.store().join("muse/problems.log")).unwrap_or_default()
    }

    /// The engine calls the hook made besides `hook` and `version` (the arguments of each, one per entry).
    pub fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.fake.join("calls.log"))
            .unwrap_or_default()
            .lines()
            .map(String::from)
            .collect()
    }

    pub fn version_calls(&self) -> usize {
        fs::read_to_string(self.fake.join("version.calls")).map_or(0, |t| t.lines().count())
    }
}

/// Muse's closed output schema (research/59 D5) as seen from outside: nothing on stdout, or one JSON object that
/// starts with `{`, uses only the closed camelCase keys, stays under 12 KiB, and follows the per-event rules.
pub fn assert_muse_output(event: &str, out: &str) {
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

pub fn note_parts(out: &str) -> (String, String) {
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

/// The rig's project folder in the check-writer's state, where its `result.json` and `lock` live (named by the
/// FNV-1a hash of the folder's real path, as `suggest::state` does).
pub fn writer_ws(r: &Rig) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
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
    ws
}

/// One scripted turn through the hooks, like Muse does it: a file is created with write_file and deleted by a bash
/// command in the same turn. `pre_bash` = whether the hook sees that bash command before it runs.
pub fn turn_with_a_created_and_deleted_file(r: &Rig, pre_bash: bool) {
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

pub fn engine_cli(r: &Rig, args: &[&str]) -> (i32, String) {
    engine_cli_stdin(r, args, "")
}

/// The engine's own command line, run the way a person runs it (nothing from Claude Code in the environment).
pub fn engine_cli_stdin(r: &Rig, args: &[&str], stdin: &str) -> (i32, String) {
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
pub fn gate_state(r: &Rig) -> String {
    let (code, text) = engine_cli(r, &["gate"]);
    assert_eq!(code, 0, "{text}");
    text.lines().next().unwrap_or("").to_owned()
}

/// The additional context of a hook's output, "" when it printed nothing.
pub fn context_of(run: &Run) -> String {
    if run.out.is_empty() {
        String::new()
    } else {
        note_parts(&run.out).1
    }
}

/// The drill's D07 setup: an app that holds a database open in WAL mode with 300 rows in its log (made with python's
/// sqlite3 module). A line on its stdin closes it, which writes the log into the database file and removes the `-wal`
/// and `-shm` files, as an app that stops cleanly does. Killed when the test ends or fails.
pub struct BusyApp(pub std::process::Child);

impl BusyApp {
    pub fn start(db: &Path) -> BusyApp {
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

    pub fn stop(mut self) {
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

/// The engine's own Stop lines (gate.rs `fail_line`; the first is its save line), as it prints them: apart on lines.
pub const SAVED: &str =
    "Torpor: save point 31 · turn 5 changed 1 file (1 edit) · undo it: torpor undo 5";

pub fn engine_stop(r: &Rig, lines: &[&str]) {
    r.set(
        "out.Stop",
        &json!({"systemMessage": lines.join("\n")}).to_string(),
    );
}

/// The fake engine's `status --json` and `show N --json` for turn `n`: it began at save point `start`, took the save
/// points `pre` before its commands and changed `rows` (path, class, change).
pub fn engine_turn(r: &Rig, n: u32, start: u32, pre: &[u32], rows: &[(&str, &str, &str)]) {
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
/// id, as Muse does: one turn id is one turn, and its Stop after the first block says nothing more unless a check still
/// fails (F4c, SN2).
pub fn stop_run(r: &Rig, t: u32) -> Run {
    let mut p = r.fixture("Stop");
    p["turn_id"] = json!(format!("00000000-0000-7000-8000-{t:012}"));
    r.hook_ok("Stop", &p)
}

/// What the Stop of turn `t` printed, Null when nothing.
pub fn stop_of(r: &Rig, t: u32) -> Value {
    serde_json::from_str(&stop_run(r, t).out).unwrap_or(Value::Null)
}

/// The reason of a Stop block, None when the output is not one.
pub fn blocked(v: &Value) -> Option<&str> {
    (v["decision"] == "block").then(|| v["reason"].as_str().unwrap_or(""))
}

pub fn said(v: &Value) -> &str {
    v["systemMessage"].as_str().unwrap_or("")
}

pub fn started(r: &Rig) {
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    // the SessionStart's own `status --json` look (F4b, the gate default) is not what the tests after it count
    let _ = fs::remove_file(r.fake.join("calls.log"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
}

/// One real-engine turn, the way Muse runs it: the prompt, one bash command (the hook sees it before it runs; `effect`
/// is what the command does to the files) and the Stop, whose output comes back (Null when it printed nothing).
pub fn real_turn(r: &Rig, t: u32, cmd: &str, effect: impl FnOnce()) -> Value {
    serde_json::from_str(&real_turn_run(r, t, cmd, effect).out).unwrap_or(Value::Null)
}

/// `real_turn`, with the whole run of its Stop (the stderr too).
pub fn real_turn_run(r: &Rig, t: u32, cmd: &str, effect: impl FnOnce()) -> Run {
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
    stop_run(r, t)
}

//! `torpor-muse doctor` (src/doctor.rs) against made-up worlds: a settings file, a vault with heartbeats, Muse logs and
//! fake hooks. doctor must never delete or kill anything but its own probe: the tests check that as well.
#[path = "../src/doctor.rs"]
mod doctor;
#[path = "../src/liveness.rs"]
mod liveness;
#[path = "../src/transcript.rs"]
mod transcript;

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

use common::{LogBuilder, Tmp, append, fixture, sid_of};
use doctor::{Ctx, Level, Report};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Every world starts here (a date in the past, so that "now" is whatever the test says it is).
const T0: u64 = 1_700_000_000_000;

// ---------------------------------------------------------------- fake hooks

/// Stands in for torpor-muse. What it does is told by files next to the (symlinked) path it was started by, so one
/// script serves every test and is written only once (a script written while other tests start processes can fail
/// with "text file busy").
const FAKE_EXE: &str = r#"#!/bin/sh
d=$(dirname "$0")
cat > "$d/stdin.txt"
env > "$d/env.txt"
pwd > "$d/cwd.txt"
[ -f "$d/stdout.txt" ] && cat "$d/stdout.txt"
[ -f "$d/bg" ] && { sleep 5 & echo $! > "$d/sleeper.pid"; }
[ -f "$d/hang" ] && { sleep 60 & echo $! > "$d/sleeper.pid"; wait; }
exit "$(cat "$d/exit" 2>/dev/null || echo 0)"
"#;

/// Stands in for the engine's `torpor gate`: prints `gate.say`, exits with `gate.exit`, and keeps what it was started with
/// and where (files next to the symlinked path, like the fake torpor-muse).
const FAKE_ENGINE: &str = r#"#!/bin/sh
d=$(dirname "$0")
echo "$*" > "$d/gate.args"
echo "$TORPOR_HOME" > "$d/gate.home"
pwd > "$d/gate.cwd"
cat "$d/gate.say" 2>/dev/null
exit "$(cat "$d/gate.exit" 2>/dev/null || echo 0)"
"#;

/// Stands in for a login shell whose start-up file talks (zsh reads ~/.zshenv for every `-c`; there is no zsh here).
const FAKE_SHELL: &str = r#"#!/bin/sh
[ -f "$HOME/.fakerc" ] && . "$HOME/.fakerc"
exec /bin/sh "$@"
"#;

fn script(cell: &'static OnceLock<PathBuf>, name: &str, body: &str) -> PathBuf {
    cell.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shim-doctor");
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        if fs::read_to_string(&p).ok().as_deref() != Some(body) {
            fs::write(&p, body).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    })
    .clone()
}

fn fake_exe() -> PathBuf {
    static C: OnceLock<PathBuf> = OnceLock::new();
    script(&C, "fake-torpor-muse", FAKE_EXE)
}

fn fake_engine() -> PathBuf {
    static C: OnceLock<PathBuf> = OnceLock::new();
    script(&C, "fake-torpor-engine", FAKE_ENGINE)
}

fn fake_shell() -> PathBuf {
    static C: OnceLock<PathBuf> = OnceLock::new();
    script(&C, "fake-shell", FAKE_SHELL)
}

// ---------------------------------------------------------------- a world to examine

fn ms_of(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

fn set_mtime(p: &Path, ms: u64) {
    fs::File::open(p)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_millis(ms))
        .unwrap();
}

struct World {
    t: Tmp,
    home: PathBuf,
    vault: PathBuf,
    settings: PathBuf,
    bin: PathBuf,
    tmp: PathBuf,
}

impl World {
    /// Settings with the three hooks the gate needs, written as `init` writes them; a fake torpor-muse and engine.
    fn new(name: &str) -> World {
        let t = Tmp::new(name);
        let (home, vault, bin, tmp) = (
            t.join("home"),
            t.join("vault"),
            t.join("bin"),
            t.join("tmp"),
        );
        for d in [&home, &bin, &tmp, &t.join("proj")] {
            fs::create_dir_all(d).unwrap();
        }
        std::os::unix::fs::symlink(fake_exe(), bin.join("torpor-muse")).unwrap();
        fs::write(bin.join("torpor"), "engine").unwrap();
        let w = World {
            settings: home.join(".config/muse/settings.json"),
            t,
            home,
            vault,
            bin,
            tmp,
        };
        w.write_json(&w.settings_value(&[
            ("SessionStart", 10),
            ("UserPromptSubmit", 10),
            ("Stop", 35),
        ]));
        w
    }

    fn exe(&self) -> PathBuf {
        self.bin.join("torpor-muse")
    }

    fn engine(&self) -> PathBuf {
        self.bin.join("torpor")
    }

    fn command(&self, event: &str) -> String {
        format!(
            "env TORPOR_HOME={} TORPOR_BIN={} {} hook {event}",
            doctor::quote(&self.vault.to_string_lossy()),
            doctor::quote(&self.engine().to_string_lossy()),
            doctor::quote(&self.exe().to_string_lossy())
        )
    }

    fn settings_value(&self, hooks: &[(&str, u64)]) -> Value {
        let h: serde_json::Map<String, Value> = hooks
            .iter()
            .map(|(e, t)| {
                let entry = json!([{"hooks": [{"type": "command", "command": self.command(e), "timeout": t}]}]);
                (e.to_string(), entry)
            })
            .collect();
        json!({"schema_version": 1, "hooks": h})
    }

    fn write_json(&self, v: &Value) {
        fs::create_dir_all(self.settings.parent().unwrap()).unwrap();
        fs::write(&self.settings, v.to_string()).unwrap();
        set_mtime(&self.settings, T0);
    }

    /// When the settings were written (heartbeat times in the tests are relative to it).
    fn t0(&self) -> u64 {
        fs::metadata(&self.settings)
            .and_then(|m| m.modified())
            .map_or(T0, ms_of)
    }

    fn sessions(&self) -> PathBuf {
        self.home.join(".local/share/muse/sessions")
    }

    fn ctx(&self) -> Ctx {
        Ctx {
            settings: self.settings.clone(),
            home: self.home.clone(),
            logs: self.sessions(),
            hook_logs: self.sessions(),
            xdg_data: None,
            env_vault: Some(self.vault.clone()),
            shell: Some("/bin/sh".to_string()),
            temps: vec![self.tmp.clone()],
            cwd: self.t.join("proj"),
            now_ms: self.t0() + 60_000,
            probe_timeout: Duration::from_secs(20),
        }
    }

    fn examine(&self) -> Report {
        doctor::examine(&self.ctx())
    }

    fn beat(&self, sid: &str, main: bool, event: &str, dt: u64) {
        liveness::beat_in(&self.vault, event, sid, main, self.t0() + dt).unwrap();
    }

    fn put(&self, text: &str) -> PathBuf {
        common::put_log(&self.sessions(), &sid_of(text), text)
    }

    /// What the fake torpor-muse was started with.
    fn seen(&self, file: &str) -> String {
        fs::read_to_string(self.bin.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))
    }
}

/// A world where the gate is proven: a finished turn, and a log of the tested Muse version.
fn healthy(name: &str) -> World {
    let w = World::new(name);
    for (e, dt) in [
        ("SessionStart", 500),
        ("UserPromptSubmit", 1000),
        ("Stop", 2000),
    ] {
        w.beat("s-main", true, e, dt);
    }
    let mut b = LogBuilder::new("01a179b0-0000-7000-8000-000000000001");
    b.metadata("1.4.13");
    w.put(&b.text());
    w
}

fn edited(name: &str, f: impl FnOnce(&World, &mut Value)) -> Report {
    let w = World::new(name);
    let mut v = w.settings_value(&[("SessionStart", 10), ("UserPromptSubmit", 10), ("Stop", 35)]);
    f(&w, &mut v);
    w.write_json(&v);
    w.examine()
}

#[track_caller]
fn must(r: &Report, l: Level, needle: &str) {
    assert!(
        r.has(l, needle),
        "expected {l:?} with {needle:?} in:\n{}",
        r.text()
    );
}

#[track_caller]
fn mustnt(r: &Report, l: Level, needle: &str) {
    assert!(
        !r.has(l, needle),
        "unexpected {l:?} with {needle:?} in:\n{}",
        r.text()
    );
}

/// Every Fix and Warn names the way out on a line of its own.
#[track_caller]
fn every_problem_says_how(r: &Report) {
    for (l, s) in &r.0 {
        if matches!(l, Level::Fix | Level::Warn) {
            assert!(s.contains("\n      do: "), "no way out given for: {s}");
        }
    }
}

fn tree(dir: &Path) -> Vec<(PathBuf, u64)> {
    let (mut out, mut stack) = (Vec::new(), vec![dir.to_path_buf()]);
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).into_iter().flatten().flatten() {
            let m = e.metadata().unwrap();
            if m.is_dir() {
                stack.push(e.path());
            }
            out.push((e.path(), m.len()));
        }
    }
    out.sort();
    out
}

/// `ps` still lists a zombie until its parent collects it; for these tests that process is gone.
fn alive(pid: u32) -> bool {
    let out = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let stat = String::from_utf8_lossy(&out.stdout);
    !stat.trim().is_empty() && !stat.trim().starts_with('Z')
}

fn wait_gone(pid: u32, secs: u64) -> bool {
    let t = Instant::now();
    while alive(pid) {
        if t.elapsed() > Duration::from_secs(secs) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

/// Ends a process one of these tests started itself (its pid is in a file the fake hook wrote).
fn end(pid: u32) {
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
}

// ---------------------------------------------------------------- the settings

#[test]
fn no_settings_file_is_a_fix() {
    let w = World::new("nosettings");
    fs::remove_file(&w.settings).unwrap();
    let r = w.examine();
    must(&r, Level::Fix, "no Muse settings file at");
    must(&r, Level::Fix, "do: torpor-muse init");
    every_problem_says_how(&r);
    assert_eq!(doctor::finish(&r), 1);
}

#[test]
fn settings_that_are_not_json_are_a_fix() {
    let w = World::new("brokenjson");
    fs::write(&w.settings, "{ not json").unwrap();
    let r = w.examine();
    must(&r, Level::Fix, "is not valid JSON");
    must(&r, Level::Fix, ".torpor-muse.bak");
    assert_eq!(r.fixes(), 1, "{}", r.text());
    assert_eq!(doctor::finish(&r), 1);
}

#[test]
fn settings_without_our_hooks_are_a_fix() {
    let w = World::new("nohooks");
    w.write_json(&json!({"schema_version": 1, "hooks": {"Stop": [{"hooks": [
        {"type": "command", "command": "echo hi", "timeout": 5}]}]}}));
    let r = w.examine();
    must(&r, Level::Fix, "the gate is not installed");
    assert_eq!(r.fixes(), 1, "{}", r.text());
}

#[test]
fn a_working_install_has_nothing_to_fix() {
    let w = healthy("healthy");
    let (vault_before, home_before) = (tree(&w.vault), tree(&w.home));
    let r = w.examine();
    println!("{}", r.text());
    assert_eq!(r.fixes(), 0, "{}", r.text());
    assert!(r.0.iter().all(|(l, _)| *l != Level::Warn), "{}", r.text());
    must(&r, Level::Ok, "gate live: Stop ran 58s ago");
    must(&r, Level::Ok, "(1 finished turn(s))");
    must(
        &r,
        Level::Ok,
        "Muse Code 1.4.13 (the version these hooks were tested with)",
    );
    must(
        &r,
        Level::Ok,
        &format!(
            "vault: {} (TORPOR_HOME in the generated hook commands)",
            w.vault.display()
        ),
    );
    must(&r, Level::Ok, "prints nothing");
    must(
        &r,
        Level::Info,
        "hook timeouts: SessionStart 10s, Stop 35s, UserPromptSubmit 10s",
    );
    must(&r, Level::Info, "last heartbeat per event, main sessions:");
    assert_eq!(doctor::finish(&r), 0);
    // doctor only looked: the probe ran in a folder of its own and nothing of the world changed
    assert_eq!(tree(&w.vault), vault_before);
    assert_eq!(tree(&w.home), home_before);
}

#[test]
fn an_async_hook_is_a_fix() {
    let r = edited("async", |_, v| {
        v["hooks"]["UserPromptSubmit"][0]["hooks"][0]["async"] = json!(true);
    });
    must(&r, Level::Fix, "the UserPromptSubmit hook is async");
    every_problem_says_how(&r);
}

#[test]
fn a_timeout_that_is_not_whole_seconds_is_a_fix() {
    let r = edited("fraction", |_, v| {
        v["hooks"]["Stop"][0]["hooks"][0]["timeout"] = json!(1.5);
    });
    must(
        &r,
        Level::Fix,
        "the Stop hook has a timeout that is not a whole number of seconds",
    );
}

#[test]
fn a_timeout_shorter_than_the_engine_needs_is_a_warning() {
    let r = edited("short", |_, v| {
        v["hooks"]["Stop"][0]["hooks"][0]["timeout"] = json!(2);
    });
    must(
        &r,
        Level::Warn,
        "the Stop timeout (2s) is shorter than the engine's own limit plus start-up (31s)",
    );
    every_problem_says_how(&r);
    let r = edited("enough", |_, v| {
        v["hooks"]["Stop"][0]["hooks"][0]["timeout"] = json!(31);
    });
    mustnt(&r, Level::Warn, "shorter than the engine's own limit");
}

#[test]
fn a_missing_stop_hook_is_a_fix() {
    let r = edited("nostop", |_, v| {
        v["hooks"].as_object_mut().unwrap().remove("Stop");
    });
    must(&r, Level::Fix, "no torpor-muse hook for Stop");
}

#[test]
fn hooks_with_different_vaults_are_a_fix() {
    let r = edited("twovaults", |_, v| {
        let c = v["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .replace("TORPOR_HOME=", "TORPOR_HOME=/elsewhere");
        v["hooks"]["Stop"][0]["hooks"][0]["command"] = json!(c);
    });
    must(&r, Level::Fix, "the hook commands use different vaults");
}

#[test]
fn commands_without_a_vault_use_the_default_and_say_so() {
    let r = edited("novault", |_, v| {
        for e in ["SessionStart", "UserPromptSubmit", "Stop"] {
            let c = v["hooks"][e][0]["hooks"][0]["command"].as_str().unwrap();
            let c: Vec<&str> = c
                .split(' ')
                .filter(|w| !w.starts_with("TORPOR_HOME="))
                .collect();
            v["hooks"][e][0]["hooks"][0]["command"] = json!(c.join(" "));
        }
    });
    must(&r, Level::Warn, "carry no TORPOR_HOME, so they use");
    every_problem_says_how(&r);
}

#[test]
fn a_missing_engine_binary_is_a_fix() {
    let r = edited("noengine", |w, _| fs::remove_file(w.engine()).unwrap());
    must(
        &r,
        Level::Fix,
        "does not exist: every hook then runs without a check",
    );
}

#[test]
fn another_vault_in_this_shell_is_a_warning_with_the_export_line() {
    let w = World::new("shellvault");
    let mut c = w.ctx();
    c.env_vault = Some(w.t.join("other-vault"));
    let r = doctor::examine(&c);
    must(&r, Level::Warn, "this shell uses the vault");
    must(
        &r,
        Level::Warn,
        &format!(
            "export TORPOR_HOME={}",
            doctor::quote(&w.vault.to_string_lossy())
        ),
    );
}

// ---------------------------------------------------------------- the heartbeats

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

// ---------------------------------------------------------------- problems.log (written by the hook core)

/// Writes `<vault>/muse/problems.log` as core does: `<unix seconds> <Event> <session, 8 chars> <message>`.
/// Each line is (seconds before doctor's "now", event, message).
fn write_problems(w: &World, lines: &[(u64, &str, &str)]) -> PathBuf {
    let now_s = (w.t0() + 60_000) / 1000;
    let f = liveness::shim_dir(&w.vault).join("problems.log");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    let text: String = lines
        .iter()
        .map(|(ago, event, msg)| format!("{} {event} 01a10000 {msg}\n", now_s - ago))
        .collect();
    fs::write(&f, text).unwrap();
    f
}

fn warns(r: &Report) -> usize {
    r.0.iter().filter(|(l, _)| *l == Level::Warn).count()
}

#[test]
fn a_problem_line_saying_the_engine_did_not_run_is_a_fix_and_says_how() {
    // the messages are the ones the hook core writes (src/engine.rs, src/hook.rs)
    let msgs = [
        (
            "UserPromptSubmit",
            "torpor not found: set TORPOR_BIN or put it on PATH",
        ),
        (
            "UserPromptSubmit",
            "torpor engine /x/torpor: No such file or directory (os error 2)",
        ),
        (
            "SessionStart",
            "torpor engine /x/torpor prints \"torpor 9.9.9\", engine.lock pins \"torpor 2.3.1\"",
        ),
        (
            "Stop",
            "/x/torpor hook Stop timed out after 31500 ms and was killed",
        ),
        (
            "PreToolUse",
            "torpor hook PreToolUse exited 3: boom: database is locked",
        ),
        (
            "Stop",
            "cannot run /x/torpor: Permission denied (os error 13)",
        ),
        ("Stop", "engine.lock has no version line"),
        ("Stop", "cannot read stdin: broken pipe"),
        ("Stop", "stdin is not a JSON object (12 bytes)"),
        ("Stop", "payload has no session_id"),
        ("Stop", "unknown event name on the command line"),
    ];
    for (event, msg) in msgs {
        let w = healthy("problems-fix");
        let f = write_problems(&w, &[(300, event, msg)]);
        let before = fs::read(&f).unwrap();
        let r = w.examine();
        must(
            &r,
            Level::Fix,
            "1 hook call(s) went through without a check",
        );
        must(
            &r,
            Level::Fix,
            &format!("the last 5m ago on {event}: {msg}"),
        );
        must(&r, Level::Fix, "do: fix what that line says");
        must(&r, Level::Fix, "torpor-muse init");
        must(
            &r,
            Level::Fix,
            &format!(": > {}", doctor::quote(&f.to_string_lossy())),
        );
        assert_eq!(r.fixes(), 1, "{msg}: {}", r.text());
        every_problem_says_how(&r);
        assert_eq!(fs::read(&f).unwrap(), before, "doctor leaves the log alone");
    }
}

#[test]
fn a_cut_output_or_a_payload_that_changed_shape_is_a_warning() {
    let msgs = [
        "engine stdout is not a JSON object: dropped",
        "engine output key foo dropped",
        "decision block dropped on SessionStart",
        "additionalContext dropped: not allowed on PostToolUse",
        "block without a reason: default reason used",
        "a field was cut to keep stdout under the Muse limit",
        "payload does not fit our schema: missing field `session_id`",
        "hook_event_name is \"Notification\" but the command says Stop",
        "a message a later version of core may write",
        // only a message that begins with a "the engine did not run" phrase is a fix
        "engine output key cannot run dropped",
    ];
    for msg in msgs {
        let w = healthy("problems-warn");
        write_problems(&w, &[(30, "Stop", msg)]);
        let r = w.examine();
        must(&r, Level::Warn, "1 hook note(s) in the last 24 h");
        must(&r, Level::Warn, &format!("the last 30s ago on Stop: {msg}"));
        assert!(!r.text().contains("no problem logged"), "{}", r.text());
        assert_eq!(r.fixes(), 0, "{msg}: {}", r.text());
        every_problem_says_how(&r);
    }
}

#[test]
fn routine_lines_and_a_missing_or_empty_log_are_no_problem() {
    let w = healthy("problems-none");
    let r = w.examine();
    assert!(!r.text().contains("problems.log"), "{}", r.text());
    assert_eq!((r.fixes(), warns(&r)), (0, 0));
    write_problems(&w, &[]);
    let r = w.examine();
    must(
        &r,
        Level::Ok,
        "no problem logged by the hooks in the last 24 h (0 routine and 0 older",
    );
    write_problems(
        &w,
        &[
            (
                10,
                "Stop",
                "Stop block dropped: 2 blocks already given in this turn",
            ),
            (
                20,
                "Stop",
                "payload key brand_new_key is not in our schema (kept, passed on)",
            ),
            (
                30,
                "SessionStart",
                "gate default not set, tried again at the next session start: `torpor gate off` exited 3: refused",
            ),
        ],
    );
    let r = w.examine();
    must(
        &r,
        Level::Ok,
        "no problem logged by the hooks in the last 24 h (3 routine and 0 older",
    );
    assert_eq!((r.fixes(), warns(&r)), (0, 0), "{}", r.text());
}

#[test]
fn a_problem_older_than_a_day_is_history() {
    let down = "torpor not found: set TORPOR_BIN or put it on PATH";
    let w = healthy("problems-old");
    // exactly 24 h old still counts; a second older does not
    write_problems(&w, &[(86_400, "Stop", down)]);
    must(&w.examine(), Level::Fix, "the last 24h ago on Stop");
    write_problems(&w, &[(86_401, "Stop", down), (90_000, "Stop", "odd")]);
    let r = w.examine();
    assert_eq!((r.fixes(), warns(&r)), (0, 0), "{}", r.text());
    must(&r, Level::Ok, "(0 routine and 2 older line(s) in");
}

#[test]
fn the_newest_line_is_shown_and_each_kind_is_counted() {
    let w = healthy("problems-newest");
    write_problems(
        &w,
        &[
            (4000, "Stop", "payload has no session_id"),
            (130, "UserPromptSubmit", "cannot read stdin: late"),
            (2000, "SessionStart", "cannot read stdin: middle"),
            (50, "Stop", "engine output key foo dropped"),
            (60, "Stop", "engine output key bar dropped"),
        ],
    );
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "3 hook call(s) went through without a check in the last 24 h (the engine did not run, or the hook's input was unusable), the last 2m ago on UserPromptSubmit: cannot read stdin: late",
    );
    must(
        &r,
        Level::Warn,
        "2 hook note(s) in the last 24 h, the last 50s ago on Stop: engine output key foo dropped",
    );
}

#[test]
fn bad_lines_are_skipped_and_only_the_end_of_a_big_log_is_read() {
    let w = healthy("problems-junk");
    let f = write_problems(&w, &[(40, "Stop", "payload has no session_id")]);
    let good = fs::read(&f).unwrap();
    let mut bytes = b"garbage\n\n   \nabc Stop s1 not a time\n99999999999999999999999 Stop s1 too big\n1700000050 Stop\n".to_vec();
    bytes.extend_from_slice(b"\xff\xfe\n");
    bytes.extend_from_slice(&good);
    fs::write(&f, &bytes).unwrap();
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "1 hook call(s) went through without a check",
    );
    assert_eq!(warns(&r), 0, "{}", r.text());
    // a fresh line that lies more than 128 KiB before the end is not read: only the end counts
    let now_s = (w.t0() + 60_000) / 1000;
    let mut big = format!("{} Stop 01a10000 payload has no session_id\n", now_s - 20).into_bytes();
    let old = format!("{} Stop 01a10000 {}\n", now_s - 200_000, "x".repeat(200));
    for _ in 0..1000 {
        big.extend_from_slice(old.as_bytes());
    }
    big.extend_from_slice(&good);
    assert!(big.len() > 200 * 1024);
    fs::write(&f, &big).unwrap();
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "1 hook call(s) went through without a check",
    );
    // the line the limit cuts in two is dropped, even when its second half looks like a line of its own
    let fake = format!("{} Stop 01a10000 cannot read stdin: cut\n", now_s - 10);
    let routine = |pad: usize| {
        format!(
            "{} Stop 01a10000 Stop block dropped: {}\n",
            now_s - 5,
            "y".repeat(pad)
        )
    };
    let cap = doctor::PROBLEMS_READ as usize;
    let rest = cap - fake.len() - routine(0).len();
    let start = format!("{} Stop 01a10000 start of a long line ", now_s - 300);
    let mut cut = start.as_bytes().to_vec();
    cut.extend_from_slice(fake.as_bytes());
    cut.extend_from_slice(routine(rest).as_bytes());
    assert_eq!(
        cut.len(),
        start.len() + cap,
        "the limit cuts exactly where `fake` starts"
    );
    fs::write(&f, &cut).unwrap();
    let r = w.examine();
    assert_eq!((r.fixes(), warns(&r)), (0, 0), "{}", r.text());
    must(&r, Level::Ok, "(1 routine and 0 older line(s) in");
}

#[test]
fn only_the_log_of_the_vault_the_hooks_write_to_is_read() {
    let w = healthy("problems-vault");
    let other = w.t.join("other-vault");
    let f = liveness::shim_dir(&other).join("problems.log");
    fs::create_dir_all(f.parent().unwrap()).unwrap();
    let now_s = (w.t0() + 60_000) / 1000;
    fs::write(
        &f,
        format!("{} Stop 01a10000 payload has no session_id\n", now_s - 5),
    )
    .unwrap();
    let r = w.examine();
    assert_eq!((r.fixes(), warns(&r)), (0, 0), "{}", r.text());
}

#[test]
fn a_stop_that_went_through_without_the_engine_is_not_a_live_gate() {
    // healthy(): the Stop heartbeat is 2 s after the settings were written, doctor's "now" is 60 s after
    let down = "torpor not found: set TORPOR_BIN or put it on PATH";
    let w = healthy("problems-gate");
    must(&w.examine(), Level::Ok, "gate live: Stop ran");
    // in the same second as the Stop heartbeat or later, whatever the event: a call went through without the engine
    for (ago, event) in [(58, "Stop"), (30, "UserPromptSubmit"), (0, "SessionStart")] {
        write_problems(&w, &[(ago, event, down)]);
        let r = w.examine();
        must(
            &r,
            Level::Fix,
            "gate not live: a hook call at or after the last Stop (58s ago, session s-main) went through without the engine",
        );
        mustnt(&r, Level::Ok, "gate live");
        must(
            &r,
            Level::Fix,
            "1 hook call(s) went through without a check",
        );
        every_problem_says_how(&r);
    }
    // a failure from before that Stop: the engine is back, the gate is live (the failure is still reported)
    write_problems(&w, &[(59, "Stop", down)]);
    let r = w.examine();
    must(&r, Level::Ok, "gate live: Stop ran");
    must(
        &r,
        Level::Fix,
        "1 hook call(s) went through without a check",
    );
    // a cut output is a note, not a gate that is down
    write_problems(&w, &[(10, "Stop", "engine output key foo dropped")]);
    must(&w.examine(), Level::Ok, "gate live: Stop ran");
}

#[test]
fn a_log_with_many_lines_and_no_prompt_is_a_fix() {
    let w = World::new("drift");
    let sid = "01a00000-0000-7000-8000-000000000004";
    let text: String = (0..250).map(|_| common::filler_line(10)).collect();
    common::put_log(&w.sessions(), sid, &text);
    w.beat(sid, true, "UserPromptSubmit", 1000);
    w.beat(sid, true, "Stop", 2000);
    let paths = transcript::Paths {
        vault: w.vault.clone(),
        sessions: w.sessions(),
    };
    transcript::translate(&paths, sid).unwrap();
    let r = w.examine();
    must(
        &r,
        Level::Fix,
        "hold no prompt: its log format may have changed",
    );
    every_problem_says_how(&r);
}

#[test]
fn lines_of_muses_log_that_could_not_be_read_are_a_warning() {
    let w = World::new("badlines");
    let sid = "s-bad";
    let mut b = LogBuilder::new(sid);
    b.accepted("chat", "main");
    let mut raw = b.text().into_bytes();
    raw.extend_from_slice(b"{\"payload_type\":\"runtime.user_intent.accepted\" broken\n");
    common::put_log(&w.sessions(), sid, &String::from_utf8(raw).unwrap());
    w.beat(sid, true, "UserPromptSubmit", 1000);
    w.beat(sid, true, "Stop", 2000);
    let paths = transcript::Paths {
        vault: w.vault.clone(),
        sessions: w.sessions(),
    };
    transcript::translate(&paths, sid).unwrap();
    let r = w.examine();
    must(
        &r,
        Level::Warn,
        "1 line(s) of Muse's log for session s-bad had an unexpected shape",
    );
    mustnt(&r, Level::Fix, "hold no prompt");
}

// ---------------------------------------------------------------- what Muse's own log says

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

// ---------------------------------------------------------------- the probe

#[test]
fn the_probe_runs_the_command_like_muse_does() {
    let w = World::new("probeenv");
    let r = w.examine();
    must(&r, Level::Ok, "probe:");
    let env = w.seen("env.txt");
    let allowed: BTreeSet<&str> = [
        "PATH",
        "USER",
        "LOGNAME",
        "TMPDIR",
        "TEMP",
        "TMP",
        "LANG",
        "LC_ALL",
        "TERM",
        "HOME",
        "SHELL",
        "MUSE_SESSION_ID",
        "MUSE_THREAD_ID",
        "TORPOR_HOME",
        "TORPOR_BIN",
        // what a shell adds by itself
        "PWD",
        "OLDPWD",
        "SHLVL",
        "_",
    ]
    .into();
    let keys: BTreeSet<&str> = env
        .lines()
        .filter_map(|l| l.split_once('=').map(|(k, _)| k))
        .collect();
    let extra: Vec<&&str> = keys.difference(&allowed).collect();
    assert!(extra.is_empty(), "the probe leaked {extra:?}");
    let get = |k: &str| {
        env.lines()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
            .map(str::to_string)
            .unwrap_or_else(|| panic!("{k} is not set in {env}"))
    };
    assert_eq!(get("HOME"), w.home.to_string_lossy());
    assert_eq!(get("SHELL"), "/bin/sh");
    assert_eq!(get("MUSE_SESSION_ID"), liveness::PROBE_SID);
    assert_eq!(get("MUSE_THREAD_ID"), liveness::PROBE_SID);
    let probe_vault = w.tmp.join("torpor-muse-doctor-probe");
    assert_eq!(get("TORPOR_HOME"), probe_vault.to_string_lossy());
    assert_ne!(get("TORPOR_HOME"), w.vault.to_string_lossy());
    assert_eq!(get("TORPOR_BIN"), w.engine().to_string_lossy());
    let payload: Value = serde_json::from_str(&w.seen("stdin.txt")).unwrap();
    assert_eq!(payload["hook_event_name"], "SubagentStart");
    assert_eq!(payload["session_id"], liveness::PROBE_SID);
    assert!(payload["transcript_path"].is_null());
    let cwd = fs::canonicalize(w.seen("cwd.txt").trim()).unwrap();
    assert_eq!(cwd, fs::canonicalize(w.t.join("proj")).unwrap());
    assert!(!w.vault.exists(), "the real vault is not touched");
}

#[test]
fn a_shell_start_up_file_that_prints_is_found() {
    let w = World::new("chatty");
    let mut c = w.ctx();
    c.shell = Some(fake_shell().to_string_lossy().into_owned());
    must(&doctor::examine(&c), Level::Ok, "prints nothing");

    fs::write(w.home.join(".fakerc"), "echo 'Welcome back'\n").unwrap();
    let r = doctor::examine(&c);
    must(
        &r,
        Level::Fix,
        "printed text that is not one JSON object: Welcome back",
    );
    must(&r, Level::Fix, "hands that text to the model as context");
    must(&r, Level::Fix, "[ -t 1 ]");
    every_problem_says_how(&r);
    assert_eq!(doctor::finish(&r), 1);

    // a good answer after the noise is still not one JSON object
    fs::write(w.bin.join("stdout.txt"), "{}").unwrap();
    must(&doctor::examine(&c), Level::Fix, "not one JSON object");
}

/// The same with the shell the mechanism is about. NOT run where there is no zsh (this machine has none; macOS has).
#[test]
fn a_zshenv_that_prints_is_found_where_zsh_exists() {
    let Some(zsh) = ["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"]
        .iter()
        .find(|p| Path::new(p).exists())
    else {
        println!("SKIPPED: no zsh on this machine");
        return;
    };
    let w = World::new("zsh");
    let mut c = w.ctx();
    c.shell = Some(zsh.to_string());
    fs::write(w.home.join(".zshenv"), "echo zsh-noise\n").unwrap();
    must(
        &doctor::examine(&c),
        Level::Fix,
        "printed text that is not one JSON object: zsh-noise",
    );
}

#[test]
fn a_hook_that_answers_with_one_json_object_is_fine() {
    let w = World::new("answers");
    fs::write(w.bin.join("stdout.txt"), "{}\n").unwrap();
    let r = w.examine();
    must(&r, Level::Ok, "prints one JSON object");
    assert_eq!(r.fixes(), 1, "only the missing heartbeat: {}", r.text());
    mustnt(&r, Level::Fix, "probe:");
}

#[test]
fn text_around_the_json_object_is_a_fix() {
    for (name, out) in [
        ("after", "{}\nbye"),
        ("before", "hi\n{}"),
        ("two", "{}{}"),
        ("array", "[]"),
        ("number", "5"),
        ("string", "\"x\""),
    ] {
        let w = World::new(name);
        fs::write(w.bin.join("stdout.txt"), out).unwrap();
        must(&w.examine(), Level::Fix, "not one JSON object");
    }
}

#[test]
fn a_hook_that_exits_with_an_error_is_a_fix() {
    let w = World::new("exit3");
    fs::write(w.bin.join("exit"), "3").unwrap();
    let r = w.examine();
    must(&r, Level::Fix, "probe: the hook command exits with Some(3)");
    every_problem_says_how(&r);
}

#[test]
fn a_shell_that_cannot_start_is_a_fix() {
    let w = World::new("noshell");
    let mut c = w.ctx();
    c.shell = Some(w.t.join("no-such-shell").to_string_lossy().into_owned());
    let r = doctor::examine(&c);
    must(&r, Level::Fix, "probe: cannot start");
}

#[test]
fn a_probe_that_hangs_is_stopped_with_everything_it_started() {
    let w = World::new("hang");
    fs::write(w.bin.join("hang"), "").unwrap();
    let mut c = w.ctx();
    c.probe_timeout = Duration::from_secs(1);
    let t = Instant::now();
    let r = doctor::examine(&c);
    let took = t.elapsed();
    let sleeper: u32 = w.seen("sleeper.pid").trim().parse().unwrap();
    let gone = wait_gone(sleeper, 5);
    if !gone {
        end(sleeper);
    }
    must(&r, Level::Fix, "did not finish in 1s");
    assert!(took < Duration::from_secs(10), "{took:?}");
    assert!(gone, "the hook's own child {sleeper} was left running");
}

#[test]
fn a_background_process_that_keeps_the_pipes_open_does_not_hold_doctor() {
    let w = World::new("background");
    fs::write(w.bin.join("bg"), "").unwrap();
    fs::write(w.bin.join("stdout.txt"), "{}").unwrap();
    let t = Instant::now();
    let r = w.examine();
    let took = t.elapsed();
    end(w.seen("sleeper.pid").trim().parse().unwrap());
    must(&r, Level::Ok, "prints one JSON object");
    assert!(took < Duration::from_secs(4), "{took:?}");
}

// ---------------------------------------------------------------- orphans of killed checks

#[test]
fn leftover_folders_of_killed_checks_are_listed_and_nothing_is_deleted() {
    let w = World::new("orphans");
    let old = T0 - 3_600_000;
    let scratch = w.vault.join("0123456789abcdef/scratch/run1");
    fs::create_dir_all(&scratch).unwrap();
    fs::write(scratch.join("app.pgid"), "999999\n").unwrap();
    let tp = w.tmp.join("tp-old-1");
    let fresh = w.tmp.join("tp-new-1");
    let future = w.tmp.join("tp-future-1");
    let other = w.tmp.join("not-a-browser-folder");
    for d in [&tp, &fresh, &future, &other] {
        fs::create_dir_all(d).unwrap();
    }
    for d in [&scratch, &tp, &other] {
        set_mtime(d, old);
    }
    set_mtime(&fresh, T0 + 30_000); // 30 s before "now": a check that may still be running
    set_mtime(&future, T0 + 120_000); // a clock that stepped back: younger than young, not ancient
    let r = w.examine();
    must(
        &r,
        Level::Warn,
        &format!("leftover folder of a killed check: {}", scratch.display()),
    );
    must(
        &r,
        Level::Warn,
        &format!("rm -rf {}", doctor::quote(&scratch.to_string_lossy())),
    );
    must(
        &r,
        Level::Warn,
        &format!("leftover folder of a killed check: {}", tp.display()),
    );
    mustnt(&r, Level::Warn, "tp-new-1");
    mustnt(&r, Level::Warn, "tp-future-1");
    mustnt(&r, Level::Warn, "not-a-browser-folder");
    mustnt(&r, Level::Warn, "kill -9");
    every_problem_says_how(&r);
    // leftovers alone are a warning, and doctor removes nothing
    mustnt(&r, Level::Fix, "leftover folder");
    for d in [&scratch, &tp, &fresh, &future, &other] {
        assert!(d.is_dir(), "{} is gone", d.display());
    }
    assert!(scratch.join("app.pgid").is_file());
}

/// A process that keeps working in the folder of a killed check (its command line names the folder), in a process
/// group of its own or in the group of this test.
fn holder(dir: &Path, own_group: bool) -> std::process::Child {
    let mut c = Command::new("/bin/sh");
    c.args(["-c", "while :; do sleep 1; done", "sh"])
        .arg(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if own_group {
        c.process_group(0);
    }
    c.spawn().unwrap()
}

fn pgid_of(pid: u32) -> u32 {
    let out = Command::new("ps")
        .args(["-o", "pgid=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

#[test]
fn a_process_left_behind_is_named_with_the_commands_that_end_it_and_not_killed() {
    let w = World::new("holder");
    let dir = w.tmp.join("tp-holder-1");
    fs::create_dir_all(&dir).unwrap();
    let mut child = holder(&dir, true);
    let pid = child.id();
    fs::write(dir.join("app.pgid"), format!("{pid}\n")).unwrap();
    set_mtime(&dir, T0 - 3_600_000);
    let r = w.examine();
    let still_running = child.try_wait().unwrap().is_none();
    let _ = child.kill(); // the process this test started
    let _ = child.wait();
    must(&r, Level::Fix, "a killed check left processes running for");
    must(&r, Level::Fix, &format!("{pid} /bin/sh -c while"));
    must(&r, Level::Fix, &format!("kill -9 -- -{pid}"));
    must(
        &r,
        Level::Fix,
        &format!("rm -rf {}", doctor::quote(&dir.to_string_lossy())),
    );
    every_problem_says_how(&r);
    assert_eq!(doctor::finish(&r), 1);
    assert!(still_running, "doctor must not kill anything itself");
    assert!(dir.is_dir(), "doctor must not delete anything itself");
}

#[test]
fn a_process_in_no_listed_group_is_ended_alone_and_a_stale_group_number_is_never_used() {
    let w = World::new("loose");
    let dir = w.tmp.join("tp-loose-1");
    fs::create_dir_all(&dir).unwrap();
    let mut child = holder(&dir, true);
    let pid = child.id();
    fs::write(dir.join("app.pgid"), "1\n").unwrap(); // a recycled number: the init group
    set_mtime(&dir, T0 - 3_600_000);
    let r = w.examine();
    let _ = child.kill();
    let _ = child.wait();
    must(&r, Level::Fix, &format!("kill -9 {pid}; rm -rf"));
    mustnt(&r, Level::Fix, "kill -9 --");
}

#[test]
fn a_process_in_the_group_doctor_runs_in_is_never_proposed_for_a_kill() {
    let w = World::new("owngroup");
    let dir = w.tmp.join("tp-own-1");
    fs::create_dir_all(&dir).unwrap();
    let mut child = holder(&dir, false); // shares the group of this test, as a sibling of doctor's shell would
    let group = pgid_of(std::process::id());
    assert_eq!(pgid_of(child.id()), group);
    fs::write(dir.join("app.pgid"), format!("{group}\n")).unwrap();
    set_mtime(&dir, T0 - 3_600_000);
    let r = w.examine();
    let _ = child.kill();
    let _ = child.wait();
    must(&r, Level::Fix, "a killed check left processes running for");
    must(&r, Level::Fix, "rm -rf");
    mustnt(&r, Level::Fix, "kill -9");
}

// ---------------------------------------------------------------- where Muse keeps its logs

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

// ---------------------------------------------------------------- the helpers

#[test]
fn words_follow_the_quoting_of_a_shell_line() {
    let w = doctor::words;
    assert_eq!(
        w(r#"env A=1 'b c' "d\"e" f\ g"#),
        Some(
            ["env", "A=1", "b c", "d\"e", "f g"]
                .map(String::from)
                .to_vec()
        )
    );
    assert_eq!(w(""), Some(vec![]));
    assert_eq!(w("''"), Some(vec![String::new()]));
    for open in ["'open", "\"open", "trailing\\"] {
        assert_eq!(w(open), None, "{open}");
    }
}

#[test]
fn quote_and_words_round_trip() {
    for s in [
        "plain",
        "/a/b-c_d.e=f:g@h%i+j,k",
        "with space",
        "it's",
        "$HOME",
        "`x`",
        "a\nb",
        "",
        "tab\t",
        "back\\slash",
        "\"dq\"",
        "*?[",
        "emoji \u{2713}",
    ] {
        assert_eq!(
            doctor::words(&doctor::quote(s)),
            Some(vec![s.to_string()]),
            "{s:?}"
        );
    }
    assert_eq!(doctor::quote("plain"), "plain");
    assert_eq!(doctor::quote("it's"), r"'it'\''s'");
}

#[test]
fn ours_is_a_torpor_muse_hook_command() {
    for c in [
        "env TORPOR_HOME=/v TORPOR_BIN=/e /x/torpor-muse hook Stop",
        "'/a b/torpor-muse' hook SessionStart",
        "torpor-muse hook Stop",
        "env 'TORPOR_HOME=/v w' /x/torpor-muse hook PreToolUse",
    ] {
        assert!(doctor::is_ours(c), "{c}");
    }
    for c in [
        "echo torpor-muse hook",
        "/x/torpor-muse-old hook Stop",
        "/x/other hook Stop",
        "torpor-muse",
        "torpor-muse hook",
        "/x/torpor-muse 'hook Stop",
        "/x/torpor-muse init Stop",
        "",
    ] {
        assert!(!doctor::is_ours(c), "{c:?}");
    }
}

#[test]
fn the_vault_is_what_the_command_carries() {
    let v = |c: &str| doctor::vault_of(c);
    assert_eq!(
        v("env TORPOR_HOME=/v/w TORPOR_BIN=/e /x/torpor-muse hook Stop"),
        Some(PathBuf::from("/v/w"))
    );
    assert_eq!(
        v("env 'TORPOR_HOME=/a b/c' /x/torpor-muse hook Stop"),
        Some(PathBuf::from("/a b/c"))
    );
    assert_eq!(v("env TORPOR_HOME= /x/torpor-muse hook Stop"), None);
    assert_eq!(v("/x/torpor-muse hook Stop"), None);
    assert_eq!(v("env 'TORPOR_HOME=/open /x/torpor-muse hook Stop"), None);
}

#[test]
fn the_probe_command_swaps_the_vault_and_the_event_and_nothing_else() {
    let vault = Path::new("/tmp/probe vault");
    let p = |c: &str| doctor::probe_command(c, vault, "SubagentStart");
    let a = p("env TORPOR_HOME=/real TORPOR_BIN=/e /x/torpor-muse hook Stop").unwrap();
    assert_eq!(
        doctor::words(&a).unwrap(),
        [
            "env",
            "TORPOR_HOME=/tmp/probe vault",
            "TORPOR_BIN=/e",
            "/x/torpor-muse",
            "hook",
            "SubagentStart"
        ]
    );
    assert!(doctor::is_ours(&a));
    // a command that carries no vault gets one, and one that does not start with env gets env
    let b = doctor::words(&p("env TORPOR_BIN=/e /x/torpor-muse hook Stop").unwrap()).unwrap();
    assert_eq!(
        b[..3],
        ["env", "TORPOR_HOME=/tmp/probe vault", "TORPOR_BIN=/e"]
    );
    let c = doctor::words(&p("/x/torpor-muse hook Stop").unwrap()).unwrap();
    assert_eq!(
        c,
        [
            "env",
            "TORPOR_HOME=/tmp/probe vault",
            "/x/torpor-muse",
            "hook",
            "SubagentStart"
        ]
    );
    assert_eq!(p("/x/torpor-muse 'hook"), None);
}

// ---------------------------------------------------------------- the engine's gate (F4b)

/// A healthy world whose engine is the fake `torpor gate` that says `said` and exits `code`.
fn gate_world(name: &str, said: &str, code: i32) -> World {
    let w = healthy(name);
    fs::remove_file(w.engine()).unwrap();
    std::os::unix::fs::symlink(fake_engine(), w.engine()).unwrap();
    fs::write(w.bin.join("gate.say"), said).unwrap();
    fs::write(w.bin.join("gate.exit"), code.to_string()).unwrap();
    w
}

const GATE_LINE: &str = "torpor gate (this folder)";

/// F4b: the line about the gate is the engine's own `torpor gate` (read only), run with the engine and the vault the hooks use
/// and in the folder doctor is run in, put as what a broken check does. An `info`, never a fault: it is a setting.
#[test]
fn doctor_says_what_a_broken_check_does_here_from_the_engines_own_gate() {
    for (said, code, needle) in [
        (
            "The gate is off. Promises show failures but do not stop the AI. Turn it on: torpor gate on",
            0,
            "told to the AI before it says Done",
        ),
        (
            "The gate is advisory: two disputes were upheld within 7 days. Turn it on again: torpor gate on",
            0,
            "advisory",
        ),
        ("The gate is on.", 0, "can stop the AI"),
        (
            "Torpor has no save points for this folder yet.",
            5,
            "not read",
        ),
    ] {
        let w = gate_world("gate-line", said, code);
        let r = w.examine();
        let line = r.0.iter().find(|(_, s)| s.starts_with(GATE_LINE));
        let line = line.unwrap_or_else(|| panic!("no gate line for {said:?} in:\n{}", r.text()));
        assert!(line.0 == Level::Info && line.1.contains(needle), "{line:?}");
        assert_eq!(w.seen("gate.args").trim(), "gate");
        assert_eq!(w.seen("gate.home").trim(), w.vault.to_string_lossy());
        let proj = fs::canonicalize(w.t.join("proj")).unwrap();
        assert_eq!(w.seen("gate.cwd").trim(), proj.to_string_lossy());
    }
    // off says how to let checks stop the AI, on says how to take that away, both as the user's own command
    let off = gate_world("gate-off", "The gate is off. Turn it on: torpor gate on", 0).examine();
    must(
        &off,
        Level::Info,
        "run `torpor gate on` in your own terminal",
    );
    let on = gate_world("gate-on", "The gate is on.", 0).examine();
    must(
        &on,
        Level::Info,
        "run `torpor gate off` in your own terminal",
    );
}

/// Which engine the hooks use is known only from the TORPOR_BIN in their commands: without one doctor does not guess from
/// its own PATH, and runs nothing.
#[test]
fn doctor_has_no_gate_line_when_the_hook_commands_do_not_say_which_engine_they_use() {
    let w = gate_world("gate-none", "The gate is on.", 0);
    // the control: with TORPOR_BIN in the commands the engine is asked
    let asked = w.examine();
    assert!(asked.text().contains(GATE_LINE), "{}", asked.text());
    fs::remove_file(w.bin.join("gate.args")).unwrap();
    let mut v = w.settings_value(&[("SessionStart", 10), ("UserPromptSubmit", 10), ("Stop", 35)]);
    for e in ["SessionStart", "UserPromptSubmit", "Stop"] {
        let cmd = format!(
            "env TORPOR_HOME={} {} hook {e}",
            doctor::quote(&w.vault.to_string_lossy()),
            doctor::quote(&w.exe().to_string_lossy())
        );
        v["hooks"][e][0]["hooks"][0]["command"] = json!(cmd);
    }
    w.write_json(&v);
    let r = w.examine();
    assert!(!r.text().contains(GATE_LINE), "{}", r.text());
    assert!(!w.bin.join("gate.args").exists(), "the engine was run");
}

// ---------------------------------------------------------------- the binary

#[test]
fn the_binary_runs_doctor_and_exits_1_when_nothing_is_installed() {
    let t = Tmp::new("binary");
    let (home, tmp) = (t.join("home"), t.join("tmp"));
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&tmp).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_torpor-muse"))
        .arg("doctor")
        .env_clear()
        .env("HOME", &home)
        .env("TMPDIR", &tmp)
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{text}");
    assert!(text.contains("FIX "), "{text}");
    assert!(text.contains("do: torpor-muse init"), "{text}");
    assert!(text.contains("doctor: 1 to fix"), "{text}");
}

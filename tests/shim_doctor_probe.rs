//! `torpor-muse doctor`'s probe and the engine's gate line, run against fake hooks, shells and engines.
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

use doctor::Level;
use doctor_common::{
    World, end, every_problem_says_how, healthy, hook, install, must, mustnt, script, suggest,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

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

fn fake_engine() -> PathBuf {
    static C: OnceLock<PathBuf> = OnceLock::new();
    script(&C, "fake-torpor-engine", FAKE_ENGINE)
}

/// Stands in for a login shell whose start-up file talks (zsh reads ~/.zshenv for every `-c`; there is no zsh here).
const FAKE_SHELL: &str = r#"#!/bin/sh
[ -f "$HOME/.fakerc" ] && . "$HOME/.fakerc"
exec /bin/sh "$@"
"#;

fn fake_shell() -> PathBuf {
    static C: OnceLock<PathBuf> = OnceLock::new();
    script(&C, "fake-shell", FAKE_SHELL)
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
            util::quote(&w.vault.to_string_lossy()),
            util::quote(&w.exe().to_string_lossy())
        );
        v["hooks"][e][0]["hooks"][0]["command"] = json!(cmd);
    }
    w.write_json(&v);
    let r = w.examine();
    assert!(!r.text().contains(GATE_LINE), "{}", r.text());
    assert!(!w.bin.join("gate.args").exists(), "the engine was run");
}

//! What the shim_doctor_* test crates share: the stubs doctor.rs needs, a fake torpor-muse, and a world to examine.
#![allow(dead_code)]

use crate::common::{self, LogBuilder, Tmp, sid_of};
use crate::doctor::{self, Ctx, Level, Report};
use crate::{liveness, util};
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// doctor.rs reads the engine's time limit per event from src/hook/events.rs; one event is enough here.
pub mod hook {
    pub struct Ev {
        pub name: &'static str,
        pub engine_ms: u64,
    }
    pub const EVENTS: &[Ev] = &[Ev {
        name: "Stop",
        engine_ms: 31_500,
    }];
}

/// doctor.rs asks src/install/settings.rs where Muse's settings file is and why `init` would refuse it; these tests
/// hand doctor the path directly and leave the refusal to tests/core_install.rs and the unit tests.
#[allow(dead_code)]
pub mod install {
    pub fn settings_path() -> std::path::PathBuf {
        std::path::PathBuf::from("settings.json")
    }
    pub fn refusal(_: &std::path::Path) -> Option<String> {
        None
    }
}

/// doctor.rs asks `suggest::writer_off` (src/suggest/trigger.rs) if the check-writer has a key; here it always has.
#[allow(dead_code)]
pub mod suggest {
    pub fn writer_off(_: bool) -> Option<String> {
        None
    }
}

/// Every world starts here (a date in the past, so that "now" is whatever the test says it is).
pub(crate) const T0: u64 = 1_700_000_000_000;

/// Stands in for torpor-muse. What it does is told by files next to the (symlinked) path it was started by, so one
/// script serves every test and is written only once (a script written while other tests start processes can fail
/// with "text file busy").
pub(crate) const FAKE_EXE: &str = r#"#!/bin/sh
d=$(dirname "$0")
cat > "$d/stdin.txt"
env > "$d/env.txt"
pwd > "$d/cwd.txt"
[ -f "$d/stdout.txt" ] && cat "$d/stdout.txt"
[ -f "$d/bg" ] && { sleep 5 & echo $! > "$d/sleeper.pid"; }
[ -f "$d/hang" ] && { sleep 60 & echo $! > "$d/sleeper.pid"; wait; }
exit "$(cat "$d/exit" 2>/dev/null || echo 0)"
"#;

pub(crate) fn script(cell: &'static OnceLock<PathBuf>, name: &str, body: &str) -> PathBuf {
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

pub(crate) fn fake_exe() -> PathBuf {
    static C: OnceLock<PathBuf> = OnceLock::new();
    script(&C, "fake-torpor-muse", FAKE_EXE)
}

pub(crate) fn ms_of(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
}

pub(crate) fn set_mtime(p: &Path, ms: u64) {
    fs::File::open(p)
        .unwrap()
        .set_modified(UNIX_EPOCH + Duration::from_millis(ms))
        .unwrap();
}

pub(crate) struct World {
    pub(crate) t: Tmp,
    pub(crate) home: PathBuf,
    pub(crate) vault: PathBuf,
    pub(crate) settings: PathBuf,
    pub(crate) bin: PathBuf,
    pub(crate) tmp: PathBuf,
}

impl World {
    /// Settings with the three hooks the gate needs, written as `init` writes them; a fake torpor-muse and engine.
    pub(crate) fn new(name: &str) -> World {
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

    pub(crate) fn exe(&self) -> PathBuf {
        self.bin.join("torpor-muse")
    }

    pub(crate) fn engine(&self) -> PathBuf {
        self.bin.join("torpor")
    }

    pub(crate) fn command(&self, event: &str) -> String {
        format!(
            "env TORPOR_HOME={} TORPOR_BIN={} {} hook {event}",
            util::quote(&self.vault.to_string_lossy()),
            util::quote(&self.engine().to_string_lossy()),
            util::quote(&self.exe().to_string_lossy())
        )
    }

    pub(crate) fn settings_value(&self, hooks: &[(&str, u64)]) -> Value {
        let h: serde_json::Map<String, Value> = hooks
            .iter()
            .map(|(e, t)| {
                let entry = json!([{"hooks": [{"type": "command", "command": self.command(e), "timeout": t}]}]);
                (e.to_string(), entry)
            })
            .collect();
        json!({"schema_version": 1, "hooks": h})
    }

    pub(crate) fn write_json(&self, v: &Value) {
        fs::create_dir_all(self.settings.parent().unwrap()).unwrap();
        fs::write(&self.settings, v.to_string()).unwrap();
        set_mtime(&self.settings, T0);
    }

    /// When the settings were written (heartbeat times in the tests are relative to it).
    pub(crate) fn t0(&self) -> u64 {
        fs::metadata(&self.settings)
            .and_then(|m| m.modified())
            .map_or(T0, ms_of)
    }

    pub(crate) fn sessions(&self) -> PathBuf {
        self.home.join(".local/share/muse/sessions")
    }

    pub(crate) fn ctx(&self) -> Ctx {
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

    pub(crate) fn examine(&self) -> Report {
        doctor::examine(&self.ctx())
    }

    pub(crate) fn beat(&self, sid: &str, main: bool, event: &str, dt: u64) {
        liveness::beat_in(&self.vault, event, sid, main, self.t0() + dt).unwrap();
    }

    pub(crate) fn put(&self, text: &str) -> PathBuf {
        common::put_log(&self.sessions(), &sid_of(text), text)
    }

    /// What the fake torpor-muse was started with.
    pub(crate) fn seen(&self, file: &str) -> String {
        fs::read_to_string(self.bin.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))
    }
}

/// A world where the gate is proven: a finished turn, and a log of the tested Muse version.
pub(crate) fn healthy(name: &str) -> World {
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

#[track_caller]
pub(crate) fn must(r: &Report, l: Level, needle: &str) {
    assert!(
        r.0.iter().any(|(k, s)| *k == l && s.contains(needle)),
        "expected {l:?} with {needle:?} in:\n{}",
        r.text()
    );
}

#[track_caller]
pub(crate) fn mustnt(r: &Report, l: Level, needle: &str) {
    assert!(
        !r.0.iter().any(|(k, s)| *k == l && s.contains(needle)),
        "unexpected {l:?} with {needle:?} in:\n{}",
        r.text()
    );
}

/// Every Fix and Warn names the way out on a line of its own.
#[track_caller]
pub(crate) fn every_problem_says_how(r: &Report) {
    for (l, s) in &r.0 {
        if matches!(l, Level::Fix | Level::Warn) {
            assert!(s.contains("\n      do: "), "no way out given for: {s}");
        }
    }
}

/// Ends a process one of these tests started itself (its pid is in a file the fake hook wrote).
pub(crate) fn end(pid: u32) {
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
}

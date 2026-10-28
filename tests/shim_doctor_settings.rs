//! `torpor-muse doctor` against made-up settings files and hook commands, and the binary's exit code.
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

use common::Tmp;
use doctor::{Level, Report};
use doctor_common::{World, every_problem_says_how, healthy, hook, install, must, mustnt, suggest};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn edited(name: &str, f: impl FnOnce(&World, &mut Value)) -> Report {
    let w = World::new(name);
    let mut v = w.settings_value(&[("SessionStart", 10), ("UserPromptSubmit", 10), ("Stop", 35)]);
    f(&w, &mut v);
    w.write_json(&v);
    w.examine()
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
            util::quote(&w.vault.to_string_lossy())
        ),
    );
}

#[test]
fn words_follow_the_quoting_of_a_shell_line() {
    let w = doctor::hook_command::words;
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
            doctor::hook_command::words(&util::quote(s)),
            Some(vec![s.to_string()]),
            "{s:?}"
        );
    }
    assert_eq!(util::quote("plain"), "plain");
    assert_eq!(util::quote("it's"), r"'it'\''s'");
}

#[test]
fn ours_is_a_torpor_muse_hook_command() {
    for c in [
        "env TORPOR_HOME=/v TORPOR_BIN=/e /x/torpor-muse hook Stop",
        "'/a b/torpor-muse' hook SessionStart",
        "torpor-muse hook Stop",
        "env 'TORPOR_HOME=/v w' /x/torpor-muse hook PreToolUse",
    ] {
        assert!(doctor::hook_command::is_ours(c), "{c}");
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
        assert!(!doctor::hook_command::is_ours(c), "{c:?}");
    }
}

#[test]
fn the_vault_is_what_the_command_carries() {
    let v = |c: &str| doctor::hook_command::vault_of(c);
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
    let p = |c: &str| doctor::hook_command::probe_command(c, vault, "SubagentStart");
    let a = p("env TORPOR_HOME=/real TORPOR_BIN=/e /x/torpor-muse hook Stop").unwrap();
    assert_eq!(
        doctor::hook_command::words(&a).unwrap(),
        [
            "env",
            "TORPOR_HOME=/tmp/probe vault",
            "TORPOR_BIN=/e",
            "/x/torpor-muse",
            "hook",
            "SubagentStart"
        ]
    );
    assert!(doctor::hook_command::is_ours(&a));
    // a command that carries no vault gets one, and one that does not start with env gets env
    let b = doctor::hook_command::words(&p("env TORPOR_BIN=/e /x/torpor-muse hook Stop").unwrap())
        .unwrap();
    assert_eq!(
        b[..3],
        ["env", "TORPOR_HOME=/tmp/probe vault", "TORPOR_BIN=/e"]
    );
    let c = doctor::hook_command::words(&p("/x/torpor-muse hook Stop").unwrap()).unwrap();
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

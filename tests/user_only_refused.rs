//! The user's own commands as Muse Code meets them. The engine's command line refuses `torpor off`, `on`, `forget`,
//! `retain`, `runner install`, `undo --force`, `gate on|off`, `promise ok|drop|block` and `promise show --spec` only
//! when `CLAUDECODE` is set (engine src/cli.rs), and Muse Code does not set it. Under Muse the one guard is the engine's
//! PreToolUse judge (engine src/destructive.rs `user_only_args`), reached through this adapter's hook. This runs that
//! path end to end: the pinned engine (TORPOR_BIN), the adapter's own hook, bash payloads as Muse sends them. A refusal
//! has the shape of the two the real Muse run saw work (E8, tests/hook_outside.rs; E5 case 4, tests/hook_database.rs).
//! The commands the guide gives the AI go through. A script file that holds `torpor off` is not stopped (a known limit,
//! pinned at the end).

mod hook_common;
use hook_common::*;
use serde_json::{Value, json};
use std::fs;

fn bash(r: &Rig, cmd: &str) -> Value {
    let mut p = r.fixture("PreToolUse-bash");
    p["tool_input"]["command"] = json!(cmd);
    p
}

/// What the hook printed for a bash call in `r`'s session (it must exit 0 and print what Muse accepts).
fn judge(r: &Rig, cmd: &str) -> String {
    r.hook_ok("PreToolUse", &bash(r, cmd)).out
}

/// The permissionDecision in `out`, None when the hook let the call through (no output, or a note without a decision).
fn decision(out: &str) -> Option<String> {
    if out.is_empty() {
        return None;
    }
    let v: Value = serde_json::from_str(out).expect("a JSON answer");
    let d = &v["hookSpecificOutput"]["permissionDecision"];
    d.as_str().map(str::to_owned)
}

/// The shape of the refusals the real Muse run saw: one object, `hookSpecificOutput`, with these three keys and no note
/// beside them. Only the words of the reason differ, so the shape has it blank.
fn proven_shape() -> Value {
    json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": ""
    }})
}

/// The reason of the refusal in `out`; panics unless `out` has the proven shape.
fn reason(out: &str) -> String {
    let mut v: Value = serde_json::from_str(out).unwrap_or_else(|e| panic!("{e}: {out:?}"));
    let slot = &mut v["hookSpecificOutput"]["permissionDecisionReason"];
    let why = slot.as_str().unwrap_or("").to_owned();
    *slot = json!("");
    assert_eq!(v, proven_shape(), "{out}");
    assert!(!why.is_empty(), "{out}");
    why
}

/// The two refusals the real Muse run saw work have the shape every refusal here is held to (`reason`): E8, a delete
/// outside the project (the payload of tests/hook_outside.rs), and E5 case 4, a delete of a database an app holds open
/// (the one of tests/hook_database.rs).
#[test]
fn real_engine_the_shape_held_to_is_the_one_the_two_refusals_the_real_run_saw_have() {
    let r = Rig::real("user-only-shape");
    fs::create_dir(r.cwd.join("data")).unwrap();
    let _app = BusyApp::start(&r.cwd.join("data/app.db"));
    started(&r);
    let out = judge(&r, "rm -rf ~/old-notes");
    assert!(reason(&out).contains("outside the project folder"), "{out}");
    let out = judge(&r, "rm -f data/app.db data/app.db-wal data/app.db-shm");
    assert!(reason(&out).contains("A database is in use"), "{out}");
}

/// `template` (`{engine}` stands for the engine's path, how the guide tells the AI to write `torpor`) is refused, and the
/// reason names the user's command, `sub`, with the engine's path as the reason says every `torpor`.
fn assert_refused(name: &str, template: &str, sub: &str) {
    let r = Rig::real(&format!("user-only-{name}"));
    let engine = r.engine.display().to_string();
    started(&r);
    let cmd = template.replace("{engine}", &engine);
    assert_eq!(
        reason(&judge(&r, &cmd)),
        format!("Torpor: only the user can do this, in their own terminal ({engine} {sub})."),
        "{cmd}"
    );
}

macro_rules! refused {
    ($($name:ident: $cmd:expr => $sub:expr;)*) => {$(
        #[test]
        fn $name() {
            assert_refused(stringify!($name), $cmd, $sub);
        }
    )*};
}

refused! {
    real_engine_refuses_torpor_off: "torpor off" => "off";
    real_engine_refuses_the_engines_path_then_off: "{engine} off" => "off";
    real_engine_refuses_the_install_path_then_off: "~/.torpor/bin/torpor off" => "off";
    real_engine_refuses_sh_c_torpor_off: "sh -c 'torpor off'" => "off";
    real_engine_refuses_bash_c_torpor_gate_on: "bash -c \"torpor gate on\"" => "gate on";
    real_engine_refuses_cd_then_torpor_gate_off: "cd /tmp && torpor gate off" => "gate off";
    real_engine_refuses_sudo_torpor_forget: "sudo torpor forget" => "forget";
    real_engine_refuses_env_then_torpor_promise_drop: "env A=1 torpor promise drop 1" => "promise drop";
    real_engine_refuses_torpor_promise_ok: "torpor promise ok 1" => "promise ok";
    real_engine_refuses_torpor_promise_block: "torpor promise block 1" => "promise block";
    real_engine_refuses_torpor_promise_show_spec: "torpor promise show --spec 1" => "promise show --spec";
    real_engine_refuses_torpor_undo_force: "torpor undo --force" => "undo --force";
    real_engine_refuses_torpor_runner_install: "torpor runner install" => "runner install";
    real_engine_refuses_torpor_retain: "torpor retain" => "retain";
    real_engine_refuses_torpor_on: "torpor on" => "on";
}

/// The controls: what the guide gives the AI to run is not refused, so the table above is not "deny everything". The
/// guide is src/hook/db_note.rs `guide` (at every session start): "undo that / last change: torpor log -n 5, torpor
/// undo N." / "go back to before ...: torpor back --turn K or --id P" / "what broke ...: torpor find-break --promise ID
/// (or --check -)" / "lost database data: the database note's command", and undo and back with `--dry-run` first. The
/// database command is the note's (`db_text`, `back --id P --with-data`), taken from a real turn that rewrote one.
/// Each is tried as `torpor ...` and with the engine's path for `torpor`, which the guide says the AI may write.
#[test]
fn real_engine_the_commands_the_guide_gives_the_ai_are_not_refused() {
    let r = Rig::real("user-only-guide");
    let engine = r.engine.display().to_string();
    fs::write(r.cwd.join("notes.db"), b"SQLite format 3\0the user's notes").unwrap();
    let guide = context_of(&r.hook_ok("SessionStart", &r.fixture("SessionStart")));
    for want in [
        &format!("torpor = {engine}. "),
        "torpor log -n 5, torpor undo N.",
        "torpor back --turn K or --id P",
        "torpor find-break --promise ID (or --check -)",
        "the database note's command",
        "Undo/back only when asked: --dry-run first",
    ] {
        assert!(guide.contains(want), "{want:?} missing from {guide}");
    }
    real_turn(&r, 2, "rm -f notes.db && node server.js", || {
        fs::write(r.cwd.join("notes.db"), b"SQLite format 3\0").unwrap();
    });
    let mut next = r.fixture("UserPromptSubmit");
    next["turn_id"] = json!("00000000-0000-7000-8000-000000000003");
    let note = context_of(&r.hook_ok("UserPromptSubmit", &next));
    let (_, restore) = note.rsplit_once("The command: ").expect(&note);
    assert!(restore.starts_with(&engine), "{restore}");

    let given = [
        "torpor log -n 5",
        "torpor undo",
        "torpor undo 3",
        "torpor undo 3 --dry-run",
        "torpor back --turn 2",
        "torpor back --turn 2 --dry-run",
        "torpor back --id 5",
        "torpor back --id 5 --dry-run",
        "torpor find-break --promise 1",
        "torpor find-break --check -",
    ];
    let mut tried = vec![restore.to_owned(), format!("{restore} --dry-run")];
    for cmd in given {
        tried.push(cmd.to_owned());
        tried.push(cmd.replacen("torpor", &engine, 1));
    }
    for cmd in &tried {
        let out = judge(&r, cmd);
        assert_eq!(decision(&out), None, "{cmd}: {out}");
    }
    assert_eq!(r.problems(), "", "the engine answered every call");
}

/// KNOWN LIMIT, pinned. The judge reads `sh -c '...'` but not a script file: the AI writes `off.sh` holding `torpor off`
/// with the file tool and runs `sh off.sh`, and neither call is refused. The README must state this limit, and must
/// change if it is closed: when this test fails because `sh off.sh` is refused, change that README sentence in the same
/// commit.
#[test]
fn real_engine_known_limit_a_script_file_that_holds_torpor_off_is_not_refused() {
    let r = Rig::real("user-only-script");
    started(&r);
    let mut w = r.fixture("PreToolUse-write_file");
    w["tool_input"] = json!({"path": "off.sh", "content": "torpor off\n"});
    let out = r.hook_ok("PreToolUse", &w).out;
    assert_eq!(decision(&out), None, "writing the script: {out}");
    fs::write(r.cwd.join("off.sh"), "torpor off\n").unwrap();
    let out = judge(&r, "sh off.sh");
    assert_eq!(decision(&out), None, "running it: {out}");
    assert_eq!(r.problems(), "", "the engine answered both calls");
}

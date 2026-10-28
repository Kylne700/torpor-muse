//! The outside guard as Muse runs it (research/71c P1a): a delete outside the project is denied after the engine
//! answered, an engine deny stays the engine's, a command inside keeps the engine's answer, an ordinary command adds
//! nothing, and text that would delete the home folder gets one note per file per turn.

mod hook_common;
use hook_common::*;
use serde_json::{Value, json};

fn bash(r: &Rig, cmd: &str) -> Value {
    let mut p = r.fixture("PreToolUse-bash");
    p["tool_input"]["command"] = json!(cmd);
    p
}

fn decision(out: &str) -> (String, String) {
    let v: Value = serde_json::from_str(out).expect("a JSON answer");
    let h = &v["hookSpecificOutput"];
    let text = |k: &str| h[k].as_str().unwrap_or("").to_owned();
    (text("permissionDecision"), text("permissionDecisionReason"))
}

#[test]
fn a_delete_outside_the_project_is_denied_with_its_reason() {
    let r = Rig::new("outside-deny");
    let run = r.hook_ok("PreToolUse", &bash(&r, "rm -rf ~/old-notes"));
    let notes = r.dir.join("home/old-notes");
    let why = format!(
        "Torpor: this command was not run. It deletes or moves {}, outside the project folder. Torpor saves only the \
         project folder, so it could not bring that back. Ask the user first; if they want it, they can run it \
         themselves in their own terminal.",
        notes.display()
    );
    assert_eq!(decision(&run.out), ("deny".into(), why));
    assert_eq!(
        r.got("PreToolUse").len(),
        1,
        "the engine answered first (its save point)"
    );
    let two = r.hook_ok("PreToolUse", &bash(&r, "rm -rf ~/a ~/b && ls"));
    assert!(
        decision(&two.out).1.contains("/a and 1 more, outside"),
        "{}",
        two.out
    );
}

#[test]
fn an_engine_deny_stays_the_engines() {
    let r = Rig::new("outside-engine-deny");
    let engine = json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": "Torpor: that is where Torpor keeps its saves."
    }});
    r.set("out.PreToolUse", &engine.to_string());
    let run = r.hook_ok("PreToolUse", &bash(&r, "rm -rf ~/old-notes"));
    let (d, why) = decision(&run.out);
    assert_eq!(
        (d.as_str(), why.as_str()),
        ("deny", "Torpor: that is where Torpor keeps its saves.")
    );
}

#[test]
fn a_command_inside_the_project_keeps_the_engines_answer() {
    let r = Rig::new("outside-inside");
    let engine = json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "additionalContext": "Torpor: saved before this command."
    }});
    r.set("out.PreToolUse", &engine.to_string());
    for cmd in [
        "rm -rf build dist",
        "cd sub && rm -f x.o",
        "rm -rf /tmp/build-1",
        "mv a.txt b.txt",
    ] {
        let run = r.hook_ok("PreToolUse", &bash(&r, cmd));
        let out: Value = serde_json::from_str(&run.out).unwrap();
        assert_eq!(out, engine, "{cmd}");
    }
}

#[test]
fn an_ordinary_command_adds_nothing() {
    let r = Rig::new("outside-ordinary");
    for cmd in [
        "npm test",
        "cargo build --release",
        "git status",
        "rm -rf node_modules",
        "ls ~",
    ] {
        assert_eq!(r.hook_ok("PreToolUse", &bash(&r, cmd)).out, "", "{cmd}");
    }
}

#[test]
fn text_that_would_delete_the_home_folder_is_told_once_per_file_per_turn() {
    let r = Rig::new("outside-text");
    let mut w = r.fixture("PreToolUse-write_file");
    w["tool_input"] = json!({"path": "tests/t.rs", "content": "fn t() { run(\"20; rm -rf ~\"); }"});
    let line = "Torpor: the text written to tests/t.rs holds `rm -rf ~`; if that ever runs, it deletes the user's home \
                folder, and Torpor cannot bring that back.";
    let first = r.hook_ok("PreToolUse", &w).out;
    assert_eq!(note_parts(&first), (String::new(), line.to_owned()));
    assert_eq!(
        r.hook_ok("PreToolUse", &w).out,
        "",
        "once per file per turn"
    );
    let mut e = r.fixture("PreToolUse-edit_file");
    e["tool_input"] =
        json!({"path": "x.sh", "find": "a", "replace": "sudo rm -rf --no-preserve-root /"});
    let (_, said) = note_parts(&r.hook_ok("PreToolUse", &e).out);
    assert!(
        said.contains("deletes every file the user can delete on this computer"),
        "{said}"
    );
    w["turn_id"] = json!("00000000-0000-7000-8000-000000000003");
    assert_eq!(
        note_parts(&r.hook_ok("PreToolUse", &w).out).1,
        line,
        "a new turn"
    );
    let mut plain = w.clone();
    plain["tool_input"]["content"] = json!("rm -rf build");
    assert_eq!(r.hook_ok("PreToolUse", &plain).out, "");
}

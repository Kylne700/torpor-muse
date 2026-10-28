//! The one Stop block of a turn against the real engine: a broken check named with its turn, a deleted file told.

mod hook_common;

use hook_common::*;
use serde_json::{Value, json};
use std::fs;

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
    let line = said(&bad);
    assert!(
        line.contains(
            "Torpor: check #1 (proof page answers) failed after this turn (3 of 3 runs; "
        ) && line.contains("). (ask the AI: torpor promise show 1)")
            && !line.contains("shadow mode")
            && !line.contains("Torpor: promise #"),
        "the user's line says check, has no shadow sentence, and the command is for the AI: {bad}"
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

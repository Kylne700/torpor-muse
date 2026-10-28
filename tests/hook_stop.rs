//! The one Stop block of a turn against a fake engine: a broken check, a deleted file, a database, a full disk.

mod hook_common;

use hook_common::*;
use serde_json::{Value, json};
use std::fs;

// ---------------------------------------------------------------- F4 items 2 and 3: the one Stop block of a turn

const SHADOW_FAIL: &str = "Torpor: promise #1 (proof page answers) failed after this turn (3 of 3 runs; it passed on the version from the end of turn 4). It would have stopped the AI, but it is in shadow mode. torpor promise show 1";
const ADVISORY_FAIL: &str = "Torpor: promise #2 (login works) failed after this turn (3 of 3 runs; it passed before). The AI was not stopped. torpor promise show 2";

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
    // the user's line is the engine's own, with the failure in it, said as a check and without the shadow sentence
    assert!(
        said(&first).starts_with("Torpor: save point 31")
            && said(&first).contains("check #1 (proof page answers) failed after this turn")
            && !said(&first).contains("shadow mode"),
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

/// A Stop at which the engine says it is not saving. A disk that is almost full is told to the user alone, once per
/// session; the engine's other reasons are about Claude Code's own settings (Muse's hooks save without them), and after
/// `torpor off` the user knows: neither is told.
#[test]
fn a_full_disk_is_told_to_the_user_once_per_session_and_no_other_reason_for_not_saving_is() {
    let r = Rig::new("f4-paused");
    started(&r);
    let status = |off: bool, problems: &[&str]| {
        let s = json!({"saving": false, "off": off, "problems": problems, "last_turn": {"n": 5}});
        r.set("out.status", &s.to_string());
    };
    let settings = "Hooks are turned off by disableAllHooks in /srv/work/settings.json, so nothing is being saved.";
    let disk = "Save points are paused: the disk is almost full (0.4 GB free).";
    let told = "Torpor is not saving this folder: Save points are paused: the disk is almost full (0.4 GB free).";
    engine_stop(&r, &[SAVED]);
    status(
        false,
        &[settings, "No Torpor hook has run in this folder yet."],
    );
    let v = stop_of(&r, 30);
    assert!(
        !said(&v).contains("not saving") && v["hookSpecificOutput"].is_null(),
        "{v}"
    );
    status(true, &[disk]);
    let v = stop_of(&r, 31);
    assert!(!said(&v).contains("not saving"), "off: {v}");
    // the disk, among other reasons: the user's line goes first, and the engine's own line stays
    status(false, &[settings, disk]);
    let v = stop_of(&r, 32);
    assert!(
        said(&v).starts_with(&format!("{told} — Torpor: save point 31")) && blocked(&v).is_none(),
        "{v}"
    );
    assert!(v["hookSpecificOutput"].is_null(), "the user alone: {v}");
    // the same session is not told again, another one is
    assert!(!said(&stop_of(&r, 33)).contains("not saving"));
    let mut other = r.fixture("Stop");
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000aa");
    other["turn_id"] = json!("00000000-0000-7000-8000-0000000000ab");
    let v: Value = serde_json::from_str(&r.hook_ok("Stop", &other).out).unwrap_or(Value::Null);
    assert!(said(&v).starts_with(told), "{v}");
}

/// F4 item 3. A turn that deleted a file that existed before it, or deleted a database, gets the same one Stop
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
        "This turn deleted data/app.db (database files). Tell the user in one plain sentence",
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

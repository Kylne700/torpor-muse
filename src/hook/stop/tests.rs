//! Tests of the one Stop block's words: the engine's fail lines, the rows of a turn, the save points, the reason.

use super::*;
use serde_json::json;

// the engine's own lines (gate.rs fail_line, flaky 443, set aside 587, "stopped for it already" 690, cut short 717)
const SHADOW: &str = "Torpor: promise #1 (proof page answers) failed after this turn (3 of 3 runs; it passed on the kept version (save point 3)). It would have stopped the AI, but it is in shadow mode. torpor promise show 1";
const ADVISORY: &str = "Torpor: promise #2 (login works) failed after this turn (3 of 3 runs; it passed on the version from the end of turn 4). The AI was not stopped. torpor promise show 2";
const OTHERS: [&str; 6] = [
    "Torpor: promise #2 (api answers) passed and failed on the same version, so it is flaky and advisory only. torpor promise show 2",
    "Torpor: promise #2 (api answers) fails on the version that passed before, so its check cannot be trusted. It is set aside. torpor promise show 2",
    "Torpor: promise #2 (api answers) still fails. The AI was stopped for it already and nothing changed since, so the turn is allowed to end. torpor promise show 2",
    "Torpor: promise checks were cut short at the last 3 turn ends. torpor status",
    "Torpor: promise #3 failed after turn 5. Run torpor check 3.",
    "Torpor: new promise #1 \"proof page answers\" (shadow), from your words at turn 1. Drop it: torpor promise drop 1",
];

fn fail(id: u32, name: &str, passed_on: &str) -> Fail {
    Fail {
        id,
        name: name.into(),
        passed_on: passed_on.into(),
    }
}

#[test]
fn the_engines_fail_lines_are_read_and_its_other_promise_lines_are_not() {
    let msg = format!(
        "Torpor: save point 7 · turn 2 changed 1 file\n{SHADOW}\n  {ADVISORY}  \n{}",
        OTHERS.join("\n")
    );
    assert_eq!(
        fails(&msg),
        [
            fail(
                1,
                "proof page answers",
                "it passed on the kept version (save point 3)"
            ),
            fail(
                2,
                "login works",
                "it passed on the version from the end of turn 4"
            ),
        ]
    );
    for l in OTHERS {
        assert_eq!(fails(l), [], "{l}");
    }
    assert_eq!(fails(""), []);
    // words with brackets in them, and words that look like the engine's own text
    let odd = "Torpor: promise #5 (a (b) failed after this turn (x) c) failed after this turn (3 of 3 runs; it passed before). The AI was not stopped. torpor promise show 5";
    assert_eq!(
        fails(odd),
        [fail(
            5,
            "a (b) failed after this turn (x) c",
            "it passed before"
        )]
    );
    // a line the engine words differently still gives the promise, with nothing known about where it passed
    let drift =
        "Torpor: promise #6 (w) failed after this turn (3 of 3 runs). torpor promise show 6";
    assert_eq!(fails(drift), [fail(6, "w", "")]);
}

#[test]
fn the_engines_own_record_names_the_turn_only_when_one_turn_is_to_blame() {
    let end4 = "it passed on the version from the end of turn 4";
    assert!(broke_in(end4, 5, Some(30)));
    assert!(!broke_in(end4, 6, Some(30)), "turn 5 went by: two to blame");
    assert!(!broke_in(end4, 4, Some(30)));
    assert!(broke_in(end4, 5, None), "the turn number is enough here");
    let start5 = "it passed on the version from the start of turn 5";
    assert!(broke_in(start5, 5, Some(30)));
    assert!(!broke_in(start5, 6, Some(30)));
    // a kept version or a plain save point: this turn only if it was saved since the turn began
    let kept = "it passed on the kept version (save point 31)";
    assert!(broke_in(kept, 9, Some(30)) && broke_in(kept, 9, Some(31)));
    assert!(!broke_in(kept, 9, Some(32)));
    assert!(!broke_in(kept, 9, None));
    assert!(broke_in("it passed on save point 40", 9, Some(30)));
    // no record, or none this reads: no turn
    assert!(!broke_in("it passed before", 9, Some(30)));
    assert!(!broke_in("", 9, Some(30)));
    assert!(!broke_in(
        "it passed on the version from the end of turn 18446744073709551615",
        0,
        None
    ));
}

fn row(path: &str, class: &str, change: &str) -> Value {
    json!({"path": path, "class": class, "change": change})
}

#[test]
fn rows_say_what_was_deleted_and_which_databases_are_at_risk() {
    let show = json!({"files": [
        row("a.txt", "shell", "deleted"),
        row("new.txt", "shell", "new"),
        row("edited.txt", "edit", "changed"),
        row("theirs.txt", "outside", "deleted"),
        row("shared.txt", "shared", "deleted"),
        row("sub/b.txt", "subagent", "deleted"),
        row("moded.txt", "shell", "mode"),
        row("kept.DB", "shell", "changed"),
        row("gone.sqlite", "shell", "deleted"),
        row("fresh.db", "shell", "new"),
        row("theirs.db", "outside", "changed"),
        row("shared.db", "shared", "changed"),
        row("mode.sqlite3", "shell", "mode"),
        row("busy-now.db", "shell", "not saved"),
        row("busy-gone.db", "shell", "not saved"),
        {"path": "no-class.db", "change": "deleted"},
    ]});
    let r = rows(&show, |p| p == "busy-gone.db");
    assert_eq!(r.deleted, ["a.txt", "sub/b.txt"]);
    assert_eq!(
        r.dbs,
        [
            "kept.DB",
            "gone.sqlite",
            "shared.db",
            "mode.sqlite3",
            "busy-now.db",
            "busy-gone.db",
            "no-class.db"
        ]
    );
    assert_eq!(
        r.risky,
        ["gone.sqlite", "busy-gone.db", "no-class.db"],
        "gone data of the AI's own; not a changed database, not one still in use, not a mode change"
    );
    assert_eq!(r.lost, ["busy-gone.db"]);
    assert_eq!(rows(&json!({}), |_| true), Rows::default());
    assert_eq!(rows(&json!({"files": []}), |_| true), Rows::default());
}

#[test]
fn a_moved_file_is_not_a_deleted_one_but_only_when_the_engines_counts_say_so() {
    let del = |p: &str, class: &str, n: Option<u64>| json!({"path": p, "class": class, "change": "deleted", "removed": n});
    let new = |p: &str, class: &str, n: Option<u64>| json!({"path": p, "class": class, "change": "new", "added": n});
    let told = |files: Vec<Value>| rows(&json!({ "files": files }), |_| false).deleted;
    let none: [&str; 0] = [];
    // the engine's rows for `mv src/old.ts src/new.ts`: same lines, same kind of file
    assert_eq!(
        told(vec![
            new("src/new.ts", "shell", Some(12)),
            del("src/old.ts", "shell", Some(12))
        ]),
        none
    );
    // another number of lines, another kind of file, or nothing counted: deleted
    assert_eq!(
        told(vec![
            del("a.ts", "shell", Some(12)),
            new("b.ts", "shell", Some(13))
        ]),
        ["a.ts"]
    );
    assert_eq!(
        told(vec![
            del("a.ts", "shell", Some(12)),
            new("b.js", "shell", Some(12))
        ]),
        ["a.ts"]
    );
    assert_eq!(
        told(vec![
            del("a.png", "shell", None),
            new("b.png", "shell", None)
        ]),
        ["a.png"]
    );
    assert_eq!(told(vec![del("a.ts", "shell", Some(3))]), ["a.ts"]);
    // one new file stands for one deleted file, not for two
    assert_eq!(
        told(vec![
            del("a.ts", "shell", Some(5)),
            del("b.ts", "edit", Some(5)),
            new("c.ts", "shell", Some(5))
        ]),
        ["b.ts"]
    );
    // a new file made by another session hides nothing
    assert_eq!(
        told(vec![
            del("a.ts", "shell", Some(5)),
            new("b.ts", "shared", Some(5))
        ]),
        ["a.ts"]
    );
    assert_eq!(
        told(vec![
            del("a.ts", "shell", Some(5)),
            new("b.ts", "outside", Some(5))
        ]),
        ["a.ts"]
    );
}

#[test]
fn the_save_points_a_turn_took_before_its_commands_are_found() {
    let show = json!({"points": [
        {"id": 3, "kind": "start"}, {"id": 9, "kind": "pre_bash"}, {"id": 5, "kind": "pre_bash"},
        {"id": 6, "kind": "tool"}, {"id": 12, "kind": "end"}
    ]});
    assert_eq!(pre_points(&show), [5, 9]);
    assert_eq!(pre_points(&json!({})), Vec::<u64>::new());
}

#[test]
fn a_dry_run_counts_only_when_it_lists_every_database_as_brought_back() {
    let need = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let ok = "Would go back to save point 5 (before rm, today 20:22).\nWould change 1 file: 1 brought back (data/app.db).\nNothing was changed (dry run).";
    assert!(brings_back(ok, &need(&["data/app.db"])));
    assert!(!brings_back(ok, &need(&["data/app.db", "other.db"])));
    assert!(!brings_back(ok, &need(&["app.db"])), "the whole path");
    let busy = "Would go back to save point 3 (start of turn 2).\nNothing needed changing.\nNot touched: data/app.db (nothing to bring back, its database was busy).";
    assert!(!brings_back(busy, &need(&["data/app.db"])));
    let many = "Would change 4 files: 2 restored, 1 removed (n.txt), 1 brought back (a.db, b.db).";
    assert!(brings_back(many, &need(&["a.db", "b.db"])));
    // a list the engine cut hides names: the one next to "and N more" is not proven
    let cut = "Would change 5 files: 5 brought back (a.db, b.db, c.db and 2 more).";
    assert!(brings_back(cut, &need(&["a.db", "b.db"])));
    assert!(!brings_back(cut, &need(&["c.db"])) && !brings_back(cut, &need(&["d.db"])));
    assert!(!brings_back("", &need(&["a.db"])));
}

#[test]
fn the_latest_save_point_that_has_the_database_is_named() {
    let need = vec!["data/app.db".to_string()];
    let yes = "Would change 1 file: 1 brought back (data/app.db).";
    let no = "Nothing needed changing.\nNot touched: data/app.db (nothing to bring back, its database was busy).";
    let asked = std::cell::RefCell::new(Vec::new());
    let dry = |good: &'static [u64]| {
        let asked = &asked;
        move |p: u64| {
            asked.borrow_mut().push(p);
            Some(if good.contains(&p) { yes } else { no }.to_string())
        }
    };
    assert_eq!(restoring_point(&[4, 5, 8], &need, dry(&[5, 8])), Some(8));
    assert_eq!(
        *asked.borrow(),
        [8],
        "the latest is asked first, and no more once one has it"
    );
    asked.borrow_mut().clear();
    assert_eq!(restoring_point(&[4, 5, 8], &need, dry(&[4])), Some(4));
    assert_eq!(*asked.borrow(), [8, 5, 4]);
    asked.borrow_mut().clear();
    assert_eq!(
        restoring_point(&[1, 2, 4, 5, 8], &need, dry(&[1])),
        None,
        "three tries"
    );
    assert_eq!(*asked.borrow(), [8, 5, 4]);
    assert_eq!(restoring_point(&[], &need, dry(&[1])), None);
    // a dry run that could not run (the engine refused or timed out) is a point that does not have it
    assert_eq!(restoring_point(&[7], &need, |_| None), None);
}

fn broke(id: u32, name: &str, turn: Option<u64>) -> Broke {
    Broke {
        id,
        name: name.into(),
        turn,
    }
}

#[test]
fn the_block_names_the_promise_the_turn_and_what_to_do_about_it() {
    let e = "/opt/torpor";
    let r = reason(e, &[broke(3, "login works", Some(5))], &Risk::default()).unwrap();
    for want in [
        "[Torpor] Before you finish:",
        "Promise #3 \"login works\" (turn 5 broke it) failed after this turn.",
        "Fix the app, not the check, and do not special-case test values.",
        "Run `/opt/torpor check 3` before you say it is fixed.",
        "run `/opt/torpor promise dispute 3 \"<why>\"` and tell the user.",
    ] {
        assert!(r.contains(want), "{want}: {r}");
    }
    assert!(!r.contains("find-break") && !r.contains("deleted"), "{r}");
    // not one turn: the AI is told how to find it
    let r = reason(e, &[broke(3, "login works", None)], &Risk::default()).unwrap();
    assert!(
        r.contains(
            "(Torpor cannot name one turn that broke it: run `/opt/torpor find-break --promise 3`)"
        ),
        "{r}"
    );
    // several: each with its own reason, one shared instruction, at most 3 listed
    let many: Vec<Broke> = (1..=5)
        .map(|i| broke(i, "w", (i == 2).then_some(7)))
        .collect();
    let r = reason(e, &many, &Risk::default()).unwrap();
    assert!(
        r.contains("Promises #1 \"w\" (Torpor cannot name")
            && r.contains("#2 \"w\" (turn 7 broke it)"),
        "{r}"
    );
    assert!(r.contains(" and 2 more failed after this turn."), "{r}");
    assert!(
        !r.contains("#4") && r.contains("check ID") && r.contains("dispute ID"),
        "{r}"
    );
    // words that could end the quotation, or hold control characters, are made harmless
    let r = reason(
        e,
        &[broke(1, "say \"hi\" `rm`\nnow", Some(2))],
        &Risk::default(),
    )
    .unwrap();
    assert!(r.contains("#1 \"say 'hi' 'rm' now\" (turn 2"), "{r}");
    assert_eq!(reason(e, &[], &Risk::default()), None);
}

#[test]
fn a_risky_turn_is_told_in_one_plain_sentence_and_a_database_names_its_command() {
    let e = "/opt/torpor";
    let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let r = reason(
        e,
        &[],
        &Risk {
            deleted: names(&["a.txt", "b.txt"]),
            ..Risk::default()
        },
    )
    .unwrap();
    assert!(r.starts_with("[Torpor] Before you finish:\n- This turn deleted a.txt, b.txt (files that existed before this turn). Tell the user in one plain sentence what was deleted or changed and that they can get it back by asking you. Do not undo anything unless they ask."), "{r}");
    assert!(!r.contains("back --id") && !r.contains("Promise"), "{r}");
    let five = names(&["1", "2", "3", "4", "5"]);
    let r = reason(
        e,
        &[],
        &Risk {
            deleted: five,
            ..Risk::default()
        },
    )
    .unwrap();
    assert!(r.contains("deleted 1, 2, 3 and 2 more (files"), "{r}");
    // a database turn: the command with the save point, the app stopped first, nothing run unasked
    let db = Risk {
        dbs: names(&["data/app.db"]),
        back: Some(5),
        ..Risk::default()
    };
    let r = reason(e, &[], &db).unwrap();
    for want in [
        "This turn deleted data/app.db (database files).",
        "stop the app that uses it first, then run `/opt/torpor back --id 5 --with-data`",
        "so later changes are undone too",
        "Run it only if the user asks; if the dry run brings back only what was deleted, that ask is enough.",
    ] {
        assert!(r.contains(want), "{want}: {r}");
    }
    // a database no save point has: no promise of a way back
    let lost = Risk {
        dbs: names(&["x.db"]),
        lost: names(&["x.db"]),
        ..Risk::default()
    };
    let r = reason(e, &[], &lost).unwrap();
    assert!(!r.contains("back --id"), "{r}");
    assert!(r.contains("Torpor could not save x.db (it was in use), so it may not be able to bring it back: tell the user so."), "{r}");
    // a broken check and a risky turn: one reason with both
    let both = reason(
        e,
        &[broke(3, "w", Some(5))],
        &Risk {
            deleted: names(&["a.txt"]),
            ..Risk::default()
        },
    )
    .unwrap();
    assert_eq!(both.matches("\n- ").count(), 2, "{both}");
    assert!(both.find("Promise #3").unwrap() < both.find("This turn deleted").unwrap());
    assert!(both.chars().count() < 1200, "{both}");
    assert_eq!(reason(e, &[], &Risk::default()), None);
}

/// The L4 proof: asked to undo the delete, the model ran the dry run, saw that only the deleted database would come back,
/// and still asked whether to go on. The block says that in this case the user's ask is enough.
#[test]
fn the_database_sentence_takes_the_ask_for_enough_when_only_what_was_deleted_comes_back() {
    let db = Risk {
        dbs: vec!["data/app.db".to_string()],
        back: Some(5),
        ..Risk::default()
    };
    let r = reason("/opt/torpor", &[], &db).unwrap();
    assert!(
        r.ends_with("(it puts every file back to that point, so later changes are undone too; use --dry-run first). Run it only if the user asks; if the dry run brings back only what was deleted, that ask is enough."),
        "{r}"
    );
    assert_eq!(r.matches("only if the user asks").count(), 1, "{r}");
}

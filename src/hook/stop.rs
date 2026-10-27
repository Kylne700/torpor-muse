//! F4 items 2 and 3: the words of the one Stop block that `hook::stop_note` gives when the turn that just ended broke a
//! saved check or did something risky. This file reads the engine's own text and writes ours; it starts nothing.

use crate::suggest::scrub;
use serde_json::Value;

/// Names listed in the block before it says "and N more".
const LIST: usize = 3;
/// The longest a promise's words or a path may be in the block.
const NAME: usize = 60;
/// Save points tried (the latest first) when looking for one that has a database that was in use.
const TRIES: usize = 3;

/// A promise the engine's Stop line says failed after the turn that just ended.
#[derive(Debug, PartialEq)]
pub struct Fail {
    pub id: u32,
    /// Its words, as the engine prints them.
    pub name: String,
    /// What the engine says about the version it last passed on: `it passed on the version from the end of turn 4`.
    pub passed_on: String,
}

/// The fail lines of the engine's Stop `systemMessage` (gate.rs `fail_line`, advisory or shadow):
/// `Torpor: promise #3 (name) failed after this turn (3 of 3 runs; it passed on ...). The AI was not stopped. torpor
/// promise show 3`. The lines for a flaky promise, a set-aside check, "stopped for it already" and "checks cut short"
/// read differently and are not matched, so they give no block. There is no machine-readable form of this in the
/// engine (`status --json` has only totals), so the line is the source.
pub fn fails(system_message: &str) -> Vec<Fail> {
    const MID: &str = ") failed after this turn (";
    system_message
        .lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix("Torpor: promise #")?;
            let (id, rest) = rest.split_once(" (")?;
            // the last MID: the words are the user's and may hold anything, the rest of the line is the engine's
            let (name, rest) = rest.rsplit_once(MID)?;
            let passed_on = rest
                .split_once("; ")
                .and_then(|(_, p)| p.split_once("). "))
                .map_or("", |(p, _)| p);
            Some(Fail {
                id: id.parse().ok()?,
                name: name.into(),
                passed_on: passed_on.into(),
            })
        })
        .collect()
}

/// True when the engine's own record puts the break in turn `n` (its first save point is `start`): the check last passed
/// on the end of turn n-1, on the start of turn n, or on a version saved since turn n began. Anything else (it passed
/// before, or on an older version) leaves several turns to blame.
pub fn broke_in(passed_on: &str, n: u64, start: Option<u64>) -> bool {
    let num = |key: &str| -> Option<u64> {
        let digits: String = passed_on
            .split_once(key)?
            .1
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    };
    if let Some(k) = num("the version from the end of turn ") {
        return k.checked_add(1) == Some(n);
    }
    if let Some(k) = num("the version from the start of turn ") {
        return k == n;
    }
    num("save point ")
        .zip(start)
        .is_some_and(|(p, start)| p >= start)
}

/// The engine's own test for a database file (its scan.rs): .db, .sqlite, .sqlite3, any case.
pub fn is_database(path: &str) -> bool {
    let p = path.to_lowercase();
    [".db", ".sqlite", ".sqlite3"]
        .iter()
        .any(|x| p.ends_with(x))
}

/// What `show N --json` says the turn did, sorted for the user's line and for the block.
#[derive(Debug, Default, PartialEq)]
pub struct Rows {
    /// Files that existed before the turn, were the AI's to change (not changed by another session or by the user) and
    /// are gone, databases apart.
    pub deleted: Vec<String>,
    /// Every database file the turn changed or deleted that was not made by someone else: the user's line and the kept
    /// database turns (L3 N1) name these.
    pub dbs: Vec<String>,
    /// Of those, the ones whose data changed or is gone: the block names these. A database that was in use at one end
    /// of the turn and is still there is not one (Torpor knows nothing about its data).
    pub risky: Vec<String>,
    /// Of those, the ones that were in use when the turn began and are gone now: the start of the turn has no copy.
    pub lost: Vec<String>,
}

/// `gone(path)`: the path no longer exists in the project folder.
pub fn rows(show: &Value, gone: impl Fn(&str) -> bool) -> Rows {
    let mut r = Rows::default();
    for f in show["files"].as_array().into_iter().flatten() {
        let Some(path) = f["path"].as_str() else {
            continue;
        };
        let (class, change) = (f["class"].as_str(), f["change"].as_str());
        if class == Some("outside") {
            continue;
        }
        let own = class != Some("shared"); // "shared": another session was working there
        if is_database(path) {
            if change != Some("new") {
                r.dbs.push(path.into());
            }
            match change {
                Some("deleted" | "changed") if own => r.risky.push(path.into()),
                Some("not saved") if own && gone(path) => {
                    r.risky.push(path.into());
                    r.lost.push(path.into());
                }
                _ => {}
            }
        } else if change == Some("deleted") && own {
            r.deleted.push(path.into());
        }
    }
    r
}

/// The ids of the save points the turn took before its commands (`pre_bash`), oldest first.
pub fn pre_points(show: &Value) -> Vec<u64> {
    let mut ids: Vec<u64> = show["points"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["kind"] == "pre_bash")
        .filter_map(|p| p["id"].as_u64())
        .collect();
    ids.sort_unstable();
    ids
}

/// True when the text of a dry run of `back --with-data` lists every path of `need` as brought back (its summary line
/// ends with `1 brought back (data/app.db).`). A list the engine cut (`a, b and 2 more`) proves nothing for the names it
/// hides, so those do not count.
pub fn brings_back(dry_run: &str, need: &[String]) -> bool {
    let listed = dry_run
        .lines()
        .find_map(|l| l.split_once("brought back ("))
        .and_then(|(_, r)| r.trim_end().strip_suffix(")."));
    listed.is_some_and(|l| need.iter().all(|p| l.split(", ").any(|x| x == p)))
}

/// The save point to name for databases that were in use when the turn began and are gone now: the latest of the points
/// the turn took before its commands (`pres`, oldest first; the app may have stopped between two of them) whose dry run
/// `dry(point)` brings every path of `need` back. None = no such point.
/// shortcut: the latest 3 points only, and a database turn that also changed other databases gets this one point for all
/// of them; upgrade when a real turn needs more.
pub fn restoring_point(
    pres: &[u64],
    need: &[String],
    mut dry: impl FnMut(u64) -> Option<String>,
) -> Option<u64> {
    pres.iter()
        .rev()
        .take(TRIES)
        .copied()
        .find(|&p| dry(p).is_some_and(|t| brings_back(&t, need)))
}

/// A promise to tell the AI about: its number, its words, and the turn that broke it when the engine's records say.
#[derive(Debug, PartialEq)]
pub struct Broke {
    pub id: u32,
    pub name: String,
    pub turn: Option<u64>,
}

/// What the turn did that the user must hear from the AI.
#[derive(Debug, Default, PartialEq)]
pub struct Risk {
    pub deleted: Vec<String>,
    /// Databases whose data changed or is gone.
    pub dbs: Vec<String>,
    /// The save point whose `back --id P --with-data` puts them back (None: no command to give).
    pub back: Option<u64>,
    /// Databases no save point has (they were in use).
    pub lost: Vec<String>,
}

/// A name for the block: no control characters and no key-like string (`scrub`), and nothing that would end the quotation
/// or the command it stands in.
fn name(s: &str) -> String {
    scrub(s, NAME).replace(['"', '`'], "'")
}

/// `a, b, c and 2 more`
fn list(names: &[String]) -> String {
    let shown: Vec<String> = names.iter().take(LIST).map(|n| name(n)).collect();
    match names.len().saturating_sub(LIST) {
        0 => shown.join(", "),
        k => format!("{} and {k} more", shown.join(", ")),
    }
}

/// The reason Muse gives the model with the one Stop block (`cmd` = how the model writes `torpor`, see `engine_cmd`;
/// this text does not pass `schema::sanitise`). None = nothing to say.
pub fn reason(cmd: &str, broke: &[Broke], risk: &Risk) -> Option<String> {
    let mut parts = Vec::new();
    if !broke.is_empty() {
        let one = broke.len() == 1;
        let id = if one {
            broke[0].id.to_string()
        } else {
            "ID".into()
        };
        let items: Vec<String> = broke
            .iter()
            .take(LIST)
            .map(|b| {
                let found = match b.turn {
                    Some(n) => format!("turn {n} broke it"),
                    None => format!(
                        "Torpor cannot name one turn that broke it: run `{cmd} find-break --promise {}`",
                        b.id
                    ),
                };
                let words = if b.name.is_empty() {
                    String::new()
                } else {
                    format!(" \"{}\"", name(&b.name))
                };
                format!("#{}{words} ({found})", b.id)
            })
            .collect();
        let more = match broke.len().saturating_sub(LIST) {
            0 => String::new(),
            k => format!(" and {k} more"),
        };
        parts.push(format!(
            "- Promise{} {}{more} failed after this turn. Fix the app, not the check, and do not special-case test \
             values. Run `{cmd} check {id}` before you say it is fixed. If the promise itself is wrong, run `{cmd} \
             promise dispute {id} \"<why>\"` and tell the user.{}",
            if one { "" } else { "s" },
            items.join(", "),
            if one {
                ""
            } else {
                " (ID is the number after #.)"
            }
        ));
    }
    let mut did = Vec::new();
    if !risk.deleted.is_empty() {
        did.push(format!(
            "deleted {} (files that existed before this turn)",
            list(&risk.deleted)
        ));
    }
    if !risk.dbs.is_empty() {
        did.push(format!(
            "changed or deleted {} (database files)",
            list(&risk.dbs)
        ));
    }
    if !did.is_empty() {
        let mut s = format!(
            "- This turn {}. Tell the user in one plain sentence what was deleted or changed and that they can get it \
             back by asking you. Do not undo anything unless they ask.",
            did.join(" and ")
        );
        if let Some(p) = risk.back {
            s.push_str(&format!(
                " To bring a database back, stop the app that uses it first, then run `{cmd} back --id {p} \
                 --with-data` (it puts every file back to that point, so later changes are undone too; use --dry-run \
                 first). Run it only if the user asks."
            ));
        }
        if !risk.lost.is_empty() {
            s.push_str(&format!(
                " Torpor could not save {} (it was in use), so it may not be able to bring it back: tell the user so.",
                list(&risk.lost)
            ));
        }
        parts.push(s);
    }
    (!parts.is_empty()).then(|| format!("[Torpor] Before you finish:\n{}", parts.join("\n")))
}

#[cfg(test)]
mod tests {
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
            ["kept.DB", "gone.sqlite", "busy-gone.db", "no-class.db"],
            "changed or gone data of the AI's own; not a database still in use, not a mode change"
        );
        assert_eq!(r.lost, ["busy-gone.db"]);
        assert_eq!(rows(&json!({}), |_| true), Rows::default());
        assert_eq!(rows(&json!({"files": []}), |_| true), Rows::default());
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
        let many =
            "Would change 4 files: 2 restored, 1 removed (n.txt), 1 brought back (a.db, b.db).";
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
            r.contains("(Torpor cannot name one turn that broke it: run `/opt/torpor find-break --promise 3`)"),
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
            "This turn changed or deleted data/app.db (database files).",
            "stop the app that uses it first, then run `/opt/torpor back --id 5 --with-data`",
            "so later changes are undone too",
            "Run it only if the user asks.",
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
}

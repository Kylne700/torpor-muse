//! F4 items 2 and 3, SN2 and SN3: the words of the Stop blocks that `hook::stop_note::stop_note` gives when the turn that just
//! ended broke a saved check or did something risky: the first block (`reason`), a second one (`again`) when the check
//! still fails at the next Stop of the turn, and the one line each block also writes to stderr (`flash`). This file reads the
//! engine's own text and writes ours; it starts nothing.

use super::busy::is_db_name;
use crate::suggest::scrub;
use serde_json::Value;
use std::path::Path;

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

/// What `show N --json` says the turn did, sorted for the user's line and for the block.
#[derive(Debug, Default, PartialEq)]
pub struct Rows {
    /// Files that existed before the turn, were the AI's to change (not changed by another session or by the user) and
    /// are gone, databases apart and moved files apart (see `rows`).
    pub deleted: Vec<String>,
    /// Every database file the turn changed or deleted that was not made by someone else: the user's line and the kept
    /// database turns (L3 N1) name these.
    pub dbs: Vec<String>,
    /// Of those, the ones that are gone: the block names these. A database that was changed (a migration, a write) or
    /// that was in use at one end of the turn and is still there is not one: Torpor knows nothing about its data, the AI
    /// did not lose it, and the user's line tells about it (F4c).
    pub risky: Vec<String>,
    /// Of those, the ones that were in use when the turn began and are gone now: the start of the turn has no copy.
    pub lost: Vec<String>,
}

/// `gone(path)`: the path no longer exists in the project folder.
pub fn rows(show: &Value, gone: impl Fn(&str) -> bool) -> Rows {
    let mut r = Rows::default();
    // the AI's own deleted and new files that are no databases, with the lines the engine counted in them (None: it
    // counted none, as for a binary file)
    let (mut deleted, mut fresh) = (Vec::new(), Vec::new());
    for f in show["files"].as_array().into_iter().flatten() {
        let Some(path) = f["path"].as_str() else {
            continue;
        };
        let (class, change) = (f["class"].as_str(), f["change"].as_str());
        if class == Some("outside") {
            continue;
        }
        let own = class != Some("shared"); // "shared": another session was working there
        if is_db_name(path) {
            if change != Some("new") {
                r.dbs.push(path.into());
            }
            match change {
                Some("deleted") if own => r.risky.push(path.into()),
                Some("not saved") if own && gone(path) => {
                    r.risky.push(path.into());
                    r.lost.push(path.into());
                }
                _ => {}
            }
        } else if own {
            match change {
                Some("deleted") => deleted.push((path, f["removed"].as_u64())),
                Some("new") => fresh.push((path, f["added"].as_u64())),
                _ => {}
            }
        }
    }
    // The engine shows a rename as a deleted file and a new one. shortcut: a deleted file counts as moved when a new
    // file of the same kind (extension) has the same number of lines, one new file for one deleted file, and one with no
    // count (binary, large) never does; a content hash would be exact and the engine gives none, so upgrade when it
    // gives a hash or says "renamed". A moved file that was edited in the same turn is still told as deleted.
    for (path, removed) in deleted {
        let moved = removed.and_then(|n| {
            fresh.iter().position(|&(p, added)| {
                added == Some(n) && Path::new(p).extension() == Path::new(path).extension()
            })
        });
        match moved {
            Some(i) => {
                fresh.remove(i);
            }
            None => r.deleted.push(path.into()),
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

/// The first line of the reason (`reason`, and `done::join` when the false-Done sentence is the only item).
pub(super) const HEAD: &str = "[Torpor] Before you finish:";

/// The reason Muse gives the model with the first Stop block of a turn (`cmd` = how the model writes `torpor`, see
/// `engine_cmd`; this text does not pass `schema::sanitise`). None = nothing to say.
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
             promise dispute {id} \"<why>\"` and tell the user. Your reply to the user must start with what broke, even \
             if the user asked for a specific reply.{}",
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
        did.push(format!("deleted {} (database files)", list(&risk.dbs)));
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
                 first). Run it only if the user asks; if the dry run brings back only what was deleted, that ask is \
                 enough."
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
    (!parts.is_empty()).then(|| format!("{HEAD}\n{}", parts.join("\n")))
}

/// The reason of the second Stop block of a turn (SN2): the first one did not make the reply tell the user (a reply format
/// the user asked for won over the note in 1 of 2 takes), the check still fails at this Stop (`fails` = the fail lines of
/// this Stop's engine text), and Muse keeps none of the note on the screen (the bar shows only the 5 s line of `flash`), so
/// the AI's reply is what stays for the user. The sentence to open that reply with is given whole. None = no check fails.
pub fn again(cmd: &str, fails: &[Fail]) -> Option<String> {
    let id = match fails {
        [] => return None,
        [f] => f.id.to_string(),
        _ => "ID".into(),
    };
    let items: Vec<String> = fails
        .iter()
        .take(LIST)
        .map(|f| match name(&f.name).as_str() {
            "" => format!("#{}", f.id),
            words => format!("#{} ({words})", f.id),
        })
        .collect();
    let more = match fails.len().saturating_sub(LIST) {
        0 => String::new(),
        k => format!(" and {k} more"),
    };
    let (what, still, is, ids) = if fails.len() == 1 {
        ("Check", "fails", "is", "")
    } else {
        ("Checks", "fail", "are", " (ID is the number after #)")
    };
    let list = format!("{what} {}{more}", items.join(", "));
    Some(format!(
        "{HEAD}\n- {list} still {still}, and only your reply reaches the user. Start your next reply with this sentence, \
         even if the user asked for a specific reply: \"{list} {is} failing; run `{cmd} check {id}` to see why{ids}.\""
    ))
}

/// The most characters of the line a Stop block also writes to stderr.
const FLASH: usize = 60;

/// The one line a Stop block also writes to stderr (SN3; `hook::run` prints it, exit 0). Muse Code 1.4.14 draws it in its
/// status label ("Hook blocked · Stop · <line>") for about 5 s and draws nothing else of Torpor's own, so it is what the user
/// sees without the AI. At most `FLASH` characters, no path and no command output. The label is cut at the right edge (the
/// room for the line is 51 characters at 80 columns and 71 at 100 with the echo model, SN3), so the order is the number and
/// the verdict, then the command that shows it, then the check's words last: a cut takes the words first. The line names the
/// failing check by number, else says what the turn deleted, else that a false Done was told.
/// `checks` = (id, words) of the failing checks, `gone` = (turn, files deleted), `claim` = a false Done; they win in that
/// order. None = nothing to say.
/// shortcut: the cap counts characters, not terminal columns (a wide character takes two); upgrade when a real name needs it.
pub fn flash(checks: &[(u32, &str)], gone: Option<(u64, usize)>, claim: bool) -> Option<String> {
    let line = match (checks, gone) {
        ([(id, words)], _) => {
            let head = format!("Torpor: check {id} fails · torpor check {id}");
            // the 3 characters are " (" and ")" around the words
            let room = FLASH.saturating_sub(head.chars().count() + 3);
            match fit(words, room) {
                Some(w) => format!("{head} ({w})"),
                None => head,
            }
        }
        ([], Some((turn, k))) => format!(
            "Torpor: turn {turn} deleted {k} file{} · torpor show {turn}",
            if k == 1 { "" } else { "s" }
        ),
        ([], None) if claim => "Torpor: the AI says it works, but the record disagrees".into(),
        ([], None) => return None,
        (many, _) => {
            // up to LIST ids, fewer when the numbers are long: the command matters more than the third id
            let n = many.len();
            let with = |k: usize| {
                let ids: Vec<String> = many.iter().take(k).map(|c| c.0.to_string()).collect();
                let head = format!("Torpor: {n} checks fail · torpor check --all");
                match k {
                    0 => head,
                    _ if k == n => format!("{head} ({})", ids.join(", ")),
                    _ => format!("{head} ({}...)", ids.join(", ")),
                }
            };
            (0..=LIST.min(n))
                .rev()
                .map(with)
                .find(|l| l.chars().count() <= FLASH)
                .unwrap_or_default()
        }
    };
    // the forms above fit; this keeps the cap whatever the numbers are
    Some(line.chars().take(FLASH).collect())
}

/// A check's words for the line: `name`, without any word that holds a path, at most `room` characters. A longer text is cut
/// after the last whole word that fits, then `...` (mid-word only when its first word is already too long). None = no
/// words, or no room for any.
fn fit(words: &str, room: usize) -> Option<String> {
    let words = name(words);
    let words = words
        .split_whitespace()
        .filter(|w| !w.contains(['/', '\\']))
        .collect::<Vec<_>>()
        .join(" ");
    if words.is_empty() {
        return None;
    }
    if words.chars().count() <= room {
        return Some(words);
    }
    let keep = room.saturating_sub(3);
    let cut: String = words.chars().take(keep).collect();
    // the cut may end exactly between two words, then nothing is dropped
    let cut = if words.chars().nth(keep) == Some(' ') {
        cut.as_str()
    } else {
        cut.rsplit_once(' ')
            .map_or(cut.as_str(), |(whole, _)| whole)
    };
    (!cut.is_empty()).then(|| format!("{cut}..."))
}

#[cfg(test)]
mod tests;

//! The guide and the kept database turns, told to the model at session start and at the next prompt after a new one.

use super::paths::engine_cmd;
use super::state;
use crate::{schema, suggest};
use std::path::Path;

/// What the model is told at the start of every session (F4): the plain words a user says, the Torpor command for each
/// (undo and back: a dry run first, and the user's ask is the yes), and the commands that are the user's own. At most
/// about 730 characters with the engine's path, which stands once for the word `torpor` in it. It goes in front of the
/// database note (`db_note`).
fn guide(bin: &Path) -> String {
    let at = engine_cmd(bin);
    let means = if at == "torpor" {
        String::new()
    } else {
        format!("torpor = {at}. ")
    };
    format!(
        "[Torpor guide: stays valid] {means}Undo/back only when asked: --dry-run first, tell them in one sentence what \
         it changes, then run it (their ask is the yes). Ask first only if the dry run also undoes other work or \
         overwrites an existing database.\n\
         undo that / last change: torpor log -n 5, torpor undo N.\n\
         go back to before ...: torpor back --turn K or --id P; tell them the printed torpor back --id N undoes it.\n\
         what broke ...: torpor find-break --promise ID (or --check -).\n\
         lost database data: the database note's command, app stopped first.\n\
         Never run off, on, gate, retain, forget, undo --force, promise ok/drop/block/show --spec, runner install: the \
         user's own."
    )
}

/// The database note for `turns` (newest first): one command per turn, see `db_note`.
fn db_text(bin: &Path, turns: &[state::DbTurn]) -> String {
    let mut note = String::from(
        "[Torpor database note: later Torpor notes do not replace this one] `torpor undo` and a plain `torpor \
         back` leave existing database files as they are. Each command below brings a database's old data back, \
         even if the app made a fresh database since. Run one only when the user asks for it.",
    );
    for t in turns {
        let (n, p) = (t.n, t.p);
        let s = if t.files.len() > 1 { "s" } else { "" };
        note.push_str(&format!(
            "\nTurn {n} changed {} (database file{s}). To get it back as it was before turn {n}, stop the app first, \
             then run the command below; it also puts every other file back to the start of turn {n}, so later \
             changes are undone too. The command: {} back --id {p} --with-data",
            suggest::scrub(&t.files.join(", "), 300), // the note must fit Muse's 12 KiB stdout limit
            engine_cmd(bin)
        ));
    }
    note
}

/// What the model is told about the database turns kept for its folder (`stop_note` keeps them, `state::db_add`): all of
/// them, each with its command (with the engine's path, see `engine_cmd`), once: at the start of the session (startup,
/// resume, clear, compact) or at the next prompt after a newer one; at the start the `guide` comes first. Both are added
/// to the engine's text even when that must be cut to make them fit (`with_note_fit`).
pub(super) fn db_note(
    dir: &Path,
    event: &str,
    sid: &str,
    cwd: &str,
    bin: &Path,
    body: Option<String>,
) -> Option<String> {
    if !matches!(event, "SessionStart" | "UserPromptSubmit") {
        return body;
    }
    let turns = if cwd.is_empty() {
        Vec::new()
    } else {
        state::db_turns(dir, cwd)
    };
    let told = state::with_session(dir, sid, |s| s.db_told).unwrap_or(0);
    let newest = turns.first().map(|t| t.n);
    let due = newest.is_some_and(|n| event == "SessionStart" || n > told);
    if !due && event != "SessionStart" {
        return body;
    }
    let note = if due {
        db_text(bin, &turns)
    } else {
        String::new()
    };
    // the guide first, the database note last; if they do not fit with the engine's text, that is what is cut
    let guide = if event == "SessionStart" {
        guide(bin)
    } else {
        String::new()
    };
    let context = match (guide.is_empty(), note.is_empty()) {
        (false, false) => format!("{guide}\n\n{note}"),
        (true, _) => note,
        _ => guide,
    };
    if context.is_empty() {
        return body;
    }
    match schema::with_note_fit(event, body.as_deref(), "", &context, false) {
        Some(m) => {
            if due {
                state::with_session(dir, sid, |s| s.db_told = newest.unwrap_or(0));
            }
            Some(m)
        }
        None => body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_guide_names_each_plain_word_command_and_the_ones_that_are_the_users() {
        // 62 characters of path: the guide stays within about 730 characters, and the path is said once
        let long = "/data/someone/with/a/rather/long/folder/name/for/engine/torpor";
        assert_eq!(long.len(), 62);
        let g = guide(Path::new(long));
        assert!(g.chars().count() <= 730, "{} characters", g.chars().count());
        assert_eq!(g.matches(long).count(), 1, "{g}");
        for want in [
            "torpor log -n 5",
            "torpor undo N.",
            "torpor back --turn K or --id P",
            "torpor find-break --promise ID (or --check -)",
            "the database note's command, app stopped first",
            "Undo/back only when asked: --dry-run first, tell them in one sentence",
            "Ask first only if the dry run also undoes other work or overwrites an existing database",
            "Never run off, on, gate, retain, forget, undo --force, promise ok/drop/block/show --spec, runner install",
        ] {
            assert!(g.contains(want), "{want:?} missing from {g}");
        }
        // an engine the model can only call by the bare name says nothing about a path
        let bare = guide(Path::new("torpor"));
        assert!(!bare.contains("torpor = "), "{bare}");
    }

    /// The L4 proof: the dry run was read as a rule for every command (a `keep` was held back), and asking first about a
    /// database made the model ask again before it brought back the database it had deleted.
    #[test]
    fn the_guide_scopes_the_dry_run_to_undo_and_back_and_takes_the_ask_for_the_yes() {
        let g = guide(Path::new("torpor"));
        // the dry run belongs to undo and back: every guide line that names it says so
        assert!(g.contains("--dry-run"), "{g}");
        for line in g.lines().filter(|l| l.contains("--dry-run")) {
            assert!(
                line.contains("Undo/back"),
                "a dry run outside undo and back: {line}"
            );
        }
        // the ask is the yes; asking again takes what the dry run shows (other work, a database that is still there), not
        // a database as such, so a deleted database that comes back alone is not asked about
        let rule = g.lines().next().unwrap();
        assert!(
            rule.contains("then run it (their ask is the yes)."),
            "{rule}"
        );
        assert!(
            rule.ends_with(" Ask first only if the dry run also undoes other work or overwrites an existing database."),
            "{rule}"
        );
        for old in [
            "a database or unnamed files",
            "Ask first if",
            "Only if the user asks",
        ] {
            assert!(!g.contains(old), "{old:?} is back in {g}");
        }
    }
}

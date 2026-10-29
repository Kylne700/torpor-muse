//! What the hook reads back from the writer: open gaps, the last result, and text made safe to show.

use super::state::{Gap, Last};
use super::{client, repo, state};
use std::time::Duration;

// How the words of a run that ended with the code `nosave` begin. src/suggest/work.rs makes them from these and
// src/hook/writer_note.rs tells them apart to say what happened; each is followed by `: ` and the reason, except the last.

/// Every step of the check needed a record already in the user's data.
pub const NO_CHECK_SAVED: &str = "no check saved";
/// The engine's lint, or its try of the check, turned it down.
pub const TURNED_DOWN: &str = "the engine turned the check down";
/// The engine had another reason to save nothing.
pub const NOT_SAVED: &str = "Torpor did not save the check";
/// The end of the work loop, which no run reaches (the last try returns on every path).
pub const NO_CHECK_MADE: &str = "no check was made";

/// What the saved checks of the project in `cwd` do not cover, newest first (src/hook/writer_note.rs tells the user,
/// src/hook/handle.rs the model).
pub fn gaps(cwd: &str) -> Vec<Gap> {
    repo::resolve_root(Some(cwd)).map_or_else(|_| Vec::new(), |r| state::gaps(&r))
}

/// What the last run in the project of `cwd` left in `result.json` (src/hook/writer_note.rs tells the user when it
/// failed).
pub fn last_result(cwd: &str) -> Option<Last> {
    state::last_result(&repo::resolve_root(Some(cwd)).ok()?)
}

/// How long the Stop hook of the project in `cwd` waited, at most `cap`, for a check that the writer is saving (see
/// `state::wait_for_save`; it makes no file and returns at once when nothing is being saved).
pub fn wait_for_save(cwd: &str, cap: Duration) -> Duration {
    repo::resolve_root(Some(cwd))
        .and_then(|r| state::ws_path(&r))
        .map_or(Duration::ZERO, |d| state::wait_for_save(&d, cap))
}

/// Text from outside (an engine's stderr) made safe to show: control characters become spaces, anything that looks
/// like a key is masked, and at most `max` characters stay.
pub fn scrub(s: &str, max: usize) -> String {
    client::clean(&repo::redact(&client::clean(s, usize::MAX), false), max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_comes_from_outside_is_made_safe_to_show() {
        let s = scrub("boom \x1b[31mred\x1b[0m\nsk-FAKECANARY1234 end", 300);
        assert_eq!(s, "boom [31mred [0m [redacted] end");
        assert_eq!(scrub(&"x ".repeat(400), 300).chars().count(), 300);
    }
}

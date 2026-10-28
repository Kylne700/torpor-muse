//! What the hook reads back from the writer: open gaps, the last result, and text made safe to show.

use super::state::{Gap, Last};
use super::{client, repo, state};

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

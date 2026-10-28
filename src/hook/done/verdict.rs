//! The one sentence for the model when a claim has a contrary record, and how it joins the Stop block.

use super::Log;
use super::claim::{self, Claim, Topic};
use super::log::{Run, Why};
use crate::hook::{state, stop};
use std::path::Path;

/// The sentence for the Stop of `turn`, if the AI's last message `said` claims something this session's record
/// contradicts. A message with no claim costs nothing: no lock, no change to the state file.
pub(in crate::hook) fn verdict(dir: &Path, sid: &str, turn: &str, said: &str) -> Option<String> {
    let claims = claim::read(said);
    if claims.is_empty() {
        return None;
    }
    state::with_session(dir, sid, |s| {
        s.done.see(turn);
        sentence(&s.done, &claims)
    })
    .flatten()
}

/// Our sentence as a line of the Stop block: after the reason Muse already gives, or as the block on its own.
pub(in crate::hook) fn join(reason: Option<String>, ours: Option<String>) -> Option<String> {
    match (reason, ours) {
        (Some(r), Some(o)) => Some(format!("{r}\n- {o}")),
        (None, Some(o)) => Some(format!("{}\n- {o}", stop::HEAD)),
        (r, None) => r,
    }
}

/// The first false claim, tests before build.
pub(super) fn sentence(log: &Log, claims: &[Claim]) -> Option<String> {
    [Topic::Test, Topic::Build]
        .into_iter()
        .filter(|t| claims.iter().any(|c| c.topic == *t))
        .find_map(|t| log.why(t).map(|why| say(log, t, &why)))
}

/// Fact and next step, two sentences at most, 300 characters at most.
fn say(log: &Log, topic: Topic, why: &Why) -> String {
    let (claim, kind, redo, it) = match topic {
        Topic::Test => ("the tests pass", "test run", "run the tests", "they were"),
        Topic::Build => ("the build succeeds", "build", "run the build", "it was"),
    };
    match why {
        Why::NoRun => format!(
            "You wrote that {claim}, but no {kind} was seen in this session; {redo} now and report the real result, or tell the user {it} not run."
        ),
        Why::Failed(r) => format!(
            "You wrote that {claim}, but the last {kind} ({}) exited with {}; {redo} now and report the real result, or tell the user {it} not run.",
            named(log, r),
            r.code
        ),
        Why::Edited(r) => format!(
            "You wrote that {claim}, but code was changed after the last passing {kind} ({}); {redo} again and report the real result, or tell the user {it} not re-run.",
            named(log, r)
        ),
    }
}

/// `` `npm test`, 3 turns ago ``: the command, and when if it is known.
fn named(log: &Log, r: &Run) -> String {
    if r.turn == 0 || log.turn == 0 {
        return format!("`{}`", r.cmd);
    }
    match log.turn.saturating_sub(r.turn) {
        0 => format!("`{}`, this turn", r.cmd),
        1 => format!("`{}`, the previous turn", r.cmd),
        n => format!("`{}`, {n} turns ago", r.cmd),
    }
}

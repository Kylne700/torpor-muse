//! Test runs and builds in one Bash command line, by the lists in the brief, with the exit code that belongs to each.

use super::segment::{Hit, Mark, classify};
use super::split::{Chain, Sep, split};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Seen {
    /// Nothing of this kind in the command.
    No,
    /// Something of this kind may have run, or its exit code is not the command's: the kind is unknown from here on.
    Unsure,
    Ran(i64),
}

/// What the command line `cmd` that exited with `code` ran: (tests, build).
///
/// shortcut: a run is `Ran` only when the exit code is the run's: the last command of the line, or the last in the `;`
/// group with everything before it passing (`&&`); a failure is `Ran` only when nothing but `cd`, `export` and the like
/// stands in front of it. A pipe (`npm test | tail`), `||`, `&`, a group or a substitution hides the exit code, so the kind
/// becomes `Unsure` and nothing is said about it. Upgrade: the engine's tool_events after Nov 4 (71c M6, `exit_code_of`)
/// give an exit code per command.
pub(super) fn read(cmd: &str, code: Option<i64>) -> (Seen, Seen) {
    let chain = split(cmd);
    let hits: Vec<Hit> = chain.segs.iter().map(classify).collect();
    (
        seen(&chain, &hits, code, |h| h.test),
        seen(&chain, &hits, code, |h| h.build),
    )
}

fn seen(chain: &Chain, hits: &[Hit], code: Option<i64>, kind: impl Fn(&Hit) -> Mark) -> Seen {
    let marks: Vec<Mark> = hits.iter().map(kind).collect();
    let Some(listed) = marks.iter().rposition(|m| *m == Mark::Listed) else {
        return if marks.contains(&Mark::Maybe) {
            Seen::Unsure
        } else {
            Seen::No
        };
    };
    let Some(code) = code.filter(|_| !chain.murky) else {
        return Seen::Unsure;
    };
    // the exit code of a line is the one of its last `;` group
    let group = chain
        .seps
        .iter()
        .rposition(|s| *s == Sep::Semi)
        .map_or(0, |p| p + 1);
    if listed < group {
        return Seen::Unsure;
    }
    if code == 0 {
        return Seen::Ran(0);
    }
    let alone = (group..listed).all(|i| hits[i].trivial || marks[i] == Mark::Listed);
    if listed == hits.len() - 1 && alone {
        Seen::Ran(code)
    } else {
        Seen::Unsure
    }
}

/// Does the command line (a code span in a message) name a test run, a build?
pub(super) fn listed(cmd: &str) -> (bool, bool) {
    let hits: Vec<Hit> = split(cmd).segs.iter().map(classify).collect();
    (
        hits.iter().any(|h| h.test == Mark::Listed),
        hits.iter().any(|h| h.build == Mark::Listed),
    )
}

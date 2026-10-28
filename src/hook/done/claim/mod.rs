//! Does the AI's last message say that the tests pass or that the build succeeds? Only an explicit statement counts, so a
//! miss stays silent: a hedge, a plan, a condition, a question, a negation, advice, a quote, a code block, a part of the
//! whole, another place or another time is no claim (71c P4).
//!
//! shortcut: hand-written word lists, English only, and only the AI's last message of the turn (a claim in an earlier
//! message of the same turn is not read). A sentence with `production`, `staging`, a Docker image and the like is skipped
//! whole. Upgrade: compare the claim with this turn's own tool events once the engine gives them (the engine's tool_events
//! after Nov 4, 71c M5 `stop_line`) instead of widening the lists.

mod sets;
mod text;

use sets::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Topic {
    Test,
    Build,
}

pub(super) struct Claim {
    pub(super) topic: Topic,
    /// The sentence that made it, for the tests and the replay.
    #[cfg(test)]
    pub(super) shown: String,
}

/// The claims of a message, at most one per topic.
pub(super) fn read(said: &str) -> Vec<Claim> {
    let mut out: Vec<Claim> = Vec::new();
    for s in text::sentences(said) {
        for topic in claims(&s.words) {
            if !out.iter().any(|c| c.topic == topic) {
                out.push(Claim {
                    topic,
                    #[cfg(test)]
                    shown: s.shown.clone(),
                });
            }
        }
    }
    out
}

fn is(set: &[&str], w: &str) -> bool {
    set.contains(&w)
}

fn claims(t: &[String]) -> Vec<Topic> {
    let mut got = Vec::new();
    if out(t) {
        return got;
    }
    for p in 0..t.len() {
        if let Some(topic) = predicate(t, p).filter(|_| clause_ok(t, p))
            && !got.contains(&topic)
        {
            got.push(topic);
        }
    }
    got
}

/// The whole sentence is not the AI saying what is the case now.
fn out(t: &[String]) -> bool {
    t.iter().any(|w| is(SENTENCE, w) || w.ends_with("'ll"))
        || t.windows(2).any(|w| {
            PAIRS.contains(&(w[0].as_str(), w[1].as_str())) || (w[0] == "to" && is(TO, &w[1]))
        })
        || t.iter()
            .enumerate()
            .any(|(i, w)| is(SAY, w) && t[i.saturating_sub(3)..i].iter().any(|s| is(SOURCE, s)))
        || imperative(t)
        || partial(t)
}

/// An order: `run the tests`, `please make sure they pass`.
fn imperative(t: &[String]) -> bool {
    t.iter()
        .find(|w| !is(LEAD, w))
        .is_some_and(|w| is(ORDER, w))
}

/// `41 of 42 tests pass`, `41/42`: not all of them.
fn partial(t: &[String]) -> bool {
    let num = |w: &str| w.parse::<u64>().ok();
    let apart = |a: &str, b: &str| matches!((num(a), num(b)), (Some(x), Some(y)) if x != y);
    t.iter()
        .any(|w| w.split_once('/').is_some_and(|(a, b)| apart(a, b)))
        || t.windows(3).any(|w| w[1] == "of" && apart(&w[0], &w[2]))
}

/// The clause around the predicate says no, or is an order.
fn clause_ok(t: &[String], p: usize) -> bool {
    let lo = t[..p]
        .iter()
        .rposition(|w| is(CLAUSE, w))
        .map_or(0, |i| i + 1);
    let hi = t[p..]
        .iter()
        .position(|w| is(CLAUSE, w))
        .map_or(t.len(), |i| p + i);
    !imperative(&t[lo..hi]) && !negative(&t[lo..hi])
}

/// Something is wrong or not done, except where it is said to be absent (`no failures`, `0 failed`).
fn negative(c: &[String]) -> bool {
    let mut i = 0;
    while i < c.len() {
        let w = c[i].as_str();
        if is(ZERO, w) {
            let mut j = i + 1;
            while c.get(j).is_some_and(|m| is(ZERO_MID, m)) {
                j += 1;
            }
            if c.get(j).is_some_and(|e| is(ZERO_END, e)) {
                i = j + 1;
                continue;
            }
        }
        if is(NEGATIVE, w) || w.ends_with("n't") {
            return true;
        }
        i += 1;
    }
    false
}

/// What the word at `p` states, if it states success of the tests or of the build.
fn predicate(t: &[String], p: usize) -> Option<Topic> {
    let w = t[p].as_str();
    let next = t.get(p + 1).map(String::as_str);
    let zero = |at: usize| {
        t.get(at)
            .is_some_and(|z| matches!(z.as_str(), "no" | "zero" | "0"))
    };
    if is(PASS, w) {
        // `passes the linter`
        next.is_none_or(|n| !is(OBJECT, n))
            .then(|| subject(t, p, false, &[]))
            .flatten()
    } else if is(SUCCEED, w) {
        subject(t, p, false, &[])
    } else if is(FINISH, w) {
        // `finishes successfully`, `completes without errors`
        (next.is_some_and(|n| is(FINISH_NEXT, n)) || (next == Some("with") && zero(p + 2)))
            .then(|| subject(t, p, false, &[]))
            .flatten()
    } else if is(BUILD_ONLY, w) {
        subject(t, p, true, &[])
    } else if is(COMPILE, w) {
        compiles(t, p)
    } else if zero(p) && next.is_some_and(|n| matches!(n, "error" | "errors")) {
        // `tsc reports no errors`
        subject(t, p, true, REPORTS)
    } else {
        None
    }
}

/// `it compiles`, `the app builds`, `compiles fine now`: the verb alone says the build is fine, when its subject is the
/// project and not, say, a regex.
fn compiles(t: &[String], p: usize) -> Option<Topic> {
    let next = match t.get(p + 1).map(String::as_str) {
        None => true,
        Some("with") => t
            .get(p + 2)
            .is_some_and(|z| matches!(z.as_str(), "no" | "zero" | "0")),
        Some(n) => is(AFTER_COMPILE, n),
    };
    let who = before(t, p, &[])
        .is_none_or(|j| t[j] == ";" || is(COMPILE_SUBJECT, &t[j]) || is(BUILD_SUBJECT, &t[j]));
    (next && who).then_some(Topic::Build)
}

/// What the predicate at `p` is said of: the tests or the build, itself or through `they`/`everything` after a run.
fn subject(t: &[String], p: usize, only_build: bool, skip: &[&str]) -> Option<Topic> {
    let j = before(t, p, skip)?;
    let topic = kind(t, j).or_else(|| if is(PRONOUN, &t[j]) { ran(t, j) } else { None })?;
    (!only_build || topic == Topic::Build).then_some(topic)
}

fn kind(t: &[String], j: usize) -> Option<Topic> {
    let w = t[j].as_str();
    if is(BUILD_SUBJECT, w) {
        return Some(Topic::Build);
    }
    let of_tests = matches!(w, "results" | "result" | "run" | "runs" | "case" | "cases")
        && j > 0
        && matches!(t[j - 1].as_str(), "test" | "tests");
    (is(TEST_SUBJECT, w) || of_tests).then_some(Topic::Test)
}

/// `they` at `j` after `I ran the tests`: the run verb nearest in front of it and the subject in the four words after it.
fn ran(t: &[String], j: usize) -> Option<Topic> {
    let r = t[..j].iter().rposition(|w| is(RUN, w))?;
    (r + 1..j.min(r + 5)).find_map(|s| kind(t, s))
}

/// The word in front of `p` that is no filler (`now`, `are`, a number, `in the repo`, the words in `skip`).
fn before(t: &[String], p: usize, skip: &[&str]) -> Option<usize> {
    let mut j = p;
    while j > 0 {
        j -= 1;
        let at = |n: usize| j.checked_sub(n).map_or("", |i| t[i].as_str());
        let (w, p1, p2) = (t[j].as_str(), at(1), at(2));
        if w == "codespan" && is(IN, p1) {
            j -= 1;
        } else if is(PLACE, w) && is(DET, p1) && is(IN, p2) {
            j -= 2;
        } else if !(is(FILLER, w) || is(skip, w) || w.starts_with(|c: char| c.is_ascii_digit())) {
            return Some(j);
        }
    }
    None
}

#[cfg(test)]
mod tests;

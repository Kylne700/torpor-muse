//! The check-writer's words for the user: what it did, what failed, what it could not take up and what a check misses.

use super::state;
use crate::util::fnv1a;
use crate::{schema, suggest};
use serde_json::Value;
use std::path::Path;

/// What the check-writer has to tell the user at a prompt: one note at most, each kind once per session, except the two
/// that are a setting of the folder, `no-key` and `app`: once per folder, and `no-key` once per reason (a different
/// reason for the same folder is told once more; a mark that is never swept; where the folder is not known, or the mark
/// cannot be written, once per session). In this order: the key it lacks (`no-key`), why a run could not start on the
/// last prompt (`busy`, `start`), why the last run of this session ended without a saved check (`key`, `cap`, `api`,
/// `slow`, `rejected`, `error`, `app`, and the ends of a claim whose check the engine did not take: `fails`, `engine`,
/// `nosave`). A prompt that was no claim, and a claim that cannot be checked, are not told. The notes are for the user
/// only (`systemMessage`; doctor says the first too): the model's context stays as it is, so a run without a key gives
/// the model the same text as one with a key. A kind is recorded only when its note really is printed.
pub(super) fn writer_note(
    dir: &Path,
    event: &str,
    sid: &str,
    cwd: &str,
    body: Option<String>,
) -> Option<String> {
    let Some((told, skip)) =
        state::with_session(dir, sid, |s| (s.writer_told.clone(), s.writer_skip.clone()))
    else {
        return body;
    };
    let off = suggest::writer_off(true).map(|why| suggest::scrub(&why, 300));
    // what a per-folder kind is marked with: the folder, and for `no-key` the reason too. A mark made before the reason
    // was part of it (the folder alone) never matches, so that note is told once more.
    let mark_of = |kind: &str| match kind {
        _ if cwd.is_empty() => None,
        "app" => Some(cwd.to_owned()),
        "no-key" => {
            let why = off.as_deref().unwrap_or("");
            Some(format!("{cwd}\n{:016x}", fnv1a(why.as_bytes())))
        }
        _ => None,
    };
    let new = |kind: &str| {
        !told.iter().any(|t| t == kind)
            && !mark_of(kind).is_some_and(|m| state::marked(dir, kind, &m))
    };
    let Some((kind, line)) = writer_news(sid, cwd, &skip, off.as_deref())
        .into_iter()
        .find(|(kind, _)| new(kind))
    else {
        return body;
    };
    let Some(merged) = schema::with_note(event, body.as_deref(), &line, "", false) else {
        return body;
    };
    if !mark_of(kind).is_some_and(|m| state::mark(dir, kind, &m)) {
        state::with_session(dir, sid, |s| {
            s.writer_told.push(kind.into());
            if s.writer_skip == kind {
                s.writer_skip.clear();
            }
        });
    }
    Some(merged)
}

/// The notes the check-writer could have for the user now, most pressing first: (kind, the words). `skip`: why the
/// trigger could not start a run on the last prompt (`Session.writer_skip`). `off`: why the check-writer is off, if it is.
fn writer_news(sid: &str, cwd: &str, skip: &str, off: Option<&str>) -> Vec<(&'static str, String)> {
    const SAVING: &str = "Save points and undo are not affected.";
    if let Some(why) = off {
        return vec![(
            "no-key",
            format!(
                "Torpor's check-writer is off: {why}. To turn it on, put your Meta API key on one line in a file only you \
                 can read (chmod 600) and add `key_file = ~/path/to/that/file` to ~/.config/torpor-muse/config (chmod 600). \
                 `torpor-muse doctor` shows this too."
            ),
        )];
    }
    let mut news = Vec::new();
    match skip {
        "busy" => news.push((
            "busy",
            format!("Torpor's check-writer was still busy with an earlier prompt, so it did not look at your last one. {SAVING}"),
        )),
        "start" => news.push((
            "start",
            format!("Torpor could not start its check-writer on your last prompt. {SAVING}"),
        )),
        _ => {}
    }
    // the run of this session that finished last; its words are the run's own, made safe once more
    let last = suggest::last_result(cwd).filter(|l| !l.saved && l.session.as_deref() == Some(sid));
    if let Some(l) = last {
        let kind = [
            "key", "cap", "api", "slow", "rejected", "app", "fails", "engine", "nosave",
        ]
        .into_iter()
        .find(|k| *k == l.code)
        .or((l.outcome == "error").then_some("error"));
        let line = match kind {
            None => return news,
            Some("app") => "Torpor's check-writer needs an app it can start with a script (`start`, `dev`, `serve` or \
                 `server` in package.json, or a server.js), and this folder has none, so it will write no checks here. \
                 Save points and undo still work."
                .to_string(),
            Some("slow") => format!(
                "Torpor's check-writer took too long on an earlier prompt of yours and was stopped before it finished. {SAVING}"
            ),
            Some("rejected") => format!(
                "Torpor's check-writer could not use the model's answer to an earlier prompt of yours, so it wrote no check. {SAVING}"
            ),
            Some("fails") => format!(
                "The check Torpor's check-writer wrote for an earlier claim of yours fails on the code as it is now, which can \
                 mean that what you claimed does not work yet or that the check is wrong, so no check was saved. {SAVING}"
            ),
            Some("engine") => format!(
                "Torpor's check-writer wrote a check for an earlier claim of yours, but the Torpor engine did not finish saving \
                 it (it timed out or ended with an error), so no check was saved. {SAVING}"
            ),
            Some(_) => format!(
                "Torpor's check-writer wrote no check from an earlier prompt of yours: {}. {SAVING}",
                suggest::scrub(&l.message, 300).trim_end_matches('.')
            ),
        };
        news.extend(kind.map(|k| (k, line)));
    }
    news
}

/// L3 N4: what a check the writer saved does not cover (`suggest::gaps`). The user reads it short inside the engine's
/// line that announces the promise (a line of its own would fall past the width of Muse's one-line status bar). Some =
/// the engine's Stop output with it put in.
pub(super) fn gap_line(event: &str, sid: &str, cwd: &str, engine_out: &[u8]) -> Option<Vec<u8>> {
    if event != "Stop" || cwd.is_empty() {
        return None;
    }
    let mut v: Value = serde_json::from_slice(engine_out).ok()?;
    let mut m = v["systemMessage"].as_str()?.to_owned();
    if !m.contains("Torpor: new promise #") {
        return None;
    }
    let mine = |g: &suggest::Gap| g.session.as_deref().is_none_or(|s| s == sid);
    for g in suggest::gaps(cwd).iter().filter(|g| mine(g)) {
        if let Some(t) = gap_in_line(&m, g.promise, &suggest::scrub(&g.gap, 80)) {
            m = t;
        }
    }
    v["systemMessage"] = Value::String(m);
    serde_json::to_vec(&v).ok()
}

/// `Torpor: new promise #2 "name" (guard only), from ... Drop it: torpor promise drop 2` gets `; not covered: <gap>`
/// before the `)` that closes its label.
fn gap_in_line(m: &str, id: u32, gap: &str) -> Option<String> {
    let start = m.find(&format!("Torpor: new promise #{id} \""))?;
    let end = start + m[start..].find(&format!(". Drop it: torpor promise drop {id}"))?;
    let at = start + m[start..end].rfind("), ")?;
    Some(format!("{}; not covered: {gap}{}", &m[..at], &m[at..]))
}

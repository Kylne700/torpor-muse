//! The check-writer's words for the user: what it sends, what it did, what failed, what it could not take up and what a
//! check misses.

use super::state;
use crate::util::fnv1a;
use crate::{engine, schema, suggest};
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

const SAVING: &str = "Save points and undo are not affected.";

/// Told once to the user in a folder where the check-writer will send a prompt: what goes to Meta and how to stop it.
/// The facts are those of the README, "What is sent to Meta".
const SEND_NOTICE: &str = "Torpor's check-writer sends each prompt you type, cleaned and redacted, to Meta to ask whether \
    it says something works, and sends file names and code from this project only when the answer is yes or when no usable \
    answer came back. `torpor off` in this folder stops this, and stops Torpor's saving here too.";

/// What the check-writer has to tell the user at a prompt: one note at most, each kind once per session, except the
/// three that are about the folder, `send`, `app` and `no-key`: once per folder, and `no-key` once per reason (a
/// different reason for the same folder is told once more; a mark that is never swept; where the folder is not known,
/// or the mark cannot be written, once per session). In this order: the key it lacks (`no-key`), what it sends to Meta
/// (`send`; it comes with the prompt whose words the run then sends, since the run starts after the hook has printed;
/// `sends` says whether this prompt will be, and is asked only while `send` is still to be told), why a run could not
/// start on the last prompt (`busy`, `start`), why the last run of this session ended without a saved check (`key`,
/// `cap`, `api`, `slow`, `rejected`, `error`, `app`, and the ends of a claim whose check the engine did not take:
/// `fails`, `engine`, `nosave`). A prompt that was no claim, and a claim that cannot be checked, are not told. The notes
/// are for the user only (`systemMessage`; doctor says the first too): the model's context stays as it is, so a run
/// without a key gives the model the same text as one with a key. A kind is recorded only when its note really is
/// printed.
pub(super) fn writer_note(
    dir: &Path,
    event: &str,
    sid: &str,
    cwd: &str,
    body: Option<String>,
    sends: &dyn Fn() -> bool,
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
        "app" | "send" => Some(cwd.to_owned()),
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
    let send = || new("send") && sends(); // `sends` costs an engine call: only while the notice is still to be told
    let Some((kind, line)) = writer_news(sid, cwd, &skip, off.as_deref(), &send)
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
/// `send`: whether the notice of what is sent is due now.
fn writer_news(
    sid: &str,
    cwd: &str,
    skip: &str,
    off: Option<&str>,
    send: &dyn Fn() -> bool,
) -> Vec<(&'static str, String)> {
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
    if send() {
        news.push(("send", SEND_NOTICE.to_string())); // the notes below wait a prompt; they stay true
    }
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
            Some("nosave") => nosave_line(&suggest::scrub(&l.message, 300)),
            Some(_) => format!(
                "Torpor's check-writer wrote no check from an earlier prompt of yours: {}. {SAVING}",
                suggest::scrub(&l.message, 300).trim_end_matches('.')
            ),
        };
        news.extend(kind.map(|k| (k, line)));
    }
    news
}

/// Will the check-writer send this prompt to Meta? What a run reads before it sends, the cheap local reads first
/// (`suggest::would_send`: a usable key, a prompt with something in it, an app to start, a day cap above 0), then the
/// engine's word that Torpor saves here (after `torpor off` the run stops before it sends anything). `left`: what is
/// left of the hook's time; an engine that does not answer in it counts as no, and the notice waits for a prompt where
/// it does.
/// shortcut: a run already under way, a day cap that is used up and a `status` that fails once are not seen, so the
/// notice can come with a prompt that is not sent after all, or one prompt late; upgrade if that shows up.
pub(super) fn will_send(cwd: &str, prompt: &str, bin: &Path, left: Duration) -> bool {
    !cwd.is_empty()
        && left >= Duration::from_millis(250)
        && suggest::would_send(cwd, prompt)
        && engine::exec(bin, &["status", "--json"], None, Some(Path::new(cwd)), left)
            .ok()
            .filter(|o| o.code == 0)
            .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
            .is_some_and(|s| {
                s["off"] != true && s["saving"] == true && s["last_turn"]["n"].is_u64()
            })
}

/// The sentence for a claim whose check was not saved, from the run's words (`words`, made safe). The generic frame
/// would say the check-writer wrote no check and then the run's own words would say it again, and for the ends the engine
/// caused a check was written.
fn nosave_line(words: &str) -> String {
    use suggest::{NO_CHECK_MADE, NO_CHECK_SAVED, NOT_SAVED, TURNED_DOWN};
    let after = |head: &str| {
        words
            .strip_prefix(head)
            .map(|rest| rest.trim_start_matches(':').trim())
    };
    let wrote = "Torpor's check-writer wrote a check for an earlier claim of yours, but";
    if after(NO_CHECK_SAVED).is_some() {
        format!(
            "{wrote} every step in it needed a record already in your data, which the check did not create, so no check \
             was saved. {SAVING}"
        )
    } else if let Some(why) = after(TURNED_DOWN).or_else(|| after(NOT_SAVED)) {
        format!(
            "{wrote} the Torpor engine did not save it{}. {SAVING}",
            because(&engine_reason(why))
        )
    } else if after(NO_CHECK_MADE).is_some() {
        format!("Torpor's check-writer made no check for an earlier claim of yours. {SAVING}")
    } else {
        format!(
            "Torpor's check-writer saved no check for an earlier claim of yours{}. {SAVING}",
            because(words.trim_end_matches('.'))
        )
    }
}

/// The engine's reason without the `Rejected: ` that heads each problem its lint names; the problems, `; ` between them.
fn engine_reason(s: &str) -> String {
    s.split("Rejected:")
        .map(|p| p.trim().trim_end_matches('.').trim_end())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

/// `: <why>` to end a sentence with, nothing when there is no why.
fn because(why: &str) -> String {
    if why.is_empty() {
        String::new()
    } else {
        format!(": {why}")
    }
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

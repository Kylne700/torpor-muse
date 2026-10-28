//! What the hook does for one event: check the payload, ask the engine, add our notes, guard a command, reply.

use super::db_note::db_note;
use super::events::{Act, EVENTS, RETIRED};
use super::gate::gate_default;
use super::paths::{engine_cmd, project_root};
use super::stop_note::{Ended, stop_note};
use super::writer_note::{gap_line, will_send, writer_note};
use super::{Done, busy, normalise, state};
use crate::{engine, liveness, schema, suggest, transcript};
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

pub(super) fn handle(dir: &Path, event: &str, input: &[u8]) -> Done {
    if RETIRED.contains(&event) {
        return Done::default();
    }
    let Some(ev) = EVENTS.iter().find(|e| e.name == event) else {
        state::problem(dir, event, "", "unknown event name on the command line");
        return Done::default();
    };
    let mut payload: Value = match serde_json::from_slice(input) {
        Ok(v @ Value::Object(_)) => v,
        _ => {
            let msg = format!("stdin is not a JSON object ({} bytes)", input.len());
            state::problem(dir, event, "", &msg);
            return Done::default();
        }
    };
    let Some(mut sid) = payload["session_id"].as_str().map(str::to_owned) else {
        state::problem(dir, event, "", "payload has no session_id");
        return Done::default();
    };
    let main = !schema::is_child(&payload);
    liveness::beat(event, &sid, main);
    if main {
        check_shape(dir, event, &sid, &payload);
    } else if let Some(parent) = adopt(dir, event, &mut payload) {
        sid = parent;
    } else {
        return Done::default();
    }
    if ev.act != Act::Engine {
        return Done::default();
    }
    if event == "SessionStart" {
        state::sweep(dir);
    }
    let text = |k: &str| payload[k].as_str().unwrap_or("").to_owned();
    let (cwd, prompt, turn) = (text("cwd"), text("prompt"), text("turn_id"));
    if main && event == "UserPromptSubmit" && !cwd.is_empty() {
        state::open_set(dir, &cwd, &sid, &turn);
    }
    if let Some(m) = payload.as_object_mut() {
        normalise::apply(event, m);
    }
    if let Some(p) = transcript::path_for(event, &payload) {
        payload["transcript_path"] = Value::String(p);
    }
    let bytes = serde_json::to_vec(&payload).unwrap_or_default();
    let started = Instant::now();
    let ran = engine::run(
        &["hook", event],
        Some(&bytes),
        Duration::from_millis(ev.engine_ms),
    );
    let mut done = Done::default();
    match ran {
        Err(e) => done.stdout = fail_open(dir, event, &sid, &e),
        Ok(out) if out.code != 0 => {
            let err = String::from_utf8_lossy(&out.stderr);
            let why = format!("torpor hook {event} exited {}: {}", out.code, err.trim());
            done.stdout = fail_open(dir, event, &sid, &why);
        }
        Ok(out) => {
            let edited = gap_line(event, &sid, &cwd, &out.stdout);
            let engine_out = edited.as_deref().unwrap_or(&out.stdout);
            done.stdout = reply(dir, event, &sid, &turn, &engine_cmd(&out.bin), engine_out);
            if event == "UserPromptSubmit" && !prompt.trim().is_empty() {
                let left = Duration::from_millis(ev.engine_ms).saturating_sub(started.elapsed());
                let sends = || will_send(&cwd, &prompt, &out.bin, left);
                done.stdout = writer_note(dir, event, &sid, &cwd, done.stdout.take(), &sends);
                done.prompt = Some((sid.clone(), cwd.clone(), prompt));
            }
            done.stdout = gap_note(dir, event, &sid, &cwd, done.stdout.take());
            done.stdout = if event == "Stop" {
                let left = Duration::from_millis(ev.engine_ms).saturating_sub(started.elapsed());
                let end = Ended {
                    dir,
                    sid: &sid,
                    turn: &turn,
                    cwd: &cwd,
                    bin: &out.bin,
                    left,
                };
                stop_note(&end, &out.stdout, done.stdout.take())
            } else {
                db_note(dir, event, &sid, &cwd, &out.bin, done.stdout.take())
            };
            if event == "SessionStart" {
                let left = Duration::from_millis(ev.engine_ms).saturating_sub(started.elapsed());
                gate_default(dir, &sid, &cwd, &out.bin, left);
            }
        }
    }
    if event == "PreToolUse" {
        let left = Duration::from_millis(ev.engine_ms).saturating_sub(started.elapsed());
        done.stdout = busy_guard(&payload, &cwd, left, done.stdout.take());
    }
    if main && !cwd.is_empty() && turn_ended(event, done.stdout.as_deref()) {
        state::open_clear(dir, &cwd, &sid);
    }
    if event == "SessionEnd" {
        state::end_session(dir, &sid);
    }
    done
}

/// A model-spawned subagent works inside its parent's turn, but Muse sends its tool events under a session id and turn
/// id of its own with no link to the parent. The engine's own picture of a subagent is the parent's ids plus an
/// `agent_id`, so when a main turn is open in the child's folder the child's tool event is rewritten that way and goes
/// on like a main one: the engine then guards Torpor's own files against the subagent too. Returns the parent's session
/// id. Everything else a child sends (SubagentStart/Stop, an observer's events) stays heartbeat-only.
/// shortcut: the parent is found by folder alone (two Muse windows in one folder can swap parents), and a subagent
/// that outlives its parent's Stop is not seen; upgrade when Muse puts the parent's id into a child's payload.
fn adopt(dir: &Path, event: &str, payload: &mut Value) -> Option<String> {
    if !matches!(event, "PreToolUse" | "PostToolUse" | "PostToolUseFailure") {
        return None;
    }
    let cwd = payload["cwd"].as_str().filter(|c| !c.is_empty())?;
    let (parent, turn) = state::open_get(dir, cwd)?;
    payload["agent_id"] = payload["session_id"].take();
    payload["session_id"] = Value::String(parent.clone());
    payload["turn_id"] = Value::String(turn);
    Some(parent)
}

/// The turn is over: a Stop that did not block (Muse runs a blocked turn on), StopFailure, SessionEnd.
pub(super) fn turn_ended(event: &str, stdout: Option<&str>) -> bool {
    match event {
        "StopFailure" | "SessionEnd" => true,
        "Stop" => !stdout
            .and_then(|s| serde_json::from_str::<Value>(s).ok())
            .is_some_and(|v| v["decision"] == "block"),
        _ => false,
    }
}

/// The typed parse is a check only (control flow uses the raw JSON): keys we have no field for are logged once.
fn check_shape(dir: &Path, event: &str, sid: &str, payload: &Value) {
    if payload["hook_event_name"]
        .as_str()
        .is_some_and(|n| n != event)
    {
        let msg = format!(
            "hook_event_name is {} but the command says {event}",
            payload["hook_event_name"]
        );
        state::problem(dir, event, sid, &msg);
    }
    match schema::Payload::parse(event, payload) {
        Ok(p) => {
            for k in p.extra().keys() {
                if state::new_key(dir, event, k) {
                    let msg = format!("payload key {k} is not in our schema (kept, passed on)");
                    state::problem(dir, event, sid, &msg);
                }
            }
        }
        Err(e) => {
            if state::new_key(dir, event, "<shape>") {
                state::problem(
                    dir,
                    event,
                    sid,
                    &format!("payload does not fit our schema: {e}"),
                );
            }
        }
    }
}

/// The model of the session whose prompt started the writer gets what its new checks do not cover, in full, once, at
/// its next prompt (so it does not say a part is checked that is not).
fn gap_note(dir: &Path, event: &str, sid: &str, cwd: &str, body: Option<String>) -> Option<String> {
    if event != "UserPromptSubmit" || cwd.is_empty() {
        return body;
    }
    let Some(told) = state::with_session(dir, sid, |s| s.gap_told) else {
        return body;
    };
    let new: Vec<suggest::Gap> = suggest::gaps(cwd)
        .into_iter()
        .filter(|g| g.session.as_deref() == Some(sid) && g.promise > told)
        .collect();
    let Some(newest) = new.iter().map(|g| g.promise).max() else {
        return body;
    };
    let mut note = String::from(
        "[Torpor check note: later Torpor notes do not replace this one] These new promises have a check that does \
         not test everything the user said. Never tell the user that these parts are checked.",
    );
    for g in &new {
        let gap = suggest::scrub(&g.gap, 600);
        note.push_str(&format!("\nPromise #{} does not cover: {gap}", g.promise));
    }
    match schema::with_note(event, body.as_deref(), "", &note, false) {
        Some(m) => {
            state::with_session(dir, sid, |s| s.gap_told = newest);
            Some(m)
        }
        None => body,
    }
}

/// F4 item 4 (drill D07): a Bash command that would delete, move or overwrite a database that is in use is denied (Muse
/// runs on, the model gets the reason). The engine saves no database whose `-wal` file holds data or whose `-journal`
/// file exists, and its own PreToolUse answer for such a command is empty, so after the command `torpor back
/// --with-data` would print "Not touched ... its database was busy" and the data would be gone. What counts as a
/// command that does that, and the commands this cannot see, is in `busy`. The engine's own deny stays as it is.
/// Not when `torpor off` was typed (nothing is saved then). The engine's answer is read from the same call as the hook's,
/// so a slow or missing engine does not lift the guard: only the `status --json` look is skipped.
fn busy_guard(payload: &Value, cwd: &str, left: Duration, body: Option<String>) -> Option<String> {
    let denied = body
        .as_deref()
        .and_then(|b| serde_json::from_str::<Value>(b).ok())
        .is_some_and(|v| v["hookSpecificOutput"]["permissionDecision"].is_string());
    let Some(cmd) = payload["tool_input"]["command"].as_str() else {
        return body;
    };
    if payload["tool_name"] != "Bash" || cwd.is_empty() || denied {
        return body;
    }
    let root = project_root(cwd);
    let workdir = payload["tool_input"]["workdir"].as_str().unwrap_or("");
    let names = busy::targets(cmd, &Path::new(cwd).join(workdir), &root);
    if names.is_empty() {
        return body;
    }
    let t = left.min(Duration::from_secs(1));
    if let (Ok(bin), true) = (engine::locate(), t >= Duration::from_millis(250)) {
        let off = engine::exec(&bin, &["status", "--json"], None, Some(Path::new(cwd)), t)
            .ok()
            .filter(|o| o.code == 0)
            .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
            .is_some_and(|s| s["saving"] == false);
        if off {
            return body;
        }
    }
    let shown: Vec<String> = names
        .iter()
        .take(3)
        .map(|n| suggest::scrub(n, 80))
        .collect();
    let more = names.len().saturating_sub(3);
    let list = format!(
        "{}{}",
        shown.join(", "),
        if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        }
    );
    // A crash leaves `-wal`/`-journal` files that no command could remove, so the deny would never lift. One read makes
    // SQLite finish the job: it rolls a hot `-journal` back, and the last connection to close folds the `-wal` into the
    // file and removes `-wal` and `-shm`; while another program holds the database the files stay and so does the deny.
    // The text never offers a command that deletes or empties a file of the database.
    // shortcut: a `-journal` left by a clean close in PERSIST or TRUNCATE mode survives the read (SQLite keeps it on
    // purpose), so the text ends with the user; upgrade when the engine stops calling such a journal busy.
    let why = format!(
        "Torpor: this command was not run. A database is in use: {list}. Torpor cannot save a database while it is in \
         use, so it could not bring it back after this command. Stop the app that uses it first (or ask the user to), \
         then run the command again. If no app is running (a crash left its files behind), let SQLite clean them up by \
         reading each database once: `sqlite3 <db> 'SELECT count(*) FROM sqlite_master;'` (without the sqlite3 \
         program: `python3 -c \"import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); \
         c.execute('SELECT count(*) FROM sqlite_master').fetchall(); c.close()\" <db>`), then run the command \
         again. If it is refused again, ask the user: they can run it in their own terminal, but Torpor cannot \
         bring the database back."
    );
    schema::deny("PreToolUse", body.as_deref(), &why).or(body)
}

/// Engine stdout -> what Muse accepts (None = print nothing). `torpor`: see `engine_cmd`.
fn reply(
    dir: &Path,
    event: &str,
    sid: &str,
    turn: &str,
    torpor: &str,
    engine_out: &[u8],
) -> Option<String> {
    let header = || {
        let n = state::with_session(dir, sid, |s| s.next_seq()).unwrap_or(0);
        let tail = "the newest Torpor note replaces all earlier ones";
        if n == 0 {
            format!("[Torpor note: {tail}]")
        } else {
            format!("[Torpor note #{n}: {tail}]")
        }
    };
    let (body, notes) = schema::sanitise(event, engine_out, torpor, header);
    for n in notes {
        state::problem(dir, event, sid, &n);
    }
    let body = body?;
    if event == "Stop" && schema::validate(event, &body).is_ok_and(|o| o.decision.is_some()) {
        let allowed = state::with_session(dir, sid, |s| s.allow_block(turn)).unwrap_or(true);
        if !allowed {
            let msg = format!(
                "Stop block dropped: {} blocks already given in this turn",
                state::STOP_CAP
            );
            state::problem(dir, event, sid, &msg);
            return None;
        }
    }
    Some(body)
}

/// The engine could not do its work: log it, let the turn go on, and tell the user once per session.
fn fail_open(dir: &Path, event: &str, sid: &str, why: &str) -> Option<String> {
    state::problem(dir, event, sid, why);
    if !matches!(event, "SessionStart" | "UserPromptSubmit" | "Stop") {
        return None;
    }
    if !state::with_session(dir, sid, |s| !std::mem::replace(&mut s.told, true))? {
        return None;
    }
    let why = suggest::scrub(why, 300); // the engine's stderr: no control characters, nothing that looks like a key
    schema::notice(
        event,
        &format!("Torpor is NOT saving this project: {why}. Run `torpor-muse doctor`."),
        &format!(
            "[Torpor note] Torpor is NOT saving this project: {why}. Tell the user so in your next reply, in one sentence, \
             and that `torpor-muse doctor` shows how to fix it. Do not try to fix it yourself."
        ),
    )
}

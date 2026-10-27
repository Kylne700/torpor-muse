//! `torpor-muse hook <Event>`: reads the Muse Code hook payload on stdin, normalises it, calls `torpor hook <Event>`,
//! writes Muse's closed output schema (research/59 D5). Owner: S3a-core.
//!
//! Policy: a hook never fails a Muse turn. Whatever goes wrong (bad payload, engine missing or of the wrong version,
//! timeout, output Muse would reject) is written to `<TORPOR_HOME>/muse/problems.log`, the hook exits 0, and the user
//! is told once per session that Torpor is not saving. The only way a hook stops a turn is a Stop block in the JSON
//! form with a reason (never exit 2).

mod normalise;
mod state;

use crate::{engine, liveness, schema, suggest, transcript};
use serde_json::Value;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    /// Registered in the Muse settings; the payload goes to `torpor hook <Event>`.
    Engine,
    /// Registered; only the heartbeat is recorded (`liveness::beat`), the engine is not called.
    Beat,
    /// Not registered.
    Off,
}

/// One Muse hook event: what we do with it and why. `install` writes the settings from this table, so the table is
/// the event coverage list (every event research/59 section 4 names, none blank).
pub struct Ev {
    pub name: &'static str,
    pub act: Act,
    /// Muse matcher. Only SessionStart and the three tool events take one (research/59 D5).
    pub matcher: Option<&'static str>,
    /// `timeout` of the settings entry: INTEGER SECONDS (research/59 D7).
    pub timeout_s: u32,
    /// We kill the engine after this many ms: its own deadline for the event plus 1.5 s.
    pub engine_ms: u64,
    #[allow(dead_code)] // the coverage list for people and tests; no code reads it
    pub why: &'static str,
}

const TOOLS: Option<&str> = Some("bash|edit_file|write_file");

pub const EVENTS: &[Ev] = &[
    Ev {
        name: "SessionStart",
        act: Act::Engine,
        matcher: Some("startup|resume|clear|compact|fork"),
        timeout_s: 10,
        engine_ms: 4_500,
        why: "engine opens the session and hands its records back after clear, resume or compact; the matcher has `fork` (D1)",
    },
    Ev {
        name: "UserPromptSubmit",
        act: Act::Engine,
        matcher: None,
        timeout_s: 10,
        engine_ms: 6_500,
        why: "engine starts the turn (save point, promise words, state note); then suggest::trigger runs",
    },
    Ev {
        name: "PreToolUse",
        act: Act::Engine,
        matcher: TOOLS,
        timeout_s: 6,
        engine_ms: 3_500,
        why: "engine saves a point before a destructive bash command, records an agent-run `torpor` call (AD3), denies edits of its own files",
    },
    Ev {
        name: "PostToolUse",
        act: Act::Engine,
        matcher: TOOLS,
        timeout_s: 6,
        engine_ms: 3_500,
        why: "engine records the call and captures the files an edit touched",
    },
    Ev {
        name: "PostToolUseFailure",
        act: Act::Engine,
        matcher: TOOLS,
        timeout_s: 6,
        engine_ms: 3_500,
        why: "a non-zero bash exit fires only this event (D11); same handling as PostToolUse, error reshaped to `Exit code N`",
    },
    Ev {
        name: "PostToolBatch",
        act: Act::Beat,
        matcher: None,
        timeout_s: 5,
        engine_ms: 0,
        why: "heartbeat only: every call of a batch also fires its own Pre/PostToolUse (42 of 42 in G0), so the engine saved per call; it has no batch entry point and nothing depends on PreCompact (D4)",
    },
    Ev {
        name: "Stop",
        act: Act::Engine,
        matcher: None,
        timeout_s: 35,
        engine_ms: 31_500,
        why: "engine saves the end-of-turn point and runs the promise gate; at most 2 blocks per turn (engine and ours) against Muse's 8 (D8)",
    },
    Ev {
        name: "StopFailure",
        act: Act::Engine,
        matcher: None,
        timeout_s: 10,
        engine_ms: 6_500,
        why: "engine closes the turn that a model or auth error ended",
    },
    Ev {
        name: "SessionEnd",
        act: Act::Engine,
        matcher: None,
        timeout_s: 5,
        engine_ms: 2_700,
        why: "engine closes the session (and its cleanup when due); our session state file is deleted",
    },
    Ev {
        name: "PreCompact",
        act: Act::Engine,
        matcher: None,
        timeout_s: 5,
        engine_ms: 2_500,
        why: "engine notes the compaction; no save depends on it, Muse does not fire it on a manual /compact (D4)",
    },
    Ev {
        name: "PostCompact",
        act: Act::Beat,
        matcher: None,
        timeout_s: 3,
        engine_ms: 0,
        why: "heartbeat only: the engine ignores it; the SessionStart with source compact brings its records back",
    },
    Ev {
        name: "SubagentStart",
        act: Act::Beat,
        matcher: None,
        timeout_s: 3,
        engine_ms: 0,
        why: "heartbeat only: the engine ignores it; a child session's own events are ignored after the heartbeat (D10)",
    },
    Ev {
        name: "SubagentStop",
        act: Act::Engine,
        matcher: None,
        timeout_s: 5,
        engine_ms: 2_500,
        why: "engine ends a subagent's work in the parent session (main sessions only; real shape not seen, K13 not run)",
    },
    Ev {
        name: "PreLLMCall",
        act: Act::Off,
        matcher: None,
        timeout_s: 0,
        engine_ms: 0,
        why: "not registered: 8 KB or more of payload per model step and the engine has no use for it",
    },
    Ev {
        name: "PostLLMCall",
        act: Act::Off,
        matcher: None,
        timeout_s: 0,
        engine_ms: 0,
        why: "not registered: 9 KB or more of payload per model step and the engine has no use for it",
    },
    Ev {
        name: "PermissionRequest",
        act: Act::Off,
        matcher: None,
        timeout_s: 0,
        engine_ms: 0,
        why: "not registered: never fired in G0 (contract unknown) and a Muse hook cannot ask the user anyway",
    },
    Ev {
        name: "Notification",
        act: Act::Off,
        matcher: None,
        timeout_s: 0,
        engine_ms: 0,
        why: "not registered: never fired in G0, contract unknown, nothing to save",
    },
    Ev {
        name: "Interrupt",
        act: Act::Off,
        matcher: None,
        timeout_s: 0,
        engine_ms: 0,
        why: "not registered: never fired in G0; an interrupted tool call arrives as PostToolUseFailure with is_interrupt",
    },
    Ev {
        name: "SessionFork",
        act: Act::Off,
        matcher: None,
        timeout_s: 0,
        engine_ms: 0,
        why: "not registered: never fired in G0; a fork starts with SessionStart source fork, which is registered (D1)",
    },
];

/// What a hook prints, and what it starts after printing.
#[derive(Default)]
struct Done {
    stdout: Option<String>,
    /// (session id, cwd, prompt) for `suggest::trigger`: a main-session UserPromptSubmit the engine took.
    prompt: Option<(String, String, String)>,
}

/// With no TORPOR_HOME and a HOME that is unset, empty or not a full path, the state folder would be `.torpor/` in
/// whatever folder the hook runs in (the user's project).
fn home_usable() -> bool {
    let set = |k| std::env::var_os(k).filter(|v| !v.is_empty());
    set("TORPOR_HOME").is_some() || set("HOME").is_some_and(|h| Path::new(&h).is_absolute())
}

pub fn run(event: &str) -> u8 {
    let mut input = Vec::new();
    let read = std::io::stdin().lock().read_to_end(&mut input);
    let done = if !home_usable() {
        eprintln!(
            "torpor-muse: hook {}: TORPOR_HOME is not set and HOME is not a full path; Torpor is not saving this project",
            event.escape_default()
        );
        Done::default()
    } else {
        let dir = engine::state_dir();
        match read {
            Ok(_) => handle(&dir, event, &input),
            Err(e) => {
                state::problem(&dir, event, "", &format!("cannot read stdin: {e}"));
                Done::default()
            }
        }
    };
    if let Some(text) = &done.stdout {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{text}");
        let _ = out.flush();
    }
    if let Some((sid, cwd, prompt)) = done.prompt {
        // Muse waits for its pipe to close, and anything started now would inherit it: close ours first.
        close_stdio();
        suggest::trigger(&sid, &cwd, &prompt);
    }
    0
}

/// Points stdout and stderr at /dev/null, so nothing we start later can hold Muse's pipes open.
fn close_stdio() {
    // SAFETY: open(2) and dup2(2) on descriptors this process owns; the output was flushed before.
    unsafe {
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null >= 0 {
            libc::dup2(null, 1);
            libc::dup2(null, 2);
            if null > 2 {
                libc::close(null);
            }
        }
    }
}

fn handle(dir: &Path, event: &str, input: &[u8]) -> Done {
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
                done.stdout = writer_note(dir, event, done.stdout.take());
                done.prompt = Some((sid.clone(), cwd.clone(), prompt));
            }
            done.stdout = gap_note(dir, event, &sid, &cwd, done.stdout.take());
            let left = Duration::from_millis(ev.engine_ms).saturating_sub(started.elapsed());
            done.stdout = db_note(dir, event, &sid, &cwd, &out.bin, left, done.stdout.take());
        }
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
fn turn_ended(event: &str, stdout: Option<&str>) -> bool {
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

/// How the model should write `torpor` in a command: the engine's own path, because its shell may not have the
/// engine's folder on its PATH. Only for a file called `torpor` (a Torpor call is known by that name); else the word.
fn engine_cmd(bin: &Path) -> String {
    let named = bin.file_name().is_some_and(|n| n == "torpor")
        && bin.parent().is_some_and(|p| !p.as_os_str().is_empty());
    match std::path::absolute(bin) {
        Ok(p) if named => p.to_str().map_or("torpor".into(), crate::doctor::quote),
        _ => "torpor".into(),
    }
}

/// A prompt the check-writer could not start on for want of a key: tell the user once per state folder. The notice is
/// for the user only (`systemMessage`; doctor says it too): the model's context stays as it is, so a run without a key
/// gives the model the same text as one with a key. The marker is written only when the note really is printed.
fn writer_note(dir: &Path, event: &str, body: Option<String>) -> Option<String> {
    let Some(why) = suggest::writer_off(true) else {
        return body;
    };
    let why = suggest::scrub(&why, 300);
    let merged = schema::with_note(
        event,
        body.as_deref(),
        &format!(
            "Torpor's check-writer is off: {why}. To turn it on, put your Meta API key on one line in a file only you can \
             read (chmod 600) and add `key_file = ~/path/to/that/file` to ~/.config/torpor-muse/config (chmod 600). \
             `torpor-muse doctor` shows this too."
        ),
        "",
        false,
    );
    match merged {
        Some(m) if state::new_key(dir, event, "writer-off") => Some(m),
        _ => body,
    }
}

/// L3 N4: what a check the writer saved does not cover (`suggest::gaps`). The user reads it short inside the engine's
/// line that announces the promise (a line of its own would fall past the width of Muse's one-line status bar). Some =
/// the engine's Stop output with it put in.
fn gap_line(event: &str, sid: &str, cwd: &str, engine_out: &[u8]) -> Option<Vec<u8>> {
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

/// `torpor undo N` and a plain `torpor back` leave database content as it is, and the engine's own lines name only those
/// (dogfood1 M1). So when a turn's own commands changed or deleted a database file, its Stop line starts with the
/// command that does put the data back (L3 N2: Muse shows the line once, on one line cut at the terminal's width), and
/// the turn is kept for its folder past SessionEnd (L3 N1, `state::db_add`). The model of every session in that folder
/// gets all kept turns, each with its command (with the engine's path, see `engine_cmd`), once: at the start of the
/// session (startup, resume, clear, compact) or at the next prompt after a newer one. Costs two engine calls at Stop,
/// within `left` of the event's engine time; skipped when that is short.
fn db_note(
    dir: &Path,
    event: &str,
    sid: &str,
    cwd: &str,
    bin: &Path,
    left: Duration,
    body: Option<String>,
) -> Option<String> {
    if matches!(event, "SessionStart" | "UserPromptSubmit") {
        let turns = if cwd.is_empty() {
            Vec::new()
        } else {
            state::db_turns(dir, cwd)
        };
        let Some(newest) = turns.first().map(|t| t.n) else {
            return body;
        };
        let told = state::with_session(dir, sid, |s| s.db_told).unwrap_or(0);
        if event == "UserPromptSubmit" && newest <= told {
            return body;
        }
        let mut note = String::from(
            "[Torpor database note: later Torpor notes do not replace this one] `torpor undo` and a plain `torpor \
             back` leave existing database files as they are. Each command below brings a database's old data back, \
             even if the app made a fresh database since. Run one only when the user asks for it.",
        );
        for t in &turns {
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
        return match schema::with_note(event, body.as_deref(), "", &note, false) {
            Some(m) => {
                state::with_session(dir, sid, |s| s.db_told = newest);
                Some(m)
            }
            None => body,
        };
    }
    let t = left.min(Duration::from_secs(4)) / 2;
    if event != "Stop"
        || cwd.is_empty()
        || t < Duration::from_millis(250)
        || !turn_ended(event, body.as_deref())
    {
        return body;
    }
    let json = |args: &[&str]| -> Option<Value> {
        let o = engine::exec(bin, args, None, Some(Path::new(cwd)), t).ok()?;
        serde_json::from_slice(&o.stdout)
            .ok()
            .filter(|_| o.code == 0)
    };
    // not saving (`torpor off`): the last turn is an old one, already told
    let status = json(&["status", "--json"]).filter(|s| s["saving"] == true);
    let Some(n) = status.and_then(|s| s["last_turn"]["n"].as_u64()) else {
        return body;
    };
    let Some(show) = json(&["show", &n.to_string(), "--json"]) else {
        return body;
    };
    // the engine's own test for a database file (its scan.rs): .db, .sqlite, .sqlite3, any case
    let dbs: Vec<String> = show["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["class"] != "outside" && f["change"] != "new")
        .filter_map(|f| f["path"].as_str())
        .filter(|p| {
            [".db", ".sqlite", ".sqlite3"]
                .iter()
                .any(|x| p.to_lowercase().ends_with(x))
        })
        .map(String::from)
        .collect();
    let (Some(p), false) = (show["turn"]["start_point"].as_u64(), dbs.is_empty()) else {
        return body;
    };
    // the user's line: the command within its first 100 characters; long names are cut, never the command
    let (head, mut tail) = (
        format!("Torpor: turn {n} changed "),
        format!(". Get it back: stop the app, then torpor back --id {p} --with-data"),
    );
    let room = 100usize.saturating_sub(head.chars().count() + tail.chars().count());
    let mut names = dbs.join(", ");
    if names.chars().count() > room {
        names = names
            .chars()
            .take(room.saturating_sub(2))
            .collect::<String>()
            + "...";
        tail.remove(0); // the "..." ends the sentence
    }
    state::db_add(dir, cwd, state::DbTurn { n, p, files: dbs });
    schema::with_note(
        event,
        body.as_deref(),
        &format!("{head}{names}{tail}"),
        "",
        true,
    )
    .or(body)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_muse_event_is_in_the_table_once_with_a_reason() {
        let mut names: Vec<&str> = EVENTS.iter().map(|e| e.name).collect();
        names.sort();
        let mut want = schema::EVENT_NAMES.to_vec();
        want.sort();
        assert_eq!(names, want, "table and schema must name the same 19 events");
        assert_eq!(names.len(), 19);
        for e in EVENTS {
            assert!(e.why.len() >= 20, "{} has no real reason", e.name);
        }
    }

    #[test]
    fn settings_timeouts_are_whole_seconds_and_outlast_the_engine_kill_time() {
        for e in EVENTS.iter().filter(|e| e.act != Act::Off) {
            assert!(e.timeout_s >= 1, "{}", e.name);
            if e.act == Act::Engine {
                // the kill time, plus the pipe grace (0.3 s), the state lock (0.5 s) and the transcript step
                assert!(
                    e.engine_ms + 2_000 <= u64::from(e.timeout_s) * 1_000,
                    "{}: Muse would cut us off before our own kill",
                    e.name
                );
            }
        }
        let stop = EVENTS.iter().find(|e| e.name == "Stop").unwrap();
        assert_eq!(stop.timeout_s, 35);
        assert!(
            stop.engine_ms > 30_000,
            "the gate may use 20 s plus its save"
        );
    }

    #[test]
    fn matchers_only_where_muse_allows_them() {
        for e in EVENTS {
            let tool = matches!(e.name, "PreToolUse" | "PostToolUse" | "PostToolUseFailure");
            if tool {
                assert_eq!(e.matcher, TOOLS, "{}", e.name);
            } else if e.name == "SessionStart" {
                assert!(e.matcher.unwrap().split('|').any(|m| m == "fork"));
            } else {
                assert!(e.matcher.is_none(), "{}", e.name);
            }
        }
        assert!(EVENTS.iter().all(|e| e.name != "state"));
    }

    #[test]
    fn the_model_is_given_the_engines_own_path_only_when_that_is_safe_to_run() {
        let cmd = |p: &str| engine_cmd(Path::new(p));
        assert_eq!(cmd("/opt/engine/torpor"), "/opt/engine/torpor");
        assert_eq!(cmd("/opt/my engine/torpor"), "'/opt/my engine/torpor'");
        // a file with another name is not known as a Torpor call; a bare name has no folder to give
        assert_eq!(cmd("/opt/engine/torpor-2.3.1"), "torpor");
        assert_eq!(cmd("torpor"), "torpor");
    }
}

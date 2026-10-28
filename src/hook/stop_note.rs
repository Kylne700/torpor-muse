//! The one Stop block of a turn: what broke, what was deleted or lost, and what is not being saved.

use super::handle::turn_ended;
use super::paths::{engine_cmd, project_root};
use super::{done, state, stop};
use crate::{engine, schema, suggest};
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

/// The Stop that just ended, as `stop_note` needs it.
pub(super) struct Ended<'a> {
    pub(super) dir: &'a Path,
    pub(super) sid: &'a str,
    pub(super) turn: &'a str,
    pub(super) cwd: &'a str,
    pub(super) bin: &'a Path,
    /// What is left of the event's engine time.
    pub(super) left: Duration,
    /// The AI's last message, to read for a false Done (`done::said`; empty at the Stop that follows a block).
    pub(super) said: &'a str,
}

/// The user's line for a database turn (L3 N2): at most 100 characters, the command at its end is never cut, long names
/// are.
fn db_line(n: u64, names: &[String], tail: &str) -> String {
    let head = format!("Torpor: turn {n} changed ");
    let mut tail = tail.to_string();
    let room = 100usize.saturating_sub(head.chars().count() + tail.chars().count());
    let mut names = names.join(", ");
    if names.chars().count() > room {
        names = names
            .chars()
            .take(room.saturating_sub(2))
            .collect::<String>()
            + "...";
        tail.remove(0); // the "..." ends the sentence
    }
    format!("{head}{names}{tail}")
}

/// A Stop at which the engine's `status --json` says it is not saving, told to the user alone, once per session
/// (`Session.told`, shared with `fail_open`; set only when the line goes out). The engine's reasons are about Claude
/// Code (its settings, its sessions' hooks) except one, and Muse's hooks make save points without those, so only the
/// disk that is almost full is told, and nothing after `torpor off`.
fn not_saving(e: &Ended, status: &Value, body: Option<String>) -> Option<String> {
    let disk = status["problems"].as_array().and_then(|p| {
        p.iter()
            .filter_map(Value::as_str)
            .find(|p| p.starts_with("Save points are paused"))
    });
    let Some(why) = disk.filter(|_| status["off"] != true) else {
        return body;
    };
    let line = format!(
        "Torpor is not saving this folder: {}",
        suggest::scrub(why, 200)
    );
    let Some(note) = schema::with_note("Stop", body.as_deref(), &line, "", true) else {
        return body;
    };
    let first = |s: &mut state::Session| !std::mem::replace(&mut s.told, true);
    if state::with_session(e.dir, e.sid, first).unwrap_or(false) {
        Some(note)
    } else {
        body
    }
}

/// What the turn that just ended did, from the engine's own records, told to the user and to the model. Only when the
/// engine did not block the turn itself (its reason says the same), and not after `torpor off`.
/// - The user's line (L3 N2), when the turn's own commands changed or deleted a database file: it starts with the command
///   that does put the data back, for `torpor undo N` and a plain `torpor back` leave database content as it is (dogfood1
///   M1). The turn is kept for its folder past SessionEnd (L3 N1, `state::db_add`) and `db_note` gives it to the model.
/// - ONE Stop block per turn (F4 items 2 and 3; `Session::allow_note_block`, inside the cap of `allow_block`): Muse runs
///   the turn on with the reason (`stop::reason`), which makes the model say to the user in its reply what is broken or
///   what was deleted, and what to run. For a promise that failed after this turn (the engine's own fail line in its Stop
///   text, `stop::fails`: advisory or shadow; the engine has no other form of it) and for a turn that deleted a file that
///   existed before it or deleted a database (`stop::rows`: a changed one only gets the user's line).
///
/// Costs the `status` and `show` calls of L3, plus up to 3 dry runs of `back` to find a save point that has a database
/// that was in use when the turn began and is gone now (`stop::restoring_point`): all within 4 s and `left`. A call that
/// is cut short leaves out what it would have told: the fail-line block still goes out, with `find-break` for the turn.
/// The Stop that follows the block, in the same turn, costs none of this and adds nothing.
pub(super) fn stop_note(e: &Ended, engine_out: &[u8], body: Option<String>) -> Option<String> {
    if e.cwd.is_empty() || !turn_ended("Stop", body.as_deref()) {
        return body;
    }
    // Muse runs a blocked turn on and stops again in the same turn: that Stop has told its one block already, so it asks
    // the engine nothing and says nothing twice (F4c)
    let told = |s: &mut state::Session| s.noted == e.turn;
    if !e.turn.is_empty() && state::with_session(e.dir, e.sid, told).unwrap_or(false) {
        return body;
    }
    let fails = serde_json::from_slice::<Value>(engine_out)
        .ok()
        .and_then(|v| v["systemMessage"].as_str().map(stop::fails))
        .unwrap_or_default();
    let end = Instant::now() + e.left.min(Duration::from_secs(4));
    let ask = |args: &[&str]| {
        let t = end
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(2));
        if t < Duration::from_millis(250) {
            return None;
        }
        engine::exec(e.bin, args, None, Some(Path::new(e.cwd)), t)
            .ok()
            .filter(|o| o.code == 0)
    };
    let json =
        |args: &[&str]| ask(args).and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok());
    let status = json(&["status", "--json"]);
    // not saving: the last turn is an old one, already told
    if let Some(s) = status.as_ref().filter(|s| s["saving"] != true) {
        return not_saving(e, s, body);
    }
    // shortcut: `last_turn` is the folder's, so with two sessions in one folder it can be the other session's turn, and this
    // Stop then tells about that one; upgrade when the engine can name the last turn of a session.
    let saving = status.is_some();
    let n = status.and_then(|s| s["last_turn"]["n"].as_u64());
    let show = n.and_then(|n| json(&["show", &n.to_string(), "--json"]));
    let root = project_root(e.cwd);
    let rows = show.as_ref().map_or_else(stop::Rows::default, |s| {
        stop::rows(s, |p| root.join(p).symlink_metadata().is_err())
    });
    let start = show
        .as_ref()
        .and_then(|s| s["turn"]["start_point"].as_u64());
    // The save point to name. A database that was in use when the turn began has no copy at its start; the save point a
    // command of the turn took after the app stopped may have one (the busy-database stop, drill D07).
    let (mut point, mut lost) = (start, rows.lost.clone());
    if let (false, Some(s)) = (lost.is_empty(), show.as_ref()) {
        let dry = |p: u64| {
            let id = p.to_string();
            ask(&["back", "--id", &id, "--with-data", "--dry-run"])
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        };
        if let Some(p) = stop::restoring_point(&stop::pre_points(s), &lost, dry) {
            (point, lost) = (Some(p), Vec::new());
        }
    }
    let kept: Vec<String> = rows
        .dbs
        .iter()
        .filter(|d| !lost.contains(d))
        .cloned()
        .collect();
    let line = match (n, point) {
        (Some(n), Some(p)) if !kept.is_empty() => {
            state::db_add(
                e.dir,
                e.cwd,
                state::DbTurn {
                    n,
                    p,
                    files: kept.clone(),
                },
            );
            let tail =
                format!(". Get it back: stop the app, then torpor back --id {p} --with-data");
            Some(db_line(n, &kept, &tail))
        }
        (Some(n), _) if !lost.is_empty() => {
            Some(db_line(n, &lost, ". Torpor has no copy: it was in use."))
        }
        _ => None,
    };
    let body = match line {
        Some(l) => schema::with_note("Stop", body.as_deref(), &l, "", true).or(body),
        None => body,
    };
    let broke: Vec<stop::Broke> = fails
        .into_iter()
        .map(|f| stop::Broke {
            turn: n.filter(|&n| stop::broke_in(&f.passed_on, n, start)),
            id: f.id,
            name: f.name,
        })
        .collect();
    let risk = stop::Risk {
        back: point.filter(|_| rows.risky.iter().any(|d| !lost.contains(d))),
        deleted: rows.deleted,
        dbs: rows.risky,
        lost,
    };
    // a false Done (`done`) joins the one block; without a status the engine may be off, and then nothing is said
    let ours = if saving {
        done::verdict(e.dir, e.sid, e.turn, e.said)
    } else {
        None
    };
    let Some(reason) = done::join(stop::reason(&engine_cmd(e.bin), &broke, &risk), ours) else {
        return body;
    };
    // the block is built before the turn is marked as told: a block that cannot go out must not use up the turn's one
    let Some(block) = schema::block(body.as_deref(), &reason) else {
        return body;
    };
    // a state file that cannot be locked cannot tell whether this turn was told already: no block
    if !state::with_session(e.dir, e.sid, |s| s.allow_note_block(e.turn)).unwrap_or(false) {
        return body;
    }
    Some(block)
}

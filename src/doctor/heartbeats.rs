//! The gate is live only when a hook ran in a finished turn: reads the hooks' heartbeats and says what is missing.

use super::ctx::Ctx;
use super::hooks::Hook;
use super::problems::engine_down_since;
use super::report::{Level, Report, ago, short};
use super::tested;
use crate::liveness::{self, PROBE_SID};
use crate::transcript;
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn check_gate(c: &Ctx, vault: &Path, ours: &[&Hook], r: &mut Report) {
    if ours.is_empty() {
        return;
    }
    let (main, obs) = liveness::last_by_event(vault, 20);
    if main.is_empty() {
        r.add(
            Level::Info,
            format!(
                "no heartbeat yet in {}: no hook has run since the install",
                vault.display()
            ),
        );
    } else {
        let list: Vec<String> = main
            .iter()
            .map(|(e, (t, _, n))| format!("{e} {} ago ({n})", ago(c.now_ms, *t)))
            .collect();
        r.add(
            Level::Info,
            format!(
                "last heartbeat per event, main sessions: {}",
                list.join(", ")
            ),
        );
    }
    if !obs.is_empty() {
        let list: Vec<String> = obs
            .iter()
            .map(|(e, (t, _, n))| format!("{e} {n}x (last {} ago)", ago(c.now_ms, *t)))
            .collect();
        r.add(
            Level::Info,
            format!(
                "observer (child) sessions, counted apart: {}",
                list.join(", ")
            ),
        );
    }
    let silent: Vec<&str> = ours
        .iter()
        .map(|h| h.event.as_str())
        .filter(|e| !main.contains_key(*e))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !silent.is_empty() && !main.is_empty() {
        r.add(
            Level::Info,
            format!("registered but no heartbeat yet: {}", silent.join(", ")),
        );
    }
    let found = liveness::sessions(vault)
        .into_iter()
        .filter(|s| s != PROBE_SID)
        .find_map(|sid| {
            let b = liveness::read_beats(vault, &sid);
            liveness::is_main(&b).then_some((sid, b))
        });
    let how =
        "send one prompt in Muse, wait until it has answered, then run torpor-muse doctor again";
    let Some((sid, beats)) = found else {
        r.problem(
            Level::Fix,
            "gate not proven live: no heartbeat from a main session",
            how,
        );
        return;
    };
    let t = liveness::turns(&beats);
    let last_stop = beats.iter().rev().find(|b| b.e == "Stop").map(|b| b.t);
    let written = fs::metadata(&c.settings)
        .and_then(|m| m.modified())
        .ok()
        .map(ms_of);
    if !liveness::gate_live(&beats) {
        r.problem(
            Level::Fix,
            format!("gate not proven live: session {} has {} prompt(s) and no finished turn (no Stop heartbeat from a previous turn)", short(&sid), t.opened),
            format!("{how}; if it stays so, the Stop hook does not run: look at the timeouts and any rejected outputs above"),
        );
    } else if let (Some(stop), Some(w)) = (last_stop, written)
        && stop < w
    {
        r.problem(
            Level::Fix,
            format!("gate not proven live since the settings changed: the last Stop heartbeat is {} old, the settings were written {} ago", ago(c.now_ms, stop), ago(c.now_ms, w)),
            how,
        );
    } else if let Some(stop) = last_stop
        && engine_down_since(vault, stop)
    {
        // the heartbeat is written before the engine is called, so a Stop that failed open still left one
        r.problem(
            Level::Fix,
            format!("gate not live: a hook call at or after the last Stop ({} ago, session {}) went through without the engine, see the hook problem below", ago(c.now_ms, stop), short(&sid)),
            format!("fix the hook problem below; after that {how}"),
        );
    } else {
        r.add(
            Level::Ok,
            format!(
                "gate live: Stop ran {} ago in session {} ({} finished turn(s))",
                ago(c.now_ms, last_stop.unwrap_or(0)),
                short(&sid),
                t.closed
            ),
        );
    }
    if t.no_stop > 0 {
        r.problem(
            Level::Warn,
            format!("{} turn(s) of session {} ended without a Stop (interrupted, step limit, or the Stop hook did not run)", t.no_stop, short(&sid)),
            "if it was not an interrupt: read shim-errors.log in the vault's muse folder and the hook_run records in Muse's log",
        );
    }
    if t.extra_stops > 0 {
        r.add(
            Level::Info,
            format!(
                "{} extra Stop(s) in session {}: Muse runs Stop again after a blocking Stop and when it double-checks its answer, normal",
                t.extra_stops,
                short(&sid)
            ),
        );
    }
    let armed: HashSet<&str> = ours.iter().map(|h| h.event.as_str()).collect();
    for u in liveness::unpaired(&beats, |e| armed.contains(e)) {
        r.problem(
            Level::Warn,
            format!("events without their pair in session {}: {u}", short(&sid)),
            "a hook of that pair failed or timed out (Muse's rejected outputs, if any, are listed above), or the turn was stopped while a tool ran",
        );
    }
    if let Some(st) = transcript::read_state(vault, &sid) {
        if st.lines > 200 && st.users == 0 {
            r.problem(
                Level::Fix,
                format!("{} lines of Muse's log for session {} hold no prompt: its log format may have changed", st.lines, short(&sid)),
                format!("check the Muse Code version (tested only with {}); until the translation is updated, origin and compaction checks are blind", tested(" and ")),
            );
        }
        if st.bad > 0 {
            r.problem(Level::Warn, format!("{} line(s) of Muse's log for session {} had an unexpected shape and were skipped", st.bad, short(&sid)), "check the Muse Code version");
        }
    }
}

fn ms_of(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

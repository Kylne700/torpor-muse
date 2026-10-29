//! What the hooks logged themselves: shim-errors.log and problems.log, and which lines mean a call went unchecked.

use super::report::{Level, Report, ago, clip};
use super::tested;
use crate::liveness;
use crate::util::quote;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub(crate) fn check_errors(vault: &Path, now: u64, r: &mut Report) {
    let f = liveness::shim_dir(vault).join("shim-errors.log");
    let Ok(text) = fs::read_to_string(&f) else {
        return;
    };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let Some(last) = lines.last() else { return };
    let (t, msg) = last.split_once(' ').unwrap_or(("0", last));
    r.problem(
        Level::Warn,
        format!(
            "{} error(s) logged by the hooks, the last {} ago: {}",
            lines.len(),
            ago(now, t.parse().unwrap_or(0)),
            clip(msg, 160)
        ),
        format!("read {}", f.display()),
    );
}

/// How far back a line of problems.log still counts.
const PROBLEMS_RECENT_S: u64 = 24 * 3600;
/// Core keeps the file near 32 KiB, newest last; doctor never reads more than this from its end.
pub const PROBLEMS_READ: u64 = 128 * 1024;

/// The last `max` bytes of a file, from the start of a whole line.
fn tail_of(path: &Path, max: u64) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(max))).ok()?;
    let mut b = Vec::new();
    f.read_to_end(&mut b).ok()?;
    let text = String::from_utf8_lossy(&b).into_owned();
    if len > max {
        return Some(
            text.split_once('\n')
                .map_or(String::new(), |x| x.1.to_string()),
        );
    }
    Some(text)
}

/// What one line of problems.log is worth. FIX: the engine did not run (missing, wrong version, killed at its
/// deadline, exited with an error) or the hook's input was unusable: that call went through without a check. info:
/// routine notes. warn: the rest (an output Muse would reject was cut, Muse's payload changed shape, an unknown message).
fn problem_level(msg: &str) -> Level {
    const DOWN: [&str; 9] = [
        "torpor not found",
        "torpor engine ",
        "torpor hook ",
        "cannot run ",
        "engine.lock has no version",
        "cannot read stdin",
        "stdin is not a JSON object",
        "payload has no session_id",
        "unknown event name",
    ];
    const ROUTINE: [&str; 3] = [
        "Stop block dropped",
        "is not in our schema",
        "gate default not set", // the next session start tries again; the line about the gate says what it is now
    ];
    if DOWN.iter().any(|d| msg.starts_with(d)) || msg.contains(" timed out after ") {
        Level::Fix
    } else if ROUTINE.iter().any(|q| msg.contains(q)) {
        Level::Info
    } else {
        Level::Warn
    }
}

/// Did a hook call at or after `since_ms` (to the second) go through without the engine? Same lines as the fixes below.
pub(crate) fn engine_down_since(vault: &Path, since_ms: u64) -> bool {
    let f = liveness::shim_dir(vault).join("problems.log");
    tail_of(&f, PROBLEMS_READ).is_some_and(|text| {
        text.lines().any(|l| {
            let mut w = l.splitn(4, ' ');
            let t = w.next().and_then(|t| t.parse::<u64>().ok());
            matches!((t, w.nth(2)), (Some(t), Some(msg))
                if t >= since_ms / 1000 && problem_level(msg) == Level::Fix)
        })
    })
}

/// (unix seconds, event, message) of one problems.log line.
type Line<'a> = (u64, &'a str, &'a str);

/// `<vault>/muse/problems.log` (written by the hook core): `<unix seconds> <Event> <session, 8 chars> <message>`.
pub(crate) fn check_problems(vault: &Path, now_ms: u64, r: &mut Report) {
    let f = liveness::shim_dir(vault).join("problems.log");
    let Some(text) = tail_of(&f, PROBLEMS_READ) else {
        return;
    };
    let now_s = now_ms / 1000;
    let (mut fix, mut warn): (Vec<Line>, Vec<Line>) = (vec![], vec![]);
    let (mut routine, mut old) = (0, 0);
    for line in text.lines() {
        let mut w = line.splitn(4, ' ');
        let (Some(t), Some(event), Some(_session), Some(msg)) = (
            w.next().and_then(|t| t.parse::<u64>().ok()),
            w.next(),
            w.next(),
            w.next(),
        ) else {
            continue;
        };
        if now_s.saturating_sub(t) > PROBLEMS_RECENT_S {
            old += 1;
            continue;
        }
        match problem_level(msg) {
            Level::Fix => fix.push((t, event, msg)),
            Level::Info => routine += 1,
            _ => warn.push((t, event, msg)),
        }
    }
    let newest = |v: &[Line]| {
        let &(t, event, msg) = v.iter().max_by_key(|x| x.0).unwrap_or(&(0, "", ""));
        format!(
            "the last {} ago on {event}: {}",
            ago(now_ms, t * 1000),
            clip(msg, 200)
        )
    };
    if !fix.is_empty() {
        r.problem(
            Level::Fix,
            format!(
                "{} hook call(s) went through without a check in the last 24 h (the engine did not run, or the hook's input was unusable), {}",
                fix.len(),
                newest(&fix)
            ),
            format!(
                "fix what that line says (an engine that is missing or of another version: install the pinned one, then torpor-muse init); all lines: {0}; when fixed, empty it so this check starts clean: : > {1}",
                f.display(),
                quote(&f.to_string_lossy())
            ),
        );
    }
    if !warn.is_empty() {
        r.problem(
            Level::Warn,
            format!(
                "{} hook note(s) in the last 24 h, {}",
                warn.len(),
                newest(&warn)
            ),
            format!(
                "read {}: an output Muse would reject was cut, or Muse's payload changed shape (tested only with Muse Code {})",
                f.display(),
                tested(" and ")
            ),
        );
    }
    if fix.is_empty() && warn.is_empty() {
        r.add(
            Level::Ok,
            format!(
                "no problem logged by the hooks in the last 24 h ({routine} routine and {old} older line(s) in {})",
                f.display()
            ),
        );
    }
}

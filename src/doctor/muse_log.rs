//! Muse's own session logs: the Muse version, hooks it rejected, and prompts that never reached the prompt hook.

use super::MUSE_TESTED;
use super::ctx::Ctx;
use super::hooks::Hook;
use super::report::{Level, Report, clip, short};
use crate::transcript;
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const LOGS_SCANNED: usize = 6;

#[derive(Deserialize)]
struct LogRec {
    /// Microseconds since the epoch.
    recorded_at: Option<u64>,
    payload: Option<LogPayload>,
}

#[derive(Deserialize)]
struct LogPayload {
    /// The turn (Muse calls it the run) a hook run belongs to.
    run_id: Option<String>,
    intent_id: Option<String>,
    semantic_kind: Option<Sem>,
    event: Option<LogEvent>,
    record: Option<Meta>,
}

#[derive(Deserialize)]
struct Sem {
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Meta {
    build: Option<Build>,
}

#[derive(Deserialize)]
struct Build {
    semver: Option<String>,
}

#[derive(Deserialize)]
struct LogEvent {
    event: Option<String>,
    hook_key: Option<String>,
    status: Option<String>,
    error: Option<String>,
    stdout: Option<String>,
}

struct HookRun {
    event: String,
    /// When Muse recorded the run (microseconds since the epoch).
    at: u64,
    status: String,
    error: String,
    stdout: String,
    log: PathBuf,
}

#[derive(Default)]
struct Scan {
    version: Option<String>,
    /// Newest run of every hook of the settings file, by hook_key.
    last: BTreeMap<String, HookRun>,
    /// (session folder, prompt) pairs that reached a Stop hook but never a UserPromptSubmit hook.
    no_ups: Vec<(String, String)>,
}

/// Reads one log once. `key_prefix`: the settings file path Muse puts in front of every hook_key.
fn scan_log(path: &Path, key_prefix: &str, scan: &mut Scan) {
    let Ok(f) = File::open(path) else { return };
    let mut rd = BufReader::new(f);
    let (mut line, mut n) = (Vec::new(), 0u32);
    let (mut intents, mut ups, mut stops) = (Vec::new(), HashSet::new(), HashSet::new());
    while rd.read_until(b'\n', &mut line).is_ok_and(|k| k > 0) {
        n += 1;
        let text = std::str::from_utf8(&line).unwrap_or("");
        let kind = if text.contains(r#""kind":"hook_run_terminal""#) {
            1
        } else if text.contains(r#""payload_type":"runtime.user_intent.accepted""#) {
            2
        } else if n <= 8 && text.contains(r#""payload_type":"runtime.session.metadata""#) {
            3
        } else {
            0
        };
        if kind != 0
            && let Ok(rec) = serde_json::from_slice::<LogRec>(&line)
            && let Some(p) = rec.payload
        {
            match (kind, p.event, p.intent_id) {
                (1, Some(ev), _)
                    if ev
                        .hook_key
                        .as_deref()
                        .is_some_and(|k| k.starts_with(key_prefix)) =>
                {
                    let (key, event) = (
                        ev.hook_key.unwrap_or_default(),
                        ev.event.unwrap_or_default(),
                    );
                    let run = p.run_id.unwrap_or_default();
                    match event.as_str() {
                        "UserPromptSubmit" => ups.insert(run),
                        "Stop" => stops.insert(run),
                        _ => false,
                    };
                    scan.last.insert(
                        key,
                        HookRun {
                            event,
                            at: rec.recorded_at.unwrap_or(0),
                            status: ev.status.unwrap_or_default(),
                            error: ev.error.unwrap_or_default(),
                            stdout: ev.stdout.unwrap_or_default(),
                            log: path.to_path_buf(),
                        },
                    );
                }
                // A live prompt has a version-7 id; a fork replays its parent's prompts with other ids.
                (2, _, Some(id))
                    if p.semantic_kind.and_then(|k| k.kind).as_deref() == Some("chat")
                        && id.as_bytes().get(14) == Some(&b'7') =>
                {
                    intents.push(id)
                }
                (3, _, _) => {
                    scan.version = scan
                        .version
                        .take()
                        .or(p.record.and_then(|r| r.build).and_then(|b| b.semver))
                }
                _ => {}
            }
        }
        line.clear();
    }
    let folder = path
        .parent()
        .and_then(Path::file_name)
        .map(|s| s.to_string_lossy().into_owned());
    for id in intents {
        if stops.contains(&id) && !ups.contains(&id) {
            scan.no_ups.push((folder.clone().unwrap_or_default(), id));
        }
    }
}

pub(crate) fn check_logs(c: &Ctx, ours: &[&Hook], r: &mut Report) {
    let mut logs = transcript::recent_logs(&c.logs, LOGS_SCANNED);
    if logs.is_empty() && c.logs != c.hook_logs {
        logs = transcript::recent_logs(&c.hook_logs, LOGS_SCANNED);
    }
    if logs.is_empty() {
        r.add(Level::Info, format!("no Muse session log under {}: Muse version and rejected outputs cannot be checked yet", c.logs.display()));
        return;
    }
    let prefix = c.settings.to_string_lossy().into_owned();
    // Oldest first, so that the newest run of a hook wins.
    let (mut scan, mut version) = (Scan::default(), None);
    for p in logs.iter().rev() {
        let before = scan.version.take();
        scan_log(p, &prefix, &mut scan);
        version = scan.version.take().or(before).or(version);
    }
    match version.as_deref() {
        Some(v) if v == MUSE_TESTED => r.add(Level::Ok, format!("Muse Code {v} (the version these hooks were tested with)")),
        Some(v) => r.problem(
            Level::Warn,
            format!("Muse Code {v}: the hooks and the log translation were tested only with {MUSE_TESTED}"),
            format!("if something below fails, install Muse Code {MUSE_TESTED}"),
        ),
        None => r.add(Level::Info, "Muse Code version not found in its newest session logs"),
    }
    // The key of a hook ends in a hash of its definition, so a repaired hook is a new key and the old key's failure is
    // the newest record it will ever have. That failure is history once the settings changed after it and the same event
    // has run fine since (a hook of the same event that never changed keeps its own newer record, so it is still found).
    let changed = fs::metadata(&c.settings)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(u64::MAX, |d| d.as_micros() as u64);
    for (key, run) in &scan.last {
        if run.status != "failed" && run.status != "timed_out" {
            continue;
        }
        if run.at < changed
            && scan.last.values().any(|o| {
                o.event == run.event
                    && o.at > changed
                    && matches!(o.status.as_str(), "completed" | "blocked")
            })
        {
            continue;
        }
        let saw = if run.stdout.is_empty() {
            String::new()
        } else {
            format!(" saw: {}", clip(&run.stdout, 100))
        };
        let sid = run
            .log
            .parent()
            .and_then(Path::file_name)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let err = if run.error.is_empty() {
            "no reason given".to_string()
        } else {
            run.error.clone()
        };
        r.problem(
            Level::Fix,
            format!(
                "Muse rejected a hook in {}: {} {} ({err}){saw}\n      seen only in Muse's session log (record hook_run_terminal, status {}): {}",
                c.settings.display(),
                run.event,
                run.status,
                run.status,
                run.log.display()
            ),
            if run.status == "timed_out" {
                "raise the timeout of that hook (torpor-muse init) or find out why it is slow; Muse kills it and goes on without its answer".to_string()
            } else {
                format!("run that hook's command from the settings file by hand with a payload on stdin and read its stdout: it must be one JSON object with camelCase keys only (an unknown key rejects the whole output); the record is in the log of session {sid}, hook {key}")
            },
        );
    }
    let ups_installed = ours.iter().any(|h| h.event == "UserPromptSubmit");
    if ups_installed {
        for (sid, id) in &scan.no_ups {
            r.problem(
                Level::Fix,
                format!("Muse took the prompt {} (session {}) and ran its Stop hook, but the UserPromptSubmit hook never ran: Torpor did not see that prompt", short(id), short(sid)),
                "check the UserPromptSubmit entry in the settings (torpor-muse init rewrites it) and the hook_run records in that session's log",
            );
        }
    }
}

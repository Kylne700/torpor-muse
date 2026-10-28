//! The hooks in Muse's settings file: reads them and checks that the three the gate needs are there and sound.

use super::Ctx;
use super::hook_command::tool_of;
use super::report::{Level, Report};
use crate::hook::EVENTS;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Without these three the gate cannot work.
const REQUIRED: [&str; 3] = ["SessionStart", "UserPromptSubmit", "Stop"];

pub struct Hook {
    pub event: String,
    pub command: String,
    pub timeout: Option<u64>,
    /// A `timeout` that is not a whole number of seconds: Muse refuses or drops it.
    pub timeout_bad: bool,
    pub is_async: bool,
}

/// Err((file is missing, message)).
pub(crate) fn read_hooks(path: &Path) -> Result<Vec<Hook>, (bool, String)> {
    let text = fs::read_to_string(path).map_err(|e| {
        let missing = e.kind() == std::io::ErrorKind::NotFound;
        let what = if missing {
            format!(
                "no Muse settings file at {}: no torpor-muse hook is installed",
                path.display()
            )
        } else {
            format!("cannot read {}: {e}", path.display())
        };
        (missing, what)
    })?;
    let v: Value = serde_json::from_str(&text).map_err(|e| {
        (
            false,
            format!(
                "{} is not valid JSON ({e}): Muse ignores its hooks",
                path.display()
            ),
        )
    })?;
    let mut out = Vec::new();
    for (event, groups) in v
        .get("hooks")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        for g in groups.as_array().into_iter().flatten() {
            for h in g
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(command) = h.get("command").and_then(Value::as_str) else {
                    continue;
                };
                let t = h.get("timeout");
                out.push(Hook {
                    event: event.clone(),
                    command: command.to_string(),
                    timeout: t.and_then(Value::as_u64),
                    timeout_bad: t.is_some_and(|t| !t.is_u64()),
                    is_async: h.get("async").and_then(Value::as_bool) == Some(true),
                });
            }
        }
    }
    Ok(out)
}

pub(crate) fn check_hooks(c: &Ctx, ours: &[&Hook], r: &mut Report) {
    if ours.is_empty() {
        // when `init` would refuse the file, the reason comes first: "run init" would only meet the same refusal
        match crate::install::refusal(&c.settings) {
            Some(why) => r.problem(
                Level::Fix,
                format!("no torpor-muse hook can be installed: {why}"),
                "fix what is named above (init does not edit a symbolic link, and it leaves a file Muse Code would reject as it is), then run torpor-muse init",
            ),
            None => r.problem(
                Level::Fix,
                format!(
                    "no torpor-muse hook in {}: the gate is not installed",
                    c.settings.display()
                ),
                "torpor-muse init",
            ),
        }
        return;
    }
    let events: BTreeSet<&str> = ours.iter().map(|h| h.event.as_str()).collect();
    for need in REQUIRED {
        if !events.contains(need) {
            r.problem(
                Level::Fix,
                format!(
                    "no torpor-muse hook for {need} in {}: the gate needs it",
                    c.settings.display()
                ),
                "torpor-muse init",
            );
        }
    }
    let mut seconds: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for h in ours {
        let t = h.timeout.map_or("default".to_string(), |t| format!("{t}s"));
        seconds.entry(&h.event).or_default().push(t);
        if h.is_async {
            r.problem(
                Level::Fix,
                format!("the {} hook is async: Muse does not wait for it, so nothing it says reaches the model", h.event),
                "torpor-muse init (it writes the hooks as synchronous)",
            );
        }
        if h.timeout_bad {
            r.problem(
                Level::Fix,
                format!("the {} hook has a timeout that is not a whole number of seconds: Muse refuses or drops it", h.event),
                "torpor-muse init",
            );
        }
        // `engine_ms` is the engine's own limit plus 1.5 s of grace; Muse's timeout has to cover the limit plus 1 s.
        let need = EVENTS
            .iter()
            .find(|e| e.name == h.event && e.engine_ms > 0)
            .map(|e| e.engine_ms.saturating_sub(500));
        if let (Some(t), Some(need)) = (h.timeout, need)
            && t * 1000 < need
        {
            r.problem(
                Level::Warn,
                format!("the {} timeout ({t}s) is shorter than the engine's own limit plus start-up ({}s): Muse can kill the hook before the engine answers", h.event, need.div_ceil(1000)),
                "torpor-muse init",
            );
        }
    }
    let list: Vec<String> = seconds
        .iter()
        .map(|(e, t)| format!("{e} {}", t.join("/")))
        .collect();
    r.add(Level::Info, format!("hook timeouts: {}", list.join(", ")));
    for h in ours {
        if let Some(bin) = tool_of(&h.command, "TORPOR_BIN")
            && !bin.is_file()
        {
            r.problem(
                Level::Fix,
                format!("the engine {} (TORPOR_BIN in the {} hook) does not exist: every hook then runs without a check", bin.display(), h.event),
                "install Torpor again, then run torpor-muse init",
            );
            break;
        }
    }
}

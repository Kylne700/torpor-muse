//! The gate default for a new folder: set once per folder, and only where Torpor is saving.

use super::paths::project_root;
use super::state;
use crate::engine;
use serde_json::Value;
use std::path::Path;
use std::time::{Duration, Instant};

/// The engine's gate is ON in every folder it saves. The user chose hands-free (F4b): a broken check is told to the AI
/// (`stop_note`) and stops it only after the user runs `torpor gate on` in their own terminal. So the first session start
/// in a saved folder runs `torpor gate off` there; `state::mark` (`gate`) then keeps it from running again, which is what lets
/// a later `gate on` of the user's stay on. The model cannot undo either: the engine's PreToolUse refuses `torpor gate ...`
/// from its Bash, and the mark is a file in the engine's store, where it refuses writes too. Nothing goes to the model;
/// a failure is one line in problems.log (doctor shows it) and the next session start tries again.
/// Order: `gate off` first, then the mark. A hook killed in between repeats `gate off` once (it does the same twice); the
/// other order would leave a mark with the gate still on, and the user's choice would never apply.
/// shortcut: a folder where the user typed `gate on` before this version has no mark and is switched off once (the engine
/// cannot tell the default on from the user's on), upgrade if the engine ever tells them apart.
pub(super) fn gate_default(dir: &Path, sid: &str, cwd: &str, bin: &Path, left: Duration) {
    if cwd.is_empty() {
        return;
    }
    let root = project_root(cwd);
    let key = root
        .canonicalize()
        .unwrap_or(root)
        .to_string_lossy()
        .into_owned();
    if state::marked(dir, "gate", &key) {
        return;
    }
    let end = Instant::now() + left;
    let ask = |args: &[&str], unset: &[&str]| {
        let t = end
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(2));
        if t < Duration::from_millis(250) {
            return None; // out of time: not a failure, the next start tries
        }
        Some(engine::exec_without(
            bin,
            args,
            None,
            Some(Path::new(cwd)),
            t,
            unset,
        ))
    };
    // only a folder the engine is saving: after `torpor off`, and in one it refuses (the home folder), there is no gate
    let saving = ask(&["status", "--json"], &[])
        .and_then(Result::ok)
        .filter(|o| o.code == 0)
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
        .is_some_and(|s| s["saving"] == true);
    if !saving {
        return;
    }
    let Some(ran) = ask(&["gate", "off"], &["CLAUDECODE"]) else {
        return;
    };
    match ran {
        Ok(o) if o.code == 0 => {}
        other => {
            let why = match other {
                Ok(o) => format!(
                    "exited {}: {}",
                    o.code,
                    String::from_utf8_lossy(&o.stderr).trim()
                ),
                Err(_) => "did not finish".to_owned(),
            };
            let msg = format!(
                "gate default not set, tried again at the next session start: `torpor gate off` {why}"
            );
            state::problem(dir, "SessionStart", sid, &msg);
            return;
        }
    }
    if !state::mark(dir, "gate", &key) {
        let msg = "gate default set but not remembered: the next session start runs `torpor gate off` again here, which undoes a `torpor gate on` of the user's";
        state::problem(dir, "SessionStart", sid, msg);
    }
}

//! The engine's own gate setting for this folder: what a broken check does here (tells the AI, or stops it).

use super::GATE_OFF_TELLS;
use super::ctx::Ctx;
use super::hook_command::tool_of;
use super::hooks::Hook;
use super::report::{Level, Report, clip};
use super::shell::run_shell;
use crate::util::quote;
use std::path::Path;
use std::time::Duration;

/// What a broken check does in the folder doctor is run in (F4b): the engine's own read-only `torpor gate`, run with the
/// engine and the vault the hooks use. An `info`: it is a setting of the user's, not a fault. Left out when the hooks do
/// not say which engine they run (doctor does not guess from its own PATH), when that engine is missing (`check_hooks`
/// says so) and when the vault does not exist yet (nothing is saved, and doctor creates nothing).
pub(crate) fn check_promise_gate(c: &Ctx, vault: &Path, ours: &[&Hook], r: &mut Report) {
    const TELLS: &str = GATE_OFF_TELLS;
    let Some(bin) = ours.iter().find_map(|h| tool_of(&h.command, "TORPOR_BIN")) else {
        return;
    };
    if !bin.is_file() || !vault.is_dir() {
        return;
    }
    let env: Vec<(String, String)> = vec![
        (
            "PATH".into(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
        ),
        ("HOME".into(), c.home.to_string_lossy().into_owned()),
        ("TORPOR_HOME".into(), vault.to_string_lossy().into_owned()),
    ];
    let cmd = format!("{} gate", quote(&bin.to_string_lossy()));
    let (ok, first) = match run_shell("/bin/sh", &cmd, &env, &c.cwd, b"", Duration::from_secs(5)) {
        Err(e) => (false, e),
        Ok(x) if x.timed_out => (false, "it did not finish in 5 s".to_string()),
        Ok(x) => {
            let text = [x.stdout, x.stderr]
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .join("\n");
            let line = text.lines().map(str::trim).find(|l| !l.is_empty());
            (x.code == Some(0), line.unwrap_or("").to_string())
        }
    };
    let now = match first.as_str() {
        s if ok && s.starts_with("The gate is on") => "on. A broken check can stop the AI. To let checks only tell it, run `torpor gate off` in your own terminal.".to_string(),
        s if ok && s.starts_with("The gate is off") => format!("off. {TELLS}"),
        s if ok && s.starts_with("The gate is advisory") => format!("off (advisory). {TELLS}"),
        s => format!("not read: {}", clip(s, 120)),
    };
    r.add(Level::Info, format!("torpor gate (this folder): {now}"));
}

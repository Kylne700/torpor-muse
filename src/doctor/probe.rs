//! The probe: runs the generated hook command once, as Muse would, and checks that it prints nothing but one JSON object.

use super::Ctx;
use super::hook_command::probe_command;
use super::hooks::Hook;
use super::report::{Level, Report, clip};
use super::shell::run_shell;
use crate::liveness::PROBE_SID;
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(crate) fn check_probe(c: &Ctx, ours: &[&Hook], r: &mut Report) {
    let probe_vault = c
        .temps
        .first()
        .cloned()
        .unwrap_or_default()
        .join("torpor-muse-doctor-probe");
    let shell = c.shell.clone().unwrap_or_else(|| "/bin/sh".into());
    let mut env: Vec<(String, String)> = [
        "PATH", "USER", "LOGNAME", "TMPDIR", "TEMP", "TMP", "LANG", "LC_ALL", "TERM",
    ]
    .iter()
    .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
    .collect();
    env.push(("HOME".into(), c.home.to_string_lossy().into_owned()));
    env.push(("SHELL".into(), shell.clone()));
    env.push(("MUSE_SESSION_ID".into(), PROBE_SID.into()));
    env.push(("MUSE_THREAD_ID".into(), PROBE_SID.into()));
    if !env.iter().any(|(k, _)| k == "PATH") {
        env.push(("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()));
    }
    // SubagentStart: a registered event that only records a heartbeat (in the probe vault), so the engine is not called.
    let payload = json!({
        "hook_event_name": "SubagentStart", "session_id": PROBE_SID, "cwd": c.cwd,
        "transcript_path": null, "model": "probe", "model_provider": "probe", "permission_mode": "default",
    })
    .to_string();
    let cmds: BTreeSet<String> = ours
        .iter()
        .filter_map(|h| probe_command(&h.command, &probe_vault, "SubagentStart"))
        .collect();
    for cmd in cmds {
        let ran = match run_shell(
            &shell,
            &cmd,
            &env,
            &c.cwd,
            payload.as_bytes(),
            c.probe_timeout,
        ) {
            Ok(x) => x,
            Err(e) => {
                r.problem(
                    Level::Fix,
                    format!("probe: {e}"),
                    "set SHELL to a working shell",
                );
                continue;
            }
        };
        let (out, err) = (
            String::from_utf8_lossy(&ran.stdout),
            String::from_utf8_lossy(&ran.stderr),
        );
        let object = serde_json::from_slice::<Value>(&ran.stdout).is_ok_and(|v| v.is_object());
        if ran.timed_out {
            r.problem(Level::Fix, format!("probe: `{shell} -c` did not finish in {}s (the hook command, or the shell's start-up files, hang)", c.probe_timeout.as_secs()), "run the same command by hand and see where it stops");
        } else if ran.code != Some(0) {
            r.problem(Level::Fix, format!("probe: the hook command exits with {:?}: {}", ran.code, clip(&err, 200)), "run `torpor-muse hook SubagentStart < /dev/null` by hand; if the shell cannot find torpor-muse, run torpor-muse init again");
        } else if !out.trim().is_empty() && !object {
            r.problem(
                Level::Fix,
                format!("probe: `{shell} -c` printed text that is not one JSON object: {}\n      Muse reads all of stdout as the hook's answer: on UserPromptSubmit and SessionStart it hands that text to the model as context, on Stop it ignores the answer", clip(&out, 100)),
                "make the shell's non-interactive start-up files print nothing (zsh reads ~/.zshenv for every `-c`; bash reads BASH_ENV): guard prints with [ -t 1 ] or move them to the interactive file",
            );
        } else {
            r.add(
                Level::Ok,
                format!(
                    "probe: `{} -c` runs the hook command with a cleared environment and prints {}",
                    shell,
                    if out.trim().is_empty() {
                        "nothing"
                    } else {
                        "one JSON object"
                    }
                ),
            );
        }
    }
}

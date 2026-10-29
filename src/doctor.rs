//! `torpor-muse doctor`: is the gate really live, and what broke without a sound. Owner: S3a-shim.
//!
//! It only reads, runs one harmless probe and the engine's read-only `torpor gate`: it never deletes or kills anything
//! that is not its own probe process.
//! Every line starts with `ok`, `info`, `warn` or `FIX`; a problem line is followed by a `do:` line that says how.
//! Exit 0 = nothing to fix (warnings may remain), 1 = at least one FIX.

// `#[path]`: the test crates that compile this file by path (tests/*.rs) would look for a plain `mod x;` next to
// it, in src/, and not in src/doctor/.
#[path = "doctor/ctx.rs"]
mod ctx;
#[path = "doctor/heartbeats.rs"]
mod heartbeats;
#[path = "doctor/hook_command.rs"]
pub(crate) mod hook_command;
#[path = "doctor/hook_env.rs"]
mod hook_env;
#[path = "doctor/hooks.rs"]
mod hooks;
#[path = "doctor/muse_log.rs"]
mod muse_log;
#[path = "doctor/probe.rs"]
mod probe;
#[path = "doctor/problems.rs"]
pub(crate) mod problems;
#[path = "doctor/promise_gate.rs"]
mod promise_gate;
#[path = "doctor/report.rs"]
mod report;
#[path = "doctor/shell.rs"]
mod shell;
#[path = "doctor/vault.rs"]
mod vault;

use crate::install;
use heartbeats::check_gate;
use hook_command::is_ours;
use hook_env::{check_writer, check_xdg};
use hooks::{Hook, check_hooks, read_hooks};
use muse_log::check_logs;
use probe::check_probe;
use problems::{check_errors, check_problems};
use promise_gate::check_promise_gate;
use vault::check_vault;

pub(crate) use ctx::Ctx;
pub(crate) use report::{Level, Report};

/// The only Muse Code version these hooks were tested with (research/60 AD13).
const MUSE_TESTED: &str = "1.4.13";

/// What a person must know while the engine's gate is off, as it is since F4b: `doctor` shows it, and so does the line the
/// check-writer prints after it saves a check (src/suggest/attach.rs).
pub const GATE_OFF_TELLS: &str = "A broken check is told to the AI before its turn ends; it does not stop the AI. To let checks stop it, run `torpor gate on` in your own terminal.";

pub fn run() -> u8 {
    finish(&examine(&Ctx::from_env(install::settings_path())))
}

/// Prints the report; the exit code is 1 when anything needs a fix.
pub fn finish(r: &Report) -> u8 {
    print!("{}", r.text());
    let warns = r.0.iter().filter(|(l, _)| *l == Level::Warn).count();
    match (r.fixes(), warns) {
        (0, 0) => println!("doctor: nothing to fix"),
        (0, w) => println!("doctor: nothing to fix, {w} warning(s) to read"),
        (n, _) => println!("doctor: {n} to fix"),
    }
    u8::from(r.fixes() > 0)
}

pub fn examine(c: &Ctx) -> Report {
    let mut r = Report::default();
    let (hooks, parsed) = match read_hooks(&c.settings) {
        Ok(h) => (h, true),
        Err((missing, what)) => {
            let how = if missing {
                "torpor-muse init"
            } else {
                "repair the JSON (init keeps a .torpor-muse.bak copy of the file it changed), then run torpor-muse init"
            };
            r.problem(Level::Fix, what, how);
            (Vec::new(), false)
        }
    };
    let ours: Vec<&Hook> = hooks.iter().filter(|h| is_ours(&h.command)).collect();
    if parsed {
        check_hooks(c, &ours, &mut r);
    }
    let vault = check_vault(c, &ours, &mut r);
    check_logs(c, &ours, &mut r);
    check_gate(c, &vault, &ours, &mut r);
    check_promise_gate(c, &vault, &ours, &mut r);
    check_errors(&vault, c.now_ms, &mut r);
    check_problems(&vault, c.now_ms, &mut r);
    check_probe(c, &ours, &mut r);
    if !ours.is_empty() {
        check_writer(&mut r);
    }
    check_xdg(c, &mut r);
    r
}

//! Which vault the hooks write to, and whether this shell sees the same one.

use super::ctx::Ctx;
use super::hook_command::vault_of;
use super::hooks::Hook;
use super::report::{Level, Report};
use crate::util::quote;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Prints the vault the hooks really write to; returns it.
pub(crate) fn check_vault(c: &Ctx, ours: &[&Hook], r: &mut Report) -> PathBuf {
    let default = c.home.join(".torpor");
    let baked: BTreeSet<Option<PathBuf>> = ours.iter().map(|h| vault_of(&h.command)).collect();
    let effective = match baked.iter().flatten().next() {
        Some(v) => v.clone(),
        None => default.clone(),
    };
    if baked.len() > 1 {
        let all: Vec<String> = baked
            .iter()
            .map(|v| {
                v.as_ref()
                    .map_or("(none)".into(), |p| p.display().to_string())
            })
            .collect();
        r.problem(
            Level::Fix,
            format!("the hook commands use different vaults: {}", all.join(", ")),
            "torpor-muse init (it rewrites every hook with the same vault)",
        );
    } else if !ours.is_empty() && baked.contains(&None) {
        r.problem(
            Level::Warn,
            format!(
                "the hook commands carry no TORPOR_HOME, so they use {}",
                default.display()
            ),
            "torpor-muse init",
        );
    } else if !ours.is_empty() {
        r.add(
            Level::Ok,
            format!(
                "vault: {} (TORPOR_HOME in the generated hook commands)",
                effective.display()
            ),
        );
    }
    let mine = c.env_vault.clone().unwrap_or(default);
    if !ours.is_empty() && mine != effective {
        r.problem(
            Level::Warn,
            format!("this shell uses the vault {}, Muse's hooks use {}: `torpor status` here shows another history", mine.display(), effective.display()),
            format!("export TORPOR_HOME={}", quote(&effective.to_string_lossy())),
        );
    }
    if !ours.is_empty() && !effective.is_dir() {
        r.add(
            Level::Info,
            format!(
                "the vault {} does not exist yet: no hook has run",
                effective.display()
            ),
        );
    }
    effective
}

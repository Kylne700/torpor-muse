//! What the hooks do not inherit from this shell: the check-writer's key file and XDG_DATA_HOME.

use super::Ctx;
use super::report::{Level, Report};
use crate::util::quote;
use std::path::PathBuf;

/// The check-writer that starts after a prompt reads its key from a file the config names; without one it stays off
/// and shows nothing. A warning: the gate itself works without it.
pub(crate) fn check_writer(r: &mut Report) {
    if let Some(why) = crate::suggest::writer_off(false) {
        r.problem(
            Level::Warn,
            format!("check-writer off: {why}"),
            "put your Meta API key alone on one line in a file only you can read (chmod 600), then add `key_file = ~/that/file` to ~/.config/torpor-muse/config (chmod 600); a META_API_KEY in your shell does not reach Muse Code's hooks",
        );
    }
}

pub(crate) fn check_xdg(c: &Ctx, r: &mut Report) {
    let Some(x) = &c.xdg_data else { return };
    let seen = x.join("muse/sessions");
    if seen == c.hook_logs {
        return;
    }
    let link = c
        .hook_logs
        .parent()
        .map_or_else(PathBuf::new, |p| p.to_path_buf());
    let (l, tail, how) = if c.hook_logs.is_dir() {
        (
            Level::Warn,
            "both places exist, the hooks use only the second",
            "start Muse without XDG_DATA_HOME so that its sessions go where the hooks look"
                .to_string(),
        )
    } else {
        (
            Level::Fix,
            "the hooks cannot find any session log",
            format!(
                "ln -s {} {}   (or start Muse without XDG_DATA_HOME)",
                quote(&x.join("muse").to_string_lossy()),
                quote(&link.to_string_lossy())
            ),
        )
    };
    r.problem(
        l,
        format!("XDG_DATA_HOME={} is set here: Muse keeps sessions in {} but hooks run with a cleared environment and look in {} ({tail})", x.display(), seen.display(), c.hook_logs.display()),
        how,
    );
}

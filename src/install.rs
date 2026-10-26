//! `init` / `uninstall`: Muse Code USER-scope settings hooks (research/59 D6, D7, D14). Owner: S3a-core.
#![allow(dead_code)] // skeleton: remove when implemented

/// The Muse Code settings file that `init` edits; doctor reads it. Owner may refine (research/59 section 1).
pub fn settings_path() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join(".config/muse/settings.json")
}

pub fn init() -> u8 {
    eprintln!("torpor-muse init: not implemented yet");
    1
}

pub fn uninstall() -> u8 {
    eprintln!("torpor-muse uninstall: not implemented yet");
    1
}

//! What doctor takes from the outside (settings file, home, logs, shell, clock), so a test can set it.

use crate::{liveness, transcript};
use std::path::PathBuf;
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Everything doctor takes from the outside, so a test can set it without touching the environment.
pub struct Ctx {
    /// Muse's USER-scope settings file.
    pub settings: PathBuf,
    pub home: PathBuf,
    /// Muse's session logs as this shell would find them (it honours XDG_DATA_HOME), and as the hooks find them
    /// (their environment is cleared, so always `$HOME/.local/share`).
    pub logs: PathBuf,
    pub hook_logs: PathBuf,
    pub xdg_data: Option<PathBuf>,
    /// TORPOR_HOME of this shell.
    pub env_vault: Option<PathBuf>,
    pub shell: Option<String>,
    pub temps: Vec<PathBuf>,
    pub cwd: PathBuf,
    pub now_ms: u64,
    /// How long the probe may run before doctor kills the process group it started.
    pub probe_timeout: Duration,
}

impl Ctx {
    pub fn from_env(settings: PathBuf) -> Ctx {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        let home = PathBuf::from(var("HOME").unwrap_or_default());
        let hook_logs = transcript::Paths::from_env().sessions;
        let xdg_data = var("XDG_DATA_HOME").map(PathBuf::from);
        let logs = xdg_data
            .as_ref()
            .map_or_else(|| hook_logs.clone(), |x| x.join("muse/sessions"));
        let mut temps = vec![std::env::temp_dir(), PathBuf::from("/tmp")];
        temps.dedup();
        Ctx {
            settings,
            home,
            logs,
            hook_logs,
            xdg_data,
            env_vault: var("TORPOR_HOME").map(PathBuf::from),
            shell: var("SHELL").map(|s| s.to_string_lossy().into_owned()),
            temps,
            cwd: std::env::current_dir().unwrap_or_default(),
            now_ms: liveness::now_ms(),
            probe_timeout: PROBE_TIMEOUT,
        }
    }
}

//! How the model is told to write `torpor` (the engine's own path only when that is safe) and where the project is.

use std::path::{Path, PathBuf};

/// How the model should write `torpor` in a command: the engine's own path, because its shell may not have the
/// engine's folder on its PATH. Only for a file called `torpor` (a Torpor call is known by that name); else the word.
pub(super) fn engine_cmd(bin: &Path) -> String {
    let named = bin.file_name().is_some_and(|n| n == "torpor")
        && bin.parent().is_some_and(|p| !p.as_os_str().is_empty());
    match std::path::absolute(bin) {
        Ok(p) if named => p.to_str().map_or("torpor".into(), crate::util::quote),
        _ => "torpor".into(),
    }
}

/// The engine's own project folder (store.rs `resolve_root`): CLAUDE_PROJECT_DIR, else the payload's cwd.
pub(super) fn project_root(cwd: &str) -> PathBuf {
    std::env::var_os("CLAUDE_PROJECT_DIR")
        .filter(|p| !p.is_empty())
        .map_or_else(|| PathBuf::from(cwd), PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_is_given_the_engines_own_path_only_when_that_is_safe_to_run() {
        let cmd = |p: &str| engine_cmd(Path::new(p));
        assert_eq!(cmd("/opt/engine/torpor"), "/opt/engine/torpor");
        assert_eq!(cmd("/opt/my engine/torpor"), "'/opt/my engine/torpor'");
        // a file with another name is not known as a Torpor call; a bare name has no folder to give
        assert_eq!(cmd("/opt/engine/torpor-2.3.1"), "torpor");
        assert_eq!(cmd("torpor"), "torpor");
    }
}

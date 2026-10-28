//! The settings file on disk: where it is, reading it, refusing a file we must not touch, writing it back safely.

use crate::util;
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Read};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The Muse Code settings file that `init` edits; doctor reads it: `$XDG_CONFIG_HOME/muse/settings.json`, else
/// `$HOME/.config/muse/settings.json` (the lookup Muse 1.4.13 itself does).
pub fn settings_path() -> PathBuf {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(x) => PathBuf::from(x),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config"),
    };
    base.join("muse/settings.json")
}

pub(crate) fn backup_path(settings: &Path) -> PathBuf {
    let mut s = settings.as_os_str().to_owned();
    s.push(".torpor-muse.bak");
    PathBuf::from(s)
}

/// The file as Muse needs it, or why we must not touch it. Ok(None) = no file.
pub(crate) fn read_settings(path: &Path) -> Result<Option<(Vec<u8>, Value)>, String> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(format!(
                "{} is a symbolic link; refusing to edit through it",
                path.display()
            ));
        }
        Ok(m) if !m.is_file() => return Err(format!("{} is not a regular file", path.display())),
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    let mut bytes = Vec::new();
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .and_then(|mut f| f.read_to_end(&mut bytes))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let doc: Value = serde_json::from_slice(&bytes).map_err(|e| {
        format!(
            "{} is not valid JSON ({e}); fix or remove it first",
            path.display()
        )
    })?;
    if !doc.is_object() {
        return Err(format!(
            "{}: the top level is not a JSON object",
            path.display()
        ));
    }
    Ok(Some((bytes, doc)))
}

/// Things in the file that would make Muse drop or refuse hooks, ours included. We do not repair them.
pub(crate) fn check_foreign(doc: &Value, path: &Path) -> Result<(), String> {
    let at = path.display();
    if doc["schema_version"] != json!(1) {
        return Err(format!(
            "{at}: schema_version is {}, Muse Code 1.4.13 needs 1",
            doc["schema_version"]
        ));
    }
    let Some(hooks) = doc.get("hooks") else {
        return Ok(());
    };
    let Some(hooks) = hooks.as_object() else {
        return Err(format!("{at}: `hooks` is not an object"));
    };
    for (event, groups) in hooks.iter().filter(|(k, _)| k.as_str() != "state") {
        let Some(groups) = groups.as_array() else {
            return Err(format!("{at}: hooks.{event} is not a list"));
        };
        for h in groups
            .iter()
            .filter_map(|g| g["hooks"].as_array())
            .flatten()
        {
            if h.get("timeout").is_some_and(|t| !t.is_u64()) {
                return Err(format!(
                    "{at}: a hooks.{event} handler has a timeout that is not a whole number of seconds \
                     ({}); Muse would drop every hook in the file, ours too",
                    h["timeout"]
                ));
            }
        }
    }
    Ok(())
}

/// Why `init` would refuse the settings file as it is (None = it would not). `doctor` names this instead of a
/// remedy that `init` then refuses.
pub fn refusal(path: &Path) -> Option<String> {
    match read_settings(path) {
        Err(e) => Some(e),
        Ok(Some((_, doc))) => check_foreign(&doc, path).err(),
        Ok(None) => None,
    }
}

/// `util::write_atomic` with the folder made, the mode of an existing file kept (`mode` is for a new one) and the
/// folder fsynced after the rename.
pub(crate) fn write_atomic(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    before_rename: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mode = fs::metadata(path).map_or(mode, |m| m.permissions().mode() & 0o777);
    util::write_atomic(path, bytes, mode, true, before_rename)?;
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

pub(crate) fn pretty(doc: &Value) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(doc).unwrap_or_default();
    s.push('\n');
    s.into_bytes()
}

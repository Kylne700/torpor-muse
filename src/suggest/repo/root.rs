//! Which folder a run is about: the project root, whether it is a synthetic app and its start choices.

use crate::suggest::schema::Start;
use crate::suggest::state::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Same rule as the engine: `CLAUDE_PROJECT_DIR`, else the given folder, else the current one; canonical.
pub fn resolve_root(cwd: Option<&str>) -> Result<PathBuf, String> {
    let raw = env("CLAUDE_PROJECT_DIR")
        .or_else(|| cwd.filter(|c| !c.is_empty()).map(String::from))
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .ok_or("no project folder")?;
    let root =
        fs::canonicalize(&raw).map_err(|e| format!("cannot open the project folder: {e}"))?;
    let home = env("HOME").and_then(|h| fs::canonicalize(h).ok());
    if root == Path::new("/") || home.as_deref() == Some(root.as_path()) {
        return Err("refusing to work in `/` or in the home folder".into());
    }
    Ok(root)
}

/// A `.synthetic` file in the project root says its content is made up (tests, demos).
pub fn is_synthetic(root: &Path) -> bool {
    root.join(".synthetic").is_file()
}

/// The start commands the model may pick, built from `package.json` scripts with fixed names. No free text.
/// shortcut: one fixed fallback (`node server.js`) when there is no usable script; no other guesses. Add more when real projects need them.
pub fn start_choices(root: &Path) -> Vec<Start> {
    let mut out = Vec::new();
    let scripts = fs::read_to_string(root.join("package.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .map(|v| v["scripts"].clone());
    for name in ["start", "dev", "serve", "server"] {
        if scripts.as_ref().is_some_and(|s| s[name].is_string()) {
            let cmd = if name == "start" {
                "npm start --silent".to_string()
            } else {
                format!("npm run {name} --silent")
            };
            out.push(Start {
                key: name.into(),
                cmd,
            });
        }
    }
    if out.is_empty() && root.join("server.js").is_file() {
        out.push(Start {
            key: "node server.js".into(),
            cmd: "node server.js".into(),
        });
    }
    out
}

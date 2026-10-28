//! Rank 1 of the pains Torpor does not solve (research/71c P1a): the AI deletes or moves files outside the project
//! folder. Torpor saves only the project folder and cannot bring those back, so such a Bash command is denied after the
//! engine's own answer (its save point is made), with a reason that has the AI ask the person first. Muse hooks cannot
//! ask (`schema::sanitise`), so it is a deny. A Write or Edit whose new text holds `rm -rf ~` gets a one-line note.
//! shortcut: this reads commands beside the engine; it belongs in the engine's reader (71c fix M4) after Nov 4.

mod place;
mod reader;
mod text;

use super::paths::project_root;
use super::state;
use crate::{engine, schema};
use place::{Env, Target, norm};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The engine's answer `body` to a Bash call, as a deny when the command deletes or moves something outside the
/// project folder that is not a temp folder. An engine deny stays as it is.
pub(super) fn guard(payload: &Value, cwd: &str, body: Option<String>) -> Option<String> {
    let denied = body
        .as_deref()
        .and_then(|b| serde_json::from_str::<Value>(b).ok())
        .is_some_and(|v| v["hookSpecificOutput"]["permissionDecision"].is_string());
    let Some(cmd) = payload["tool_input"]["command"].as_str() else {
        return body;
    };
    if payload["tool_name"] != "Bash" || cwd.is_empty() || denied {
        return body;
    }
    let var = |k: &str| {
        std::env::var_os(k)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .map(|p| norm(&p))
    };
    let home = var("HOME");
    let root = norm(&project_root(cwd));
    let env = Env {
        tmp: var("TMPDIR").and_then(|t| place::usable_tmp(t, home.as_deref())),
        store: Some(engine::home_dir())
            .filter(|s| s.is_absolute())
            .map(|s| norm(&s)),
        root: [root.canonicalize().ok(), Some(root)]
            .into_iter()
            .flatten()
            .collect(),
        home,
    };
    let workdir = payload["tool_input"]["workdir"].as_str().unwrap_or("");
    let hits = stopped(cmd, norm(&Path::new(cwd).join(workdir)), &env);
    let Some(first) = hits.first() else {
        return body;
    };
    let more = match hits.len() {
        1 => String::new(),
        n => format!(" and {} more", n - 1),
    };
    let why = format!(
        "Torpor: this command was not run. It deletes or moves {first}{more}, outside the project folder. Torpor saves \
         only the project folder, so it could not bring that back. Ask the user first; if they want it, they can run it \
         themselves in their own terminal."
    );
    schema::deny("PreToolUse", body.as_deref(), &why).or(body)
}

/// The targets of `cmd`, run in `dir`, that are stopped, as the reason names them.
fn stopped(cmd: &str, dir: PathBuf, env: &Env) -> Vec<String> {
    let mut hits: Vec<String> = reader::targets(cmd, dir, env)
        .iter()
        .filter(|t| place::stopped(t, env))
        .map(|t| match t {
            Target::Path(p) => shown(&p.to_string_lossy(), 200),
            Target::Prefix(s, pat) => shown(pat.as_deref().unwrap_or(&format!("{s}*")), 200),
        })
        .collect();
    hits.dedup();
    hits
}

/// `s` on one line, at most `max` characters: a path or command the AI wrote, shown back to it.
fn shown(s: &str, max: usize) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max)
        .collect()
}

/// For a Write or Edit call (read before the payload is normalised, which drops file bodies) whose new text holds `rm`
/// aimed at the home folder or `/`: the file and the line for the AI.
pub(super) fn text_line(event: &str, payload: &Value) -> Option<(String, String)> {
    let tool = payload["tool_name"].as_str();
    if event != "PreToolUse" || !matches!(tool, Some("write_file" | "edit_file" | "Write" | "Edit"))
    {
        return None;
    }
    let input = &payload["tool_input"];
    let (cmd, home) = text::find(input["content"].as_str().or(input["replace"].as_str())?)?;
    let file = input["path"]
        .as_str()
        .or(input["file_path"].as_str())
        .unwrap_or("");
    let what = if home {
        "the user's home folder"
    } else {
        "every file the user can delete on this computer"
    };
    let line = format!(
        "Torpor: the text written to {} holds `{}`; if that ever runs, it deletes {what}, and Torpor cannot bring \
         that back.",
        shown(file, 200),
        shown(&cmd, 60)
    );
    Some((file.to_owned(), line))
}

/// `body` with `line` added for the AI, once per file per turn.
pub(super) fn text_note(
    dir: &Path,
    sid: &str,
    turn: &str,
    (file, line): (String, String),
    body: Option<String>,
) -> Option<String> {
    let first = state::with_session(dir, sid, |s| {
        if s.text_told.0 != turn {
            s.text_told = (turn.to_owned(), Vec::new());
        }
        let new = !s.text_told.1.contains(&file);
        if new {
            s.text_told.1.push(file);
        }
        new
    })
    .unwrap_or(true);
    if !first {
        return body;
    }
    schema::with_note("PreToolUse", body.as_deref(), "", &line, false).or(body)
}

#[cfg(test)]
mod tests;

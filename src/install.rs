//! `init` / `uninstall`: Muse Code USER-scope settings hooks (research/59 D6, D7, D14). Owner: S3a-core.
//!
//! Muse reads hooks from `settings.json` in its user config directory. A settings hook needs no approval (D14), a
//! plugin hook does, so we only ever edit this one file. The file is fragile (D6): a bad `schema_version`, or one
//! `timeout` that is not a whole number, makes Muse refuse to start or drop every hook in the file. So `init` checks
//! the file before and the result after, writes through a temp file and a rename, and refuses rather than guesses.

mod entries;
mod settings;
#[cfg(test)]
mod tests;

use crate::engine;
use crate::hook::EVENTS;
use entries::{check_result, command, group, prune, strip_empty};
use serde_json::{Value, json};
use settings::{backup_path, check_foreign, pretty, read_settings, write_atomic};
use std::fs;
use std::path::Path;

pub(crate) use settings::{refusal, settings_path};

/// An absolute path as text, or why not.
fn abs_text(p: &Path) -> Result<String, String> {
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().map_err(|e| e.to_string())?.join(p)
    };
    p.to_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{} is not valid UTF-8", p.display()))
}

fn try_init() -> Result<(), String> {
    let path = settings_path();
    if !path.is_absolute() {
        return Err("HOME is not set".into());
    }
    let exe = std::env::current_exe().map_err(|e| format!("cannot find myself: {e}"))?;
    let exe = abs_text(&fs::canonicalize(&exe).unwrap_or(exe))?;
    let bin = engine::locate()?;
    engine::verify(&bin)?;
    let bin = abs_text(&fs::canonicalize(&bin).unwrap_or(bin))?;
    let home = abs_text(&engine::home_dir())?;

    let (old_bytes, old_doc) = match read_settings(&path)? {
        Some((b, d)) => (Some(b), d),
        None => (None, json!({"schema_version": 1})),
    };
    check_foreign(&old_doc, &path)?;
    let mut doc = old_doc;
    prune(&mut doc); // an earlier install of ours is replaced, never doubled
    let before_ours = doc.clone();
    let root = doc.as_object_mut().ok_or("settings is not an object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("hooks is not an object")?;
    for ev in EVENTS {
        let list = hooks.entry(ev.name).or_insert_with(|| json!([]));
        list.as_array_mut()
            .ok_or_else(|| format!("hooks.{} is not a list", ev.name))?
            .push(group(ev, command(&home, &bin, &exe, ev.name)));
    }
    check_result(&doc, &before_ours)?;
    let new_bytes = pretty(&doc);
    if serde_json::from_slice::<Value>(&new_bytes).ok().as_ref() != Some(&doc) {
        return Err("internal check failed: the new file does not read back the same".into());
    }
    if old_bytes.as_deref() == Some(&new_bytes[..]) {
        println!(
            "torpor-muse hooks are already installed in {}",
            path.display()
        );
        return Ok(());
    }
    let mut note = String::new();
    if let Some(orig) = &old_bytes {
        let bak = backup_path(&path);
        if !bak.exists() {
            write_atomic(&bak, orig, 0o600, || Ok(()))
                .map_err(|e| format!("cannot write the backup {}: {e}", bak.display()))?;
            note = format!(" (original saved as {})", bak.display());
        }
    }
    write_atomic(&path, &new_bytes, 0o600, || Ok(()))
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    println!("torpor-muse hooks installed in {}{note}", path.display());
    println!("store {home}, engine {bin}; start a new Muse session to use them");
    Ok(())
}

pub fn init() -> u8 {
    match try_init() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("torpor-muse init: {e}");
            1
        }
    }
}

fn remove_hooks() -> Result<(), String> {
    let path = settings_path();
    let Some((_, mut doc)) = read_settings(&path)? else {
        println!("nothing to do: {} does not exist", path.display());
        return Ok(());
    };
    if prune(&mut doc) == 0 {
        println!("no torpor-muse hooks in {}", path.display());
        return Ok(());
    }
    let bak = backup_path(&path);
    let saved = fs::read(&bak).ok();
    let mut now = doc.clone();
    strip_empty(&mut now);
    // The file is what it was before `init`, apart from our entries: put the original bytes back.
    if let Some(orig) = saved.filter(|o| {
        serde_json::from_slice::<Value>(o).is_ok_and(|mut v| {
            strip_empty(&mut v);
            v == now
        })
    }) {
        write_atomic(&path, &orig, 0o600, || Ok(())).map_err(|e| e.to_string())?;
        let _ = fs::remove_file(&bak);
        println!(
            "torpor-muse hooks removed; {} is back to its original bytes",
            path.display()
        );
    } else if bak.exists() || now != json!({"schema_version": 1}) {
        write_atomic(&path, &pretty(&doc), 0o600, || Ok(())).map_err(|e| e.to_string())?;
        // the copy init saved no longer matches the file (it was edited since), so it stays: say where
        let left = if bak.exists() {
            format!("; the copy init saved stays at {}", bak.display())
        } else {
            String::new()
        };
        println!(
            "torpor-muse hooks removed from {}; your other settings are kept{left}",
            path.display()
        );
    } else {
        fs::remove_file(&path).map_err(|e| e.to_string())?; // we created it and nothing else is in it
        println!(
            "torpor-muse hooks removed; {} was created by init and is deleted",
            path.display()
        );
    }
    Ok(())
}

/// What `init` leaves besides the hooks: the engine-ok cache, and the folders made for it and for the settings file
/// (`<config>/muse`, `<config>`, `<store>/muse`, `<store>`). Rule: a directory goes only if it is empty (`remove_dir`
/// never takes content); removing an empty one the user already had is harmless. Files in the state folder (spend
/// ledger, results, logs) are never deleted: while any are left this returns the line that says so.
fn tidy(settings: &Path, state: &Path) -> Option<String> {
    if !settings.is_absolute() || !state.is_absolute() {
        return None; // HOME is not set: these would be paths under the current directory
    }
    let _ = fs::remove_file(state.join(engine::ENGINE_OK));
    for dir in settings
        .ancestors()
        .skip(1)
        .take(2)
        .chain(state.ancestors().take(2))
    {
        let _ = fs::remove_dir(dir);
    }
    fs::read_dir(state).ok()?.next().map(|_| {
        format!(
            "kept: {} still holds torpor-muse's files (spend ledger, results, logs); deleting that folder is your choice. \
             Also kept: ~/.config/torpor-muse/config (if you made one) and the engine's {} and ~/.config/torpor; \
             to remove Torpor completely, delete those too",
            state.display(),
            state.parent().unwrap_or(state).display()
        )
    })
}

fn try_uninstall() -> Result<(), String> {
    remove_hooks()?;
    if let Some(line) = tidy(&settings_path(), &engine::state_dir()) {
        println!("{line}");
    }
    Ok(())
}

pub fn uninstall() -> u8 {
    match try_uninstall() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("torpor-muse uninstall: {e}");
            1
        }
    }
}

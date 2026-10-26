//! `init` / `uninstall`: Muse Code USER-scope settings hooks (research/59 D6, D7, D14). Owner: S3a-core.
//!
//! Muse reads hooks from `settings.json` in its user config directory. A settings hook needs no approval (D14), a
//! plugin hook does, so we only ever edit this one file. The file is fragile (D6): a bad `schema_version`, or one
//! `timeout` that is not a whole number, makes Muse refuse to start or drop every hook in the file. So `init` checks
//! the file before and the result after, writes through a temp file and a rename, and refuses rather than guesses.

use crate::engine;
use crate::hook::{Act, EVENTS, Ev};
use serde_json::{Map, Value, json};
use std::fs;
use std::io::{self, Read, Write};
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

fn backup_path(settings: &Path) -> PathBuf {
    let mut s = settings.as_os_str().to_owned();
    s.push(".torpor-muse.bak");
    PathBuf::from(s)
}

/// Single quotes only when the shell needs them. Works in sh, bash, zsh and fish.
fn quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-./:=+,@%".contains(&b));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Muse runs hooks with a cleared environment (D5/AD8), so the store and the engine are baked into the command.
fn command(home: &str, bin: &str, exe: &str, event: &str) -> String {
    format!(
        "env TORPOR_HOME={} TORPOR_BIN={} {} hook {event}",
        quote(home),
        quote(bin),
        quote(exe)
    )
}

/// Is this handler one of ours (written by any earlier `init`, from any install path)?
fn is_ours(handler: &Value, event: &str) -> bool {
    handler["command"].as_str().is_some_and(|c| {
        c.starts_with("env TORPOR_HOME=")
            && c.contains("torpor-muse")
            && c.ends_with(&format!(" hook {event}"))
    })
}

fn group(ev: &Ev, cmd: String) -> Value {
    let mut g = Map::new();
    if let Some(m) = ev.matcher {
        g.insert("matcher".into(), m.into());
    }
    g.insert(
        "hooks".into(),
        json!([{"type": "command", "command": cmd, "timeout": ev.timeout_s}]),
    );
    Value::Object(g)
}

/// Removes our handlers; a group or event that only we filled goes with them. Returns how many handlers went.
fn prune(doc: &mut Value) -> usize {
    let mut removed = 0;
    let Some(hooks) = doc.get_mut("hooks").and_then(Value::as_object_mut) else {
        return 0;
    };
    for ev in EVENTS {
        let Some(groups) = hooks.get_mut(ev.name).and_then(Value::as_array_mut) else {
            continue;
        };
        let before = removed;
        let mut kept = Vec::new();
        for mut g in std::mem::take(groups) {
            let mut here = 0;
            if let Some(hs) = g.get_mut("hooks").and_then(Value::as_array_mut) {
                let n = hs.len();
                hs.retain(|h| !is_ours(h, ev.name));
                here = n - hs.len();
                if here > 0 && hs.is_empty() {
                    removed += here;
                    continue;
                }
            }
            removed += here;
            kept.push(g);
        }
        *groups = kept;
        if removed > before && groups.is_empty() {
            hooks.remove(ev.name);
        }
    }
    if removed > 0 && hooks.is_empty() {
        drop_hooks(doc);
    }
    removed
}

/// For comparisons: empty event lists and an empty `hooks` object do not count.
fn strip_empty(doc: &mut Value) {
    if let Some(hooks) = doc.get_mut("hooks").and_then(Value::as_object_mut) {
        hooks.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
        if hooks.is_empty() {
            drop_hooks(doc);
        }
    }
}

fn drop_hooks(doc: &mut Value) {
    if let Some(o) = doc.as_object_mut() {
        o.remove("hooks");
    }
}

/// The file as Muse needs it, or why we must not touch it. Ok(None) = no file.
fn read_settings(path: &Path) -> Result<Option<(Vec<u8>, Value)>, String> {
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
fn check_foreign(doc: &Value, path: &Path) -> Result<(), String> {
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

/// The finished document must hold exactly our entries, as Muse reads them, and leave everything else as it was.
fn check_result(new: &Value, before_ours: &Value) -> Result<(), String> {
    let mut rest = new.clone();
    prune(&mut rest);
    strip_empty(&mut rest);
    let mut old = before_ours.clone();
    strip_empty(&mut old);
    if rest != old {
        return Err("internal check failed: the edit would change more than our entries".into());
    }
    for ev in EVENTS {
        let ours: Vec<(&Value, &Value)> = new["hooks"][ev.name]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|g| {
                g["hooks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(move |h| (g, h))
            })
            .filter(|(_, h)| is_ours(h, ev.name))
            .collect();
        if ev.act == Act::Off {
            if !ours.is_empty() {
                return Err(format!(
                    "internal check failed: {} must not be registered",
                    ev.name
                ));
            }
            continue;
        }
        let [(g, h)] = ours[..] else {
            return Err(format!(
                "internal check failed: {} entries for {}",
                ours.len(),
                ev.name
            ));
        };
        let keys_ok = h.as_object().is_some_and(|o| {
            o.keys()
                .all(|k| ["type", "command", "timeout"].contains(&k.as_str()))
        });
        let matcher_ok = g.get("matcher").and_then(Value::as_str) == ev.matcher;
        let fork_ok = ev.name != "SessionStart"
            || g["matcher"]
                .as_str()
                .is_some_and(|m| m.split('|').any(|x| x == "fork"));
        if h["type"] != "command"
            || h["timeout"] != json!(ev.timeout_s)
            || !keys_ok
            || !matcher_ok
            || !fork_ok
        {
            return Err(format!(
                "internal check failed: the {} entry is wrong: {h}",
                ev.name
            ));
        }
    }
    if new["hooks"].get("state") != before_ours["hooks"].get("state") {
        return Err("internal check failed: hooks.state changed".into());
    }
    Ok(())
}

/// Writes `bytes` to `path` through a temp file in the same directory: write, fsync, `before_rename`, rename.
/// A failure (or a crash) before the rename leaves `path` as it was. `mode` is for a new file.
fn write_atomic(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    before_rename: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp{}", std::process::id()));
    let _ = fs::remove_file(&tmp); // left over from a crash of an earlier run with the same process id
    let put = || -> io::Result<()> {
        let mode = fs::metadata(path).map_or(mode, |m| m.permissions().mode() & 0o777);
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        before_rename()?;
        fs::rename(&tmp, path)
    };
    let r = put();
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
    } else if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    r
}

fn pretty(doc: &Value) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(doc).unwrap_or_default();
    s.push('\n');
    s.into_bytes()
}

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
    for ev in EVENTS.iter().filter(|e| e.act != Act::Off) {
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

fn try_uninstall() -> Result<(), String> {
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
        println!(
            "torpor-muse hooks removed from {}; your other settings are kept",
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

pub fn uninstall() -> u8 {
    match try_uninstall() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("torpor-muse uninstall: {e}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::testing::scratch;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn foreign() -> Value {
        json!({
            "schema_version": 1,
            "runtime_capabilities": {"plugin:x": {"enabled": false}},
            "unknown_future_key": {"a": [1, 2, {"b": null}]},
            "hooks": {
                "state": {"approved": ["x"]},
                "Stop": [{"hooks": [{"type": "command", "command": "echo foreign-stop", "timeout": 7}]}],
                "PreToolUse": [{"matcher": "bash", "hooks": [{"type": "command", "command": "echo foreign-pre", "timeout": 3}]}]
            }
        })
    }

    fn ours_doc(extra: Value) -> Value {
        let mut doc = extra;
        let root = doc.as_object_mut().unwrap();
        let hooks = root
            .entry("hooks")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap();
        for ev in EVENTS.iter().filter(|e| e.act != Act::Off) {
            let cmd = command("/s/home", "/s/bin/torpor", "/s/bin/torpor-muse", ev.name);
            hooks
                .entry(ev.name)
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap()
                .push(group(ev, cmd));
        }
        doc
    }

    #[test]
    fn prune_removes_exactly_our_entries() {
        let mut doc = ours_doc(foreign());
        assert_eq!(
            prune(&mut doc),
            EVENTS.iter().filter(|e| e.act != Act::Off).count()
        );
        assert_eq!(doc, foreign());
        let mut fresh = ours_doc(json!({"schema_version": 1}));
        prune(&mut fresh);
        assert_eq!(
            fresh,
            json!({"schema_version": 1}),
            "an empty hooks object goes too"
        );
    }

    #[test]
    fn what_we_write_passes_our_own_check_and_a_broken_edit_does_not() {
        let before = foreign();
        let mut doc = ours_doc(before.clone());
        check_result(&doc, &before).unwrap();
        // a string timeout (Muse drops the whole file), a missing fork, an extra key, a changed foreign hook
        for (path, bad) in [
            ("/hooks/Stop/1/hooks/0/timeout", json!("35")),
            ("/hooks/Stop/1/hooks/0/timeout", json!(35.5)),
            (
                "/hooks/SessionStart/0/matcher",
                json!("startup|resume|clear|compact"),
            ),
            ("/hooks/Stop/1/hooks/0/async", json!(true)),
            ("/hooks/Stop/0/hooks/0/command", json!("echo changed")),
            ("/hooks/state", json!({})),
            ("/unknown_future_key", json!(1)),
        ] {
            let mut broken = doc.clone();
            let (parent, key) = path.rsplit_once('/').unwrap();
            let slot = broken.pointer_mut(parent).unwrap().as_object_mut().unwrap();
            slot.insert(key.into(), bad);
            assert!(
                check_result(&broken, &before).is_err(),
                "{path} should be refused"
            );
        }
        doc["hooks"]["Stop"][1]["hooks"][0]["timeout"] = json!(35);
        check_result(&doc, &before).unwrap();
    }

    #[test]
    fn foreign_problems_are_refused_not_repaired() {
        let p = Path::new("settings.json");
        let mut d = foreign();
        d["hooks"]["Stop"][0]["hooks"][0]["timeout"] = json!("7");
        assert!(check_foreign(&d, p).unwrap_err().contains("whole number"));
        let mut d = foreign();
        d["hooks"]["Stop"] = json!({"not": "a list"});
        assert!(check_foreign(&d, p).is_err());
        let mut d = foreign();
        d["schema_version"] = json!(2);
        assert!(check_foreign(&d, p).is_err());
        d.as_object_mut().unwrap().remove("schema_version");
        assert!(check_foreign(&d, p).is_err());
        check_foreign(&foreign(), p).unwrap();
    }

    #[test]
    fn commands_survive_odd_paths_and_are_recognised_from_any_install() {
        let c = command("/a b/h'ome", "/x/torpor", "/x/torpor-muse", "Stop");
        assert_eq!(
            c,
            r"env TORPOR_HOME='/a b/h'\''ome' TORPOR_BIN=/x/torpor /x/torpor-muse hook Stop"
        );
        assert!(is_ours(&json!({"command": c}), "Stop"));
        assert!(!is_ours(&json!({"command": c}), "SessionStart"));
        assert!(!is_ours(
            &json!({"command": "env TORPOR_HOME=/h torpor hook Stop"}),
            "Stop"
        ));
        assert!(!is_ours(
            &json!({"command": "echo torpor-muse hook Stop"}),
            "Stop"
        ));
        assert!(!is_ours(&json!({"type": "http"}), "Stop"));
    }

    #[test]
    fn a_crash_between_the_temp_write_and_the_rename_leaves_the_old_file() {
        let dir = scratch("install-crash");
        let target = dir.join("settings.json");
        fs::write(&target, b"old contents").unwrap();
        let leftovers = || {
            fs::read_dir(&dir)
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name() != "settings.json")
                .count()
        };
        // an error at that moment: old file intact, temp file cleaned up
        let r = write_atomic(&target, b"new contents", 0o600, || {
            Err(io::Error::other("disk gone"))
        });
        assert!(r.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old contents");
        assert_eq!(leftovers(), 0);
        // a hard stop at that moment (the panic skips the cleanup): old file intact, temp file stays
        let r = catch_unwind(AssertUnwindSafe(|| {
            let _ = write_atomic(&target, b"new contents", 0o600, || panic!("power cut"));
        }));
        assert!(r.is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old contents");
        assert_eq!(leftovers(), 1, "the stale temp file is what a crash leaves");
        // the next write succeeds even with that stale file around
        write_atomic(&target, b"new contents", 0o600, || Ok(())).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new contents");
        assert_eq!(
            leftovers(),
            0,
            "a stale temp file with our process id is replaced"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_mode_is_kept() {
        let dir = scratch("install-mode");
        let target = dir.join("settings.json");
        fs::write(&target, b"{}").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&target, b"{\"a\":1}", 0o600, || Ok(())).unwrap();
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

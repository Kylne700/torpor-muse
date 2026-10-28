//! Our hook entries inside a settings document: what we write, how we recognise them, removing them, checking the result.

use crate::hook::{EVENTS, Ev, RETIRED};
use crate::util::quote;
use serde_json::{Map, Value, json};

/// Muse runs hooks with a cleared environment (D5/AD8), so the store and the engine are baked into the command.
pub(crate) fn command(home: &str, bin: &str, exe: &str, event: &str) -> String {
    format!(
        "env TORPOR_HOME={} TORPOR_BIN={} {} hook {event}",
        quote(home),
        quote(bin),
        quote(exe)
    )
}

/// Is this handler one of ours (written by any earlier `init`, from any install path)?
pub(crate) fn is_ours(handler: &Value, event: &str) -> bool {
    handler["command"].as_str().is_some_and(|c| {
        c.starts_with("env TORPOR_HOME=")
            && c.contains("torpor-muse")
            && c.ends_with(&format!(" hook {event}"))
    })
}

pub(crate) fn group(ev: &Ev, cmd: String) -> Value {
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

/// Removes our handlers, also those an earlier version registered for an event we no longer use (RETIRED); a group or
/// event that only we filled goes with them. Returns how many handlers went.
pub(crate) fn prune(doc: &mut Value) -> usize {
    let mut removed = 0;
    let Some(hooks) = doc.get_mut("hooks").and_then(Value::as_object_mut) else {
        return 0;
    };
    for name in EVENTS.iter().map(|e| e.name).chain(RETIRED.iter().copied()) {
        let Some(groups) = hooks.get_mut(name).and_then(Value::as_array_mut) else {
            continue;
        };
        let before = removed;
        let mut kept = Vec::new();
        for mut g in std::mem::take(groups) {
            let mut here = 0;
            if let Some(hs) = g.get_mut("hooks").and_then(Value::as_array_mut) {
                let n = hs.len();
                hs.retain(|h| !is_ours(h, name));
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
            hooks.remove(name);
        }
    }
    if removed > 0 && hooks.is_empty() {
        drop_hooks(doc);
    }
    removed
}

/// For comparisons: empty event lists and an empty `hooks` object do not count.
pub(crate) fn strip_empty(doc: &mut Value) {
    if let Some(hooks) = doc.get_mut("hooks").and_then(Value::as_object_mut) {
        hooks.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
        if hooks.is_empty() {
            drop_hooks(doc);
        }
    }
}

pub(crate) fn drop_hooks(doc: &mut Value) {
    if let Some(o) = doc.as_object_mut() {
        o.remove("hooks");
    }
}

/// Our (group, handler) pairs under one event name.
pub(crate) fn ours_in<'a>(doc: &'a Value, name: &str) -> Vec<(&'a Value, &'a Value)> {
    doc["hooks"][name]
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
        .filter(|(_, h)| is_ours(h, name))
        .collect()
}

/// The finished document must hold exactly our entries, as Muse reads them, and leave everything else as it was.
pub(crate) fn check_result(new: &Value, before_ours: &Value) -> Result<(), String> {
    let mut rest = new.clone();
    prune(&mut rest);
    strip_empty(&mut rest);
    let mut old = before_ours.clone();
    strip_empty(&mut old);
    if rest != old {
        return Err("internal check failed: the edit would change more than our entries".into());
    }
    for name in RETIRED {
        if !ours_in(new, name).is_empty() {
            return Err(format!(
                "internal check failed: {name} must not be registered"
            ));
        }
    }
    for ev in EVENTS {
        let ours = ours_in(new, ev.name);
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

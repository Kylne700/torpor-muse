//! The typed payload of each Muse hook event (unknown keys kept in `extra`) and the test for an observer child.

use serde::Deserialize;
use serde_json::{Map, Value};

/// Keys every Muse event carries (`turn_id` is absent on SessionStart and SessionEnd; `transcript_path` is always null).
#[allow(dead_code)]
// the fields document the payload; the hook reads the raw JSON and uses the `extra` keys of the parse
#[derive(Deserialize, Debug, Clone)]
pub struct Common {
    pub session_id: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub hook_event_name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub model_provider: Option<String>,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
}

/// One struct per event (common keys + the event's own keys + every key we do not know yet in `extra`), an enum over
/// them, and the list of event names.
macro_rules! payloads {
    ($($name:ident: $event:literal { $($field:ident: $ty:ty),* $(,)? })*) => {
        $(
            #[allow(dead_code)] // the fields document the payload
            #[derive(Deserialize, Debug, Clone)]
            pub struct $name {
                #[serde(flatten)]
                pub common: Common,
                $( #[serde(default)] pub $field: $ty, )*
                #[serde(flatten)]
                pub extra: Map<String, Value>,
            }
        )*

        #[derive(Debug, Clone)]
        pub enum Payload { $($name($name)),* }

        impl Payload {
            pub fn parse(event: &str, v: &Value) -> Result<Payload, String> {
                match event {
                    $($event => $name::deserialize(v).map(Payload::$name).map_err(|e| e.to_string()),)*
                    _ => Err(format!("unknown event {event}")),
                }
            }
            #[cfg(test)]
            pub fn common(&self) -> &Common {
                match self { $(Payload::$name(p) => &p.common),* }
            }
            /// Keys the typed struct does not know.
            pub fn extra(&self) -> &Map<String, Value> {
                match self { $(Payload::$name(p) => &p.extra),* }
            }
        }

        /// The events we register (`hook::EVENTS`).
        #[cfg(test)]
        pub const EVENT_NAMES: &[&str] = &[$($event),*];
    };
}

payloads! {
    SessionStart: "SessionStart" { source: Option<String> }
    SessionEnd: "SessionEnd" { reason: Option<String> }
    UserPromptSubmit: "UserPromptSubmit" { prompt: Option<String> }
    PreToolUse: "PreToolUse" { tool_name: Option<String>, tool_input: Value, tool_use_id: Option<String> }
    PostToolUse: "PostToolUse" {
        tool_name: Option<String>, tool_input: Value, tool_response: Value, tool_use_id: Option<String>,
    }
    PostToolUseFailure: "PostToolUseFailure" {
        tool_name: Option<String>, tool_input: Value, tool_use_id: Option<String>,
        error: Option<String>, duration_ms: Value, is_interrupt: Option<bool>,
    }
    Stop: "Stop" { last_assistant_message: Option<String>, stop_hook_active: Option<bool> }
    StopFailure: "StopFailure" { error: Option<String>, last_assistant_message: Option<String> }
    SubagentStart: "SubagentStart" { subagent_id: Option<String>, child_session_id: Option<String> }
    SubagentStop: "SubagentStop" {
        subagent_id: Option<String>, child_session_id: Option<String>,
        last_assistant_message: Option<String>, stop_hook_active: Option<bool>,
    }
    PreCompact: "PreCompact" { trigger: Option<String> }
}

/// True when the event comes from an observer child session (research/59 D10): in every one of the 1,329 hook payloads
/// G0 recorded, a child event has `turn_id == session_id`, and no main-session event has.
pub fn is_child(v: &Value) -> bool {
    let id = |k: &str| v.get(k).and_then(Value::as_str);
    let sid = id("session_id");
    sid.is_some() && (id("turn_id") == sid || id("child_session_id") == sid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn fixtures() -> Vec<(String, String, Value)> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/core");
        let mut v = Vec::new();
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "json") {
                let val: Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
                let name = p.file_stem().unwrap().to_string_lossy().to_string();
                let event = val["hook_event_name"].as_str().unwrap().to_string();
                v.push((name, event, val));
            }
        }
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    #[test]
    fn golden_every_fixture_parses_into_its_event_struct_without_unknown_keys() {
        let all = fixtures();
        assert!(all.len() >= 25, "{} fixtures", all.len());
        for (name, event, v) in &all {
            let p = Payload::parse(event, v).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(p.extra().is_empty(), "{name}: unknown keys {:?}", p.extra());
            assert_eq!(p.common().hook_event_name.as_deref(), Some(event.as_str()));
        }
        // every event that has a fixture is a known event; the four that never fired have none (research/59)
        for (name, event, _) in &all {
            assert!(EVENT_NAMES.contains(&event.as_str()), "{name}");
        }
    }

    #[test]
    fn a_new_key_lands_in_extra() {
        let v = json!({"session_id": "s", "hook_event_name": "Stop", "brand_new": 1});
        let p = Payload::parse("Stop", &v).unwrap();
        assert_eq!(p.extra().keys().collect::<Vec<_>>(), ["brand_new"]);
    }

    #[test]
    fn children_are_found_by_turn_id_or_child_session_id() {
        assert!(is_child(&json!({"session_id": "c", "turn_id": "c"})));
        assert!(is_child(
            &json!({"session_id": "c", "child_session_id": "c"})
        ));
        assert!(!is_child(&json!({"session_id": "m", "turn_id": "t"})));
        assert!(!is_child(&json!({"session_id": "m"})));
        assert!(!is_child(
            &json!({"session_id": "m", "child_session_id": "c"})
        ));
    }
}

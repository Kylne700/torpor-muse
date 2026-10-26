//! Typed Muse Code hook payloads (input, unknown keys kept) and outputs (exact, camelCase). Owner: S3a-core.
//!
//! Output rules come from research/59 D5 and from the strings of Muse Code 1.4.13: stdout is parsed only when it starts
//! with `{`, the key set is closed and camelCase, and ONE unknown key rejects the whole output (the hook then counts as
//! failed and the turn goes on, so a Torpor block would silently vanish). `validate` is that rule set; `sanitise` turns
//! what the engine printed (Claude-style JSON) into something `validate` accepts.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

// ---------------------------------------------------------------- input

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

/// One entry of PostToolBatch `tool_calls[]`.
#[allow(dead_code)] // the fields document the payload
#[derive(Deserialize, Debug, Clone, Default)]
pub struct ToolCall {
    pub tool_name: Option<String>,
    pub tool_input: Value,
    pub tool_response: Value,
    pub tool_use_id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
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

        /// Every Muse hook event kind (research/59 section 4, binary strings of 1.4.13).
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
    PostToolBatch: "PostToolBatch" { tool_calls: Vec<ToolCall> }
    Stop: "Stop" { last_assistant_message: Option<String>, stop_hook_active: Option<bool> }
    StopFailure: "StopFailure" { error: Option<String>, last_assistant_message: Option<String> }
    SubagentStart: "SubagentStart" { subagent_id: Option<String>, child_session_id: Option<String> }
    SubagentStop: "SubagentStop" {
        subagent_id: Option<String>, child_session_id: Option<String>,
        last_assistant_message: Option<String>, stop_hook_active: Option<bool>,
    }
    PreCompact: "PreCompact" { trigger: Option<String> }
    PostCompact: "PostCompact" { trigger: Option<String> }
    PreLLMCall: "PreLLMCall" {
        attempt: Value, message_count: Value, messages: Value, options: Value, provider: Option<String>,
        request_id: Option<String>, step: Value, tool_count: Value, tools: Value,
    }
    PostLLMCall: "PostLLMCall" {
        attempt: Value, message_count: Value, messages: Value, options: Value, provider: Option<String>,
        request_id: Option<String>, step: Value, tool_count: Value, tools: Value,
        error: Value, finish_reason: Option<String>, output_text_preview: Option<String>,
        response_id: Option<String>, status: Option<String>, tool_call_count: Value, usage: Value,
    }
    PermissionRequest: "PermissionRequest" {}
    Notification: "Notification" {}
    Interrupt: "Interrupt" {}
    SessionFork: "SessionFork" {}
}

/// True when the event comes from an observer child session (research/59 D10): in every one of the 1,329 hook payloads
/// G0 recorded, a child event has `turn_id == session_id`, and no main-session event has.
pub fn is_child(v: &Value) -> bool {
    let id = |k: &str| v.get(k).and_then(Value::as_str);
    let sid = id("session_id");
    sid.is_some() && (id("turn_id") == sid || id("child_session_id") == sid)
}

// ---------------------------------------------------------------- output

/// Largest stdout we print, in encoded bytes. Muse fails a hook at 16 KiB; JSON escaping can grow text by a third.
pub const STDOUT_LIMIT: usize = 12 * 1024;
/// Muse stores `systemMessage` cut to 1000 characters; we stay under.
const SYSTEM_MESSAGE_MAX: usize = 999;
/// `decision: block` is verified on Stop only (the engine uses it nowhere else).
const BLOCKABLE: &[&str] = &["Stop"];
/// `additionalContext` is accepted here and rejected on Stop, PreCompact and PostCompact (research/59 D5).
const CONTEXT_OK: &[&str] = &[
    "SessionStart",
    "SubagentStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PostToolBatch",
    "PreLLMCall",
    "PostLLMCall",
];
const DEFAULT_REASON: &str =
    "Torpor stopped this turn from ending but gave no reason. Run `torpor status` to see why.";
const DEFAULT_DENY: &str = "Torpor: this call is not allowed here.";
/// The engine ends its question with ASK_TAIL; a Muse hook cannot ask, so the question becomes a deny that says so.
const ASK_TAIL: &str = "Approve only if you meant it.";
const DENY_TAIL: &str = "Muse hooks cannot ask, so this was stopped. If the user wants it, they can run it themselves in their own terminal.";

/// What a hook may print. The closed camelCase key set of Muse (research/59 D5), limited to what we ever emit.
#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Output {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<Specific>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default, PartialEq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Specific {
    pub hook_event_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_decision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_decision_reason: Option<String>,
}

/// Err with the reason when Muse would reject or ignore `text` as the stdout of `event`.
pub fn validate(event: &str, text: &str) -> Result<Output, String> {
    if !text.starts_with('{') {
        return Err("stdout does not start with `{`".into());
    }
    if text.len() >= STDOUT_LIMIT {
        return Err(format!(
            "{} encoded bytes, limit {STDOUT_LIMIT}",
            text.len()
        ));
    }
    let o: Output =
        serde_json::from_str(text).map_err(|e| format!("not the closed camelCase schema: {e}"))?;
    match (o.decision.as_deref(), o.reason.as_deref()) {
        (None, None) => {}
        (Some("block"), Some(r)) if !r.trim().is_empty() && BLOCKABLE.contains(&event) => {}
        _ => return Err(format!("decision/reason not usable on {event}")),
    }
    if o.system_message
        .as_deref()
        .is_some_and(|m| m.chars().count() > SYSTEM_MESSAGE_MAX)
    {
        return Err("systemMessage is 1000 characters or more".into());
    }
    if let Some(h) = &o.hook_specific_output {
        if h.hook_event_name != event {
            return Err(format!(
                "hookEventName {:?} is not {event}",
                h.hook_event_name
            ));
        }
        if h.additional_context.is_some() && !CONTEXT_OK.contains(&event) {
            return Err(format!("additionalContext is not allowed on {event}"));
        }
        match (&h.permission_decision, &h.permission_decision_reason) {
            (None, None) => {}
            (Some(d), Some(r)) if d == "deny" && event == "PreToolUse" && !r.trim().is_empty() => {}
            _ => return Err("permissionDecision must be deny with a reason, on PreToolUse".into()),
        }
    }
    Ok(o)
}

/// A message of our own (the engine did not run): `systemMessage`, plus `additionalContext` where the event takes it.
pub fn notice(event: &str, system: &str, context: &str) -> Option<String> {
    let mut o = Output {
        system_message: Some(cut_chars(system, SYSTEM_MESSAGE_MAX)),
        ..Output::default()
    };
    if CONTEXT_OK.contains(&event) {
        o.hook_specific_output = Some(Specific {
            hook_event_name: event.to_string(),
            additional_context: Some(context.to_string()),
            ..Specific::default()
        });
    }
    let body = serde_json::to_string(&o).ok()?;
    validate(event, &body).ok().map(|_| body)
}

fn cut_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(3)).collect();
    t.push_str("...");
    t
}

/// Drops about `excess` encoded bytes from the end of `s` (at a character boundary) and marks the cut. A text byte
/// can cost several encoded bytes (a newline is two), so the drop is scaled by this text's own expansion.
fn cut_bytes(s: &mut String, excess: usize) {
    let encoded = serde_json::to_string(&*s).map_or(s.len(), |e| e.len() - 2);
    let drop = (excess * s.len()).div_ceil(encoded.max(1)) + 8;
    let mut keep = s.len().saturating_sub(drop);
    while !s.is_char_boundary(keep) {
        keep -= 1;
    }
    s.truncate(keep);
    s.push_str("\n[cut]");
}

/// The engine's Claude-style stdout turned into what Muse accepts, plus a note for everything dropped or changed.
/// `header` makes the "latest supersedes earlier" line (research/59 D9) for the notes that carry state. None = print
/// nothing (the hook then exits 0 and the turn goes on).
pub fn sanitise(
    event: &str,
    engine_out: &[u8],
    mut header: impl FnMut() -> String,
) -> (Option<String>, Vec<String>) {
    let mut notes = Vec::new();
    let raw = String::from_utf8_lossy(engine_out);
    if raw.trim().is_empty() {
        return (None, notes);
    }
    let map = match serde_json::from_str::<Value>(raw.trim()) {
        Ok(Value::Object(m)) => m,
        _ => {
            notes.push("engine stdout is not a JSON object: dropped".into());
            return (None, notes);
        }
    };
    for k in map.keys() {
        if !["decision", "reason", "systemMessage", "hookSpecificOutput"].contains(&k.as_str()) {
            notes.push(format!("engine output key {k} dropped"));
        }
    }
    let text = |m: &Map<String, Value>, k: &str| {
        m.get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
    };
    let mut out = Output {
        system_message: text(&map, "systemMessage").map(|m| cut_chars(&m, SYSTEM_MESSAGE_MAX)),
        ..Output::default()
    };
    if let Some(d) = map.get("decision") {
        if d.as_str() == Some("block") && BLOCKABLE.contains(&event) {
            out.decision = Some("block".into());
            out.reason = Some(text(&map, "reason").unwrap_or_else(|| {
                notes.push("block without a reason: default reason used".into());
                DEFAULT_REASON.into()
            }));
        } else {
            notes.push(format!("decision {d} dropped on {event}"));
        }
    }
    if let Some(Value::Object(h)) = map.get("hookSpecificOutput") {
        let mut spec = Specific {
            hook_event_name: event.to_string(),
            ..Specific::default()
        };
        if let Some(c) = text(h, "additionalContext") {
            if !CONTEXT_OK.contains(&event) {
                notes.push(format!("additionalContext dropped: not allowed on {event}"));
            } else if matches!(event, "SessionStart" | "UserPromptSubmit") {
                spec.additional_context = Some(format!("{}\n{c}", header()));
            } else {
                spec.additional_context = Some(c);
            }
        }
        if let Some(d) = h.get("permissionDecision").and_then(Value::as_str) {
            if event == "PreToolUse" && (d == "deny" || d == "ask") {
                let mut why =
                    text(h, "permissionDecisionReason").unwrap_or_else(|| DEFAULT_DENY.into());
                if d == "ask" {
                    notes.push(
                        "permissionDecision ask changed to deny: Muse hooks cannot ask".into(),
                    );
                    why = why.replace(ASK_TAIL, DENY_TAIL);
                }
                spec.permission_decision = Some("deny".into());
                spec.permission_decision_reason = Some(why);
            } else {
                notes.push(format!("permissionDecision {d} dropped on {event}"));
            }
        }
        if spec.additional_context.is_some() || spec.permission_decision.is_some() {
            out.hook_specific_output = Some(spec);
        }
    }
    if out == Output::default() {
        return (None, notes);
    }
    let mut body = serde_json::to_string(&out).unwrap_or_default();
    // Cut the longest text field until it fits; a few rounds always do it, and what is still too big is dropped below.
    for _ in 0..16 {
        if body.len() < STDOUT_LIMIT {
            break;
        }
        let excess = body.len() - STDOUT_LIMIT + 256;
        let (ctx, why) = match out.hook_specific_output.as_mut() {
            Some(s) => (
                s.additional_context.as_mut(),
                s.permission_decision_reason.as_mut(),
            ),
            None => (None, None),
        };
        let longest = [ctx, why, out.reason.as_mut(), out.system_message.as_mut()]
            .into_iter()
            .flatten()
            .max_by_key(|s| s.len());
        let Some(longest) = longest else { break };
        cut_bytes(longest, excess);
        notes.push("a field was cut to keep stdout under the Muse limit".into());
        body = serde_json::to_string(&out).unwrap_or_default();
    }
    match validate(event, &body) {
        Ok(_) => (Some(body), notes),
        Err(e) => {
            notes.push(format!(
                "output rejected by our own validator, dropped: {e}"
            ));
            (None, notes)
        }
    }
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

    #[test]
    fn negative_control_known_bad_outputs_are_rejected() {
        let bad = [
            (
                "Stop",
                r#"{"decision":"block","reason":"x","stop_reason":"y"}"#,
            ), // snake_case key
            ("Stop", r#"{"decision":"block","reason":"x","foo":1}"#), // unknown key
            ("Stop", r#"{"decision":"block"}"#),                      // no reason
            ("Stop", r#"{"decision":"block","reason":"  "}"#),        // blank reason
            (
                "Stop",
                r#"{"hookSpecificOutput":{"hookEventName":"Stop","additionalContext":"x"}}"#,
            ),
            ("PreToolUse", r#"{"decision":"block","reason":"x"}"#), // legacy form: Muse rejects it
            (
                "PreToolUse",
                r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"x"}}"#,
            ),
            (
                "PreToolUse",
                r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny"}}"#,
            ),
            (
                "PostToolUse",
                r#"{"hookSpecificOutput":{"hookEventName":"Stop","additionalContext":"x"}}"#,
            ),
            ("Stop", "plain text"),
            ("Stop", r#"[{"decision":"block"}]"#),
        ];
        for (event, text) in bad {
            assert!(validate(event, text).is_err(), "{event}: {text}");
        }
        let too_long = format!(r#"{{"systemMessage":"{}"}}"#, "x".repeat(1000));
        assert!(validate("Stop", &too_long).is_err());
        let too_big = format!(r#"{{"systemMessage":"{}"}}"#, "\\n".repeat(7000));
        assert!(validate("Stop", &too_big).is_err());
    }

    #[test]
    fn good_outputs_are_accepted() {
        let good = [
            ("Stop", r#"{"decision":"block","reason":"prove it"}"#),
            ("Stop", r#"{"systemMessage":"Torpor saved this turn."}"#),
            (
                "UserPromptSubmit",
                r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"x"}}"#,
            ),
            (
                "PreToolUse",
                r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"no"}}"#,
            ),
        ];
        for (event, text) in good {
            validate(event, text).unwrap_or_else(|e| panic!("{event}: {text}: {e}"));
        }
    }

    fn clean(event: &str, engine: &str) -> (Option<String>, Vec<String>) {
        sanitise(event, engine.as_bytes(), || "[Torpor note #7]".into())
    }

    #[test]
    fn sanitise_fixes_what_the_engine_prints_for_muse() {
        // a Stop gate block passes through unchanged
        let (o, n) = clean("Stop", r#"{"decision":"block","reason":"prove X"}"#);
        assert_eq!(o.unwrap(), r#"{"decision":"block","reason":"prove X"}"#);
        assert!(n.is_empty());
        // an empty reason gets a default one
        let (o, _) = clean("Stop", r#"{"decision":"block","reason":""}"#);
        assert!(o.unwrap().contains("torpor status"));
        // unknown and snake_case keys are dropped, the rest survives
        let (o, n) = clean(
            "Stop",
            r#"{"systemMessage":"saved","stop_reason":"y","continue":false}"#,
        );
        assert_eq!(o.unwrap(), r#"{"systemMessage":"saved"}"#);
        assert_eq!(n.len(), 2);
        // additionalContext on Stop is dropped, not forwarded
        let (o, _) = clean(
            "Stop",
            r#"{"hookSpecificOutput":{"hookEventName":"Stop","additionalContext":"x"}}"#,
        );
        assert!(o.is_none());
        // the engine says PostToolUse for a failure: the event name is rewritten
        let (o, _) = clean(
            "PostToolUseFailure",
            r#"{"hookSpecificOutput":{"hookEventName":"PostToolUse","additionalContext":"hi"}}"#,
        );
        assert_eq!(
            o.unwrap(),
            r#"{"hookSpecificOutput":{"hookEventName":"PostToolUseFailure","additionalContext":"hi"}}"#
        );
        // state notes get the sequence header; ask becomes deny
        let (o, _) = clean(
            "UserPromptSubmit",
            r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"state"}}"#,
        );
        assert!(o.unwrap().contains("[Torpor note #7]\\nstate"));
        let (o, n) = clean(
            "PreToolUse",
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"sure?"}}"#,
        );
        assert!(o.unwrap().contains(r#""permissionDecision":"deny""#));
        assert_eq!(n.len(), 1);
        let ask = format!("it deletes. {ASK_TAIL}");
        let engine = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "ask", "permissionDecisionReason": ask}});
        let (o, _) = clean("PreToolUse", &engine.to_string());
        let o = o.unwrap();
        assert!(
            o.contains("it deletes. Muse hooks cannot ask") && !o.contains("Approve"),
            "{o}"
        );
        // not JSON, or not an object: nothing is printed
        assert!(clean("Stop", "hello").0.is_none());
        assert!(clean("Stop", "[1]").0.is_none());
        assert!(clean("Stop", "").0.is_none());
    }

    #[test]
    fn sanitise_cuts_long_fields_to_fit() {
        // 9,900 two-byte characters plus escapes would be far over the limit
        let big = "\u{5d0}\n".repeat(9_900);
        let engine = json!({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": big}}).to_string();
        let (o, _) = clean("UserPromptSubmit", &engine);
        let o = o.unwrap();
        assert!(o.len() < STDOUT_LIMIT, "{}", o.len());
        validate("UserPromptSubmit", &o).unwrap();
        // most of the text survives the cut, not only the marker (a newline costs two encoded bytes)
        let v: Value = serde_json::from_str(&o).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(ctx.starts_with("[Torpor note #7]\n\u{5d0}") && ctx.ends_with("\n[cut]"));
        assert!(
            o.len() > STDOUT_LIMIT - 1500,
            "cut far too much: {}",
            o.len()
        );
        // a huge deny reason is cut as well, and a field nobody can cut ends in a drop, not a loop
        let deny = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": "x\n".repeat(50_000)}}).to_string();
        let (o, _) = clean("PreToolUse", &deny);
        let o = o.unwrap();
        assert!(o.len() < STDOUT_LIMIT && o.contains(r#""permissionDecision":"deny""#));
        let sys = json!({"systemMessage": "y".repeat(5000)}).to_string();
        let (o, _) = clean("Stop", &sys);
        assert!(o.unwrap().len() < 1100);
    }
}

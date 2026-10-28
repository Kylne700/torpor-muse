//! What Muse accepts from a hook: the closed output keys, the size limits, the cuts to fit them and `validate`.

use serde::{Deserialize, Serialize};

/// Largest stdout we print, in encoded bytes. Muse fails a hook at 16 KiB; JSON escaping can grow text by a third.
pub const STDOUT_LIMIT: usize = 12 * 1024;
/// Muse stores `systemMessage` cut to 1000 characters; we stay under.
pub(super) const SYSTEM_MESSAGE_MAX: usize = 999;
/// Muse shows `systemMessage` on one line and drops its newlines, so lines ran together (L3 N7): they get this between.
pub(super) const JOIN: &str = " — ";
/// `decision: block` is verified on Stop only (the engine uses it nowhere else).
pub(super) const BLOCKABLE: &[&str] = &["Stop"];
/// `additionalContext` is accepted here and rejected on Stop, PreCompact and PostCompact (research/59 D5).
pub(super) const CONTEXT_OK: &[&str] = &[
    "SessionStart",
    "SubagentStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
];

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

pub(super) fn cut_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut t: String = s.chars().take(max.saturating_sub(3)).collect();
    t.push_str("...");
    t
}

/// Drops about `excess` encoded bytes from the end of `s` (at a character boundary) and marks the cut. A text byte
/// can cost several encoded bytes (a newline is two), so the drop is scaled by this text's own expansion.
pub(super) fn cut_bytes(s: &mut String, excess: usize) {
    let encoded = serde_json::to_string(&*s).map_or(s.len(), |e| e.len() - 2);
    let drop = (excess * s.len()).div_ceil(encoded.max(1)) + 8;
    let mut keep = s.len().saturating_sub(drop);
    while !s.is_char_boundary(keep) {
        keep -= 1;
    }
    s.truncate(keep);
    s.push_str("\n[cut]");
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

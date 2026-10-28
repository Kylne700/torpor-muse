//! Composes the output Torpor writes itself: a note beside the engine's output, a deny and a Stop block.

use super::output::{
    CONTEXT_OK, JOIN, Output, STDOUT_LIMIT, SYSTEM_MESSAGE_MAX, Specific, cut_bytes, cut_chars,
    validate,
};

/// A message of our own (the engine did not run): `systemMessage`, plus `additionalContext` where the event takes it.
/// An empty `system` or `context` leaves that channel out.
pub fn notice(event: &str, system: &str, context: &str) -> Option<String> {
    let mut o = Output {
        system_message: (!system.is_empty()).then(|| cut_chars(system, SYSTEM_MESSAGE_MAX)),
        ..Output::default()
    };
    if CONTEXT_OK.contains(&event) && !context.is_empty() {
        o.hook_specific_output = Some(Specific {
            hook_event_name: event.to_string(),
            additional_context: Some(context.to_string()),
            ..Specific::default()
        });
    }
    let body = serde_json::to_string(&o).ok()?;
    validate(event, &body).ok().map(|_| body)
}

/// The engine's output `body` (None or blank: it printed nothing) with a note of our own added: `system` after its
/// `systemMessage` (before it when `first`; the engine's part is the one cut to fit), `context` after its
/// `additionalContext` (where the event takes one). None = `body` is not valid for `event`, or the two together do
/// not fit; the caller then keeps `body` as it is. An empty `system` or `context` adds nothing to that channel.
pub fn with_note(
    event: &str,
    body: Option<&str>,
    system: &str,
    context: &str,
    first: bool,
) -> Option<String> {
    let Some(body) = body.filter(|b| !b.trim().is_empty()) else {
        return notice(event, system, context);
    };
    let o = merge(event, validate(event, body).ok()?, system, context, first);
    let merged = serde_json::to_string(&o).ok()?;
    validate(event, &merged).ok().map(|_| merged)
}

/// `with_note` for a note that must get through because it holds the user's data (the database note): when the
/// engine's output and the note together reach Muse's limit, the end of the ENGINE's `additionalContext` is cut
/// (marked `[cut]`) and never the note. None = still too long without any context of the engine's, or not valid.
pub fn with_note_fit(
    event: &str,
    body: Option<&str>,
    system: &str,
    context: &str,
    first: bool,
) -> Option<String> {
    let Some(body) = body.filter(|b| !b.trim().is_empty()) else {
        return notice(event, system, context);
    };
    let engine = validate(event, body).ok()?;
    let theirs = engine
        .hook_specific_output
        .as_ref()
        .and_then(|h| h.additional_context.clone());
    let mut excess = 0;
    for _ in 0..6 {
        let mut o = engine.clone();
        if let (Some(c), Some(h)) = (&theirs, o.hook_specific_output.as_mut()) {
            let mut c = c.clone();
            if excess > 0 {
                cut_bytes(&mut c, excess);
            }
            h.additional_context = Some(c);
        }
        let merged = serde_json::to_string(&merge(event, o, system, context, first)).ok()?;
        if merged.len() < STDOUT_LIMIT {
            return validate(event, &merged).ok().map(|_| merged);
        }
        theirs.as_ref()?; // nothing of the engine's to cut
        excess += merged.len() - STDOUT_LIMIT + 1; // the cut text re-encodes unevenly: measure again
    }
    None
}

/// `body` (the engine's PreToolUse answer; None or blank: it printed nothing) with a deny and its reason added: the
/// call does not run and the model is told `why`. None = the result is not valid for `event`.
pub fn deny(event: &str, body: Option<&str>, why: &str) -> Option<String> {
    let mut o = match body.filter(|b| !b.trim().is_empty()) {
        Some(b) => validate(event, b).unwrap_or_default(),
        None => Output::default(),
    };
    let h = o.hook_specific_output.get_or_insert_with(|| Specific {
        hook_event_name: event.to_string(),
        ..Specific::default()
    });
    h.permission_decision = Some("deny".into());
    h.permission_decision_reason = Some(why.to_string());
    let text = serde_json::to_string(&o).ok()?;
    validate(event, &text).ok().map(|_| text)
}

/// `body` (the engine's Stop answer; None or blank: it printed nothing) with a block added: Muse runs the turn on and
/// the model is given `reason`. The engine's `systemMessage` stays. None = `body` or the result is not valid for Stop.
pub fn block(body: Option<&str>, reason: &str) -> Option<String> {
    let mut o = match body.filter(|b| !b.trim().is_empty()) {
        Some(b) => validate("Stop", b).ok()?,
        None => Output::default(),
    };
    o.decision = Some("block".into());
    o.reason = Some(reason.to_string());
    let text = serde_json::to_string(&o).ok()?;
    validate("Stop", &text).ok().map(|_| text)
}

/// The engine's (valid) output `o` with our `system` and `context` added, see `with_note`. Not yet checked for size.
fn merge(event: &str, mut o: Output, system: &str, context: &str, first: bool) -> Output {
    if !system.is_empty() {
        let room = SYSTEM_MESSAGE_MAX.saturating_sub(system.chars().count() + JOIN.chars().count());
        o.system_message = Some(match o.system_message.take() {
            Some(m) if first => format!("{system}{JOIN}{}", cut_chars(&m, room)),
            Some(m) => format!("{}{JOIN}{system}", cut_chars(&m, room)),
            None => cut_chars(system, SYSTEM_MESSAGE_MAX),
        });
    }
    if CONTEXT_OK.contains(&event) && !context.is_empty() {
        let h = o.hook_specific_output.get_or_insert_with(|| Specific {
            hook_event_name: event.to_string(),
            ..Specific::default()
        });
        h.additional_context = Some(match h.additional_context.take() {
            Some(c) => format!("{c}\n\n{context}"),
            None => context.to_string(),
        });
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn a_note_is_added_to_what_the_engine_printed_or_stands_alone() {
        let note = |event, body: Option<&str>| {
            with_note(event, body, "SYS", "CTX", false).unwrap_or_default()
        };
        // nothing printed: the note alone, on both channels where the event takes context
        for body in [None, Some(""), Some("  \n")] {
            let o = note("UserPromptSubmit", body);
            assert_eq!(
                o,
                notice("UserPromptSubmit", "SYS", "CTX").unwrap(),
                "{body:?}"
            );
        }
        assert_eq!(note("Stop", None), r#"{"systemMessage":"SYS"}"#);
        // the engine's own message and context stay, ours follow
        let engine = r#"{"systemMessage":"saved","hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"[H]\nx"}}"#;
        let o = note("UserPromptSubmit", Some(engine));
        let v: Value = serde_json::from_str(&o).unwrap();
        assert_eq!(v["systemMessage"], "saved — SYS");
        assert_eq!(
            v["hookSpecificOutput"]["additionalContext"],
            "[H]\nx\n\nCTX"
        );
        // a decision and a reason stay; no context where the event takes none
        let o = note("Stop", Some(r#"{"decision":"block","reason":"prove X"}"#));
        assert_eq!(
            o,
            r#"{"decision":"block","reason":"prove X","systemMessage":"SYS"}"#
        );
        // an engine message near the limit is cut to make room, and the result is still valid
        let long = json!({"systemMessage": "y".repeat(999)}).to_string();
        let o = with_note("SessionStart", Some(&long), &"n".repeat(400), "CTX", false).unwrap();
        let sys = serde_json::from_str::<Value>(&o).unwrap()["systemMessage"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            sys.chars().count() <= SYSTEM_MESSAGE_MAX
                && sys.ends_with(&"n".repeat(400))
                && sys.contains("y..."),
            "{}",
            sys.len()
        );
        validate("SessionStart", &o).unwrap();
        // `first` (the database line, L3 N2): ours leads, the engine's message is the part cut
        let o = with_note("Stop", Some(&long), &"n".repeat(400), "", true).unwrap();
        let sys = serde_json::from_str::<Value>(&o).unwrap()["systemMessage"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(
            sys.chars().count() <= SYSTEM_MESSAGE_MAX
                && sys.starts_with(&format!("{} — y", "n".repeat(400)))
                && sys.ends_with("y..."),
            "{sys}"
        );
        // output that is not valid for the event, or does not fit with the note: None
        assert_eq!(
            with_note("Stop", Some("not json"), "SYS", "CTX", false),
            None
        );
        let big = json!({"hookSpecificOutput": {"hookEventName": "UserPromptSubmit", "additionalContext": "z".repeat(STDOUT_LIMIT - 400)}}).to_string();
        assert_eq!(
            with_note(
                "UserPromptSubmit",
                Some(&big),
                "SYS",
                &"c".repeat(600),
                false
            ),
            None
        );
    }

    #[test]
    fn a_note_that_must_get_through_cuts_the_engines_context_and_never_its_own() {
        // quotes, newlines, backslashes and multibyte text cost 2 to 4 encoded bytes each; the plain tail costs 1
        let hostile = "\"q\"\n\\b\\ \t\u{20ac}\u{e9}\u{1f642} ";
        let engine = |chars: usize| {
            let ctx: String = hostile
                .chars()
                .cycle()
                .take(chars)
                .chain("p".repeat(4000).chars())
                .collect();
            json!({"systemMessage": "s".repeat(990), "hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": ctx}}).to_string()
        };
        let mut chars = 3000;
        while engine(chars + 20).len() < STDOUT_LIMIT - 40 {
            chars += 20;
        }
        let body = engine(chars);
        validate("SessionStart", &body).unwrap();
        let note = format!("[note] {}\nlast line", "n".repeat(3000));
        assert_eq!(
            with_note("SessionStart", Some(&body), "", &note, false),
            None,
            "plain with_note gives up"
        );
        let o = with_note_fit("SessionStart", Some(&body), "", &note, false).unwrap();
        validate("SessionStart", &o).unwrap();
        let v: Value = serde_json::from_str(&o).unwrap();
        let ctx = v["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap();
        assert!(
            ctx.starts_with("\"q\"\n\\b")
                && ctx.contains("\n[cut]\n\n[note] n")
                && ctx.ends_with("n\nlast line"),
            "{}",
            &ctx[ctx.len().saturating_sub(120)..]
        );
        assert_eq!(v["systemMessage"].as_str().unwrap(), "s".repeat(990));
        // it fits: nothing is cut and the result is what `with_note` gives
        let small = "[note] small";
        let fit = with_note_fit(
            "SessionStart",
            Some(
                r#"{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"x"}}"#,
            ),
            "",
            small,
            false,
        );
        assert_eq!(
            fit.as_deref(),
            with_note("SessionStart", Some(r#"{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"x"}}"#), "", small, false).as_deref()
        );
        // nothing of the engine's to cut and still too long: None
        let only_message = json!({"systemMessage": "s"}).to_string();
        assert_eq!(
            with_note_fit(
                "SessionStart",
                Some(&only_message),
                "",
                &"n".repeat(STDOUT_LIMIT),
                false
            ),
            None
        );
    }

    #[test]
    fn a_stop_block_keeps_the_engines_line_and_is_valid_for_stop_only() {
        let line = json!({"systemMessage": "Torpor: save point 3"}).to_string();
        let b = block(Some(&line), "Before you finish: say it.").unwrap();
        let o = validate("Stop", &b).unwrap();
        assert_eq!(o.decision.as_deref(), Some("block"));
        assert_eq!(o.reason.as_deref(), Some("Before you finish: say it."));
        assert_eq!(o.system_message.as_deref(), Some("Torpor: save point 3"));
        // the engine printed nothing: the block stands alone
        let alone = block(None, "r").unwrap();
        assert_eq!(alone, r#"{"decision":"block","reason":"r"}"#);
        assert_eq!(block(Some(" "), "r").as_deref(), Some(alone.as_str()));
        // no reason, or an engine answer Muse would reject: no block
        assert_eq!(block(None, " "), None);
        assert_eq!(block(Some("not json"), "r"), None);
        assert!(validate("UserPromptSubmit", &b).is_err());
    }
}

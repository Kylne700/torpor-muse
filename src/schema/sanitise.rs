//! Turns what the engine printed (Claude-style JSON) into output `validate` accepts and a person under Muse can follow.

use super::output::{
    BLOCKABLE, CONTEXT_OK, JOIN, Output, STDOUT_LIMIT, SYSTEM_MESSAGE_MAX, Specific, cut_bytes,
    cut_chars, validate,
};
use serde_json::{Map, Value};

const DEFAULT_REASON: &str =
    "Torpor stopped this turn from ending but gave no reason. Run `torpor status` to see why.";
const DEFAULT_DENY: &str = "Torpor: this call is not allowed here.";
/// The engine ends its question with ASK_TAIL; a Muse hook cannot ask, so the question becomes a deny that says so.
const ASK_TAIL: &str = "Approve only if you meant it.";
const DENY_TAIL: &str = "Muse hooks cannot ask, so this was stopped. If the user wants it, they can run it themselves in their own terminal.";

/// The engine's subcommands (`torpor help`): `torpor <one of these>` in text is a command line.
const SUBCOMMANDS: &[&str] = &[
    "status",
    "log",
    "show",
    "back",
    "undo",
    "recover",
    "init",
    "statusline",
    "gc",
    "quiet",
    "keep",
    "promise",
    "check",
    "find-break",
    "gate",
    "off",
    "on",
    "retain",
    "runner",
    "forget",
    "version",
    "help",
];

/// `text` for the model with each `torpor <subcommand>` written as `cmd <subcommand>`. The model's shell may not have
/// the engine's folder on its PATH, so `cmd` is the engine's own path. A word that only ends in `torpor` (a path, a
/// name with `-` or `_`) is left alone.
fn runnable(text: &str, cmd: &str) -> String {
    let (mut out, mut done) = (String::with_capacity(text.len()), 0);
    for (i, _) in text.match_indices("torpor ") {
        let glued = text[..i]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || "_-/.~".contains(c));
        let word: String = text[i + 7..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if !glued && SUBCOMMANDS.contains(&word.as_str()) {
            out.push_str(&text[done..i]);
            out.push_str(cmd);
            done = i + 6;
        }
    }
    out.push_str(&text[done..]);
    out
}

/// What the engine tells the AI to ask the person for, said as it is true under Muse: the AI runs `recover` itself (it
/// is not on the engine's list of the person's own commands).
const ASK_RECOVER: &str = "Ask the user to run torpor recover before editing.";
const RUN_RECOVER: &str = "Run torpor recover yourself before editing; it is safe.";

/// The engine nudges the AI to run a check after each edit of a file the check covers. With the gate off (Muse's
/// default) nothing waits for it, so the line is not passed on.
const TOUCH_NUDGE: &str = "Torpor: edit touches promise";

/// One line of the engine's systemMessage as the person should read it. Under Muse they talk to the AI and do not type
/// `torpor`, so the engine's "type this" tails are dropped or made into something to ask the AI, and a failed check no
/// longer says it "would have stopped the AI" (the gate is off). The engine's texts are frozen: only this rewords them.
fn for_user(line: &str) -> String {
    const UNDO_TAIL: &str = " · undo it: torpor undo ";
    let mut s = line
        .replace(" See the turns: torpor log. Undo one: torpor undo N.", "")
        .replace(
            " not saved (torpor status)",
            " not saved (ask the AI to run torpor status)",
        );
    if let Some(i) = s.find(UNDO_TAIL) {
        let end = i + UNDO_TAIL.len();
        let digits = s[end..].bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 {
            s.replace_range(i..end + digits, "");
        }
    }
    failed_check(&s).unwrap_or(s)
}

/// gate.rs `fail_line`, `Torpor: promise #3 (name) failed after this turn (3 of 3 runs; it passed on ...). <tail> torpor
/// promise show 3`, as `Torpor: check #3 (name) failed after this turn (...). (ask the AI: torpor promise show 3)`. Any
/// other line is None.
fn failed_check(line: &str) -> Option<String> {
    const MID: &str = ") failed after this turn (";
    const SHOW: &str = " torpor promise show ";
    let rest = line.strip_prefix("Torpor: promise #")?;
    // the last MID: the words are the person's and may hold anything, the rest of the line is the engine's
    let (head, tail) = rest.rsplit_once(MID)?;
    let (what, id) = tail.rsplit_once(SHOW)?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let what = what.replace(
        " It would have stopped the AI, but it is in shadow mode.",
        "",
    );
    Some(format!(
        "Torpor: check #{head}{MID}{what} (ask the AI: torpor promise show {id})"
    ))
}

/// The engine's Claude-style stdout turned into what Muse accepts, plus a note for everything dropped or changed.
/// `header` makes the "latest supersedes earlier" line (research/59 D9) for the notes that carry state. None = print
/// nothing (the hook then exits 0 and the turn goes on).
pub fn sanitise(
    event: &str,
    engine_out: &[u8],
    torpor: &str,
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
            .map(|s| s.replace("Claude Code", "Muse Code")) // the engine's text names another product
    };
    // What the model reads names the engine by `torpor` (the path of the engine that ran, see `runnable`).
    let for_model = |m: &Map<String, Value>, k: &str| {
        text(m, k).map(|s| runnable(&s.replace(ASK_RECOVER, RUN_RECOVER), torpor))
    };
    let mut out = Output {
        system_message: text(&map, "systemMessage").map(|m| {
            let lines: Vec<String> = m
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(for_user)
                .collect();
            cut_chars(&lines.join(JOIN), SYSTEM_MESSAGE_MAX)
        }),
        ..Output::default()
    };
    if let Some(d) = map.get("decision") {
        if d.as_str() == Some("block") && BLOCKABLE.contains(&event) {
            out.decision = Some("block".into());
            out.reason = Some(for_model(&map, "reason").unwrap_or_else(|| {
                notes.push("block without a reason: default reason used".into());
                runnable(DEFAULT_REASON, torpor)
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
        let context = for_model(h, "additionalContext").map(|c| match event {
            "PostToolUse" | "PostToolUseFailure" => {
                let keep = c
                    .lines()
                    .filter(|l| !l.trim_start().starts_with(TOUCH_NUDGE));
                keep.collect::<Vec<_>>().join("\n")
            }
            _ => c,
        });
        if let Some(c) = context.filter(|c| !c.trim().is_empty()) {
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
                    for_model(h, "permissionDecisionReason").unwrap_or_else(|| DEFAULT_DENY.into());
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
mod tests;

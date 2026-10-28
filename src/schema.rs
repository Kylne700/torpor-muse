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

// ---------------------------------------------------------------- output

/// Largest stdout we print, in encoded bytes. Muse fails a hook at 16 KiB; JSON escaping can grow text by a third.
pub const STDOUT_LIMIT: usize = 12 * 1024;
/// Muse stores `systemMessage` cut to 1000 characters; we stay under.
const SYSTEM_MESSAGE_MAX: usize = 999;
/// Muse shows `systemMessage` on one line and drops its newlines, so lines ran together (L3 N7): they get this between.
const JOIN: &str = " — ";
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
        sanitise(event, engine.as_bytes(), "torpor", || {
            "[Torpor note #7]".into()
        })
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

    /// The person under Muse talks to the AI and never types `torpor`: these are the engine's own lines (engine
    /// a2289f4, hook.rs 675 and 1338-1361, gate.rs 241-258 and 683-693) as the person, or the model, reads them.
    #[test]
    fn what_the_engine_tells_a_person_to_type_is_not_passed_on() {
        let user = |event: &str, msg: &str| {
            let engine = json!({ "systemMessage": msg });
            let (o, n) = clean(event, &engine.to_string());
            assert!(n.is_empty(), "{n:?}");
            let o: Value = serde_json::from_str(&o.unwrap()).unwrap();
            o["systemMessage"].as_str().unwrap().to_string()
        };
        // the Stop line loses the undo command, and what is not saved is for the AI to look up
        assert_eq!(
            user(
                "Stop",
                "Torpor: save point 31 · turn 5 changed 1 file (1 edit) · undo it: torpor undo 5 · saved before npm test · 2 files not saved (torpor status)"
            ),
            "Torpor: save point 31 · turn 5 changed 1 file (1 edit) · saved before npm test · 2 files not saved (ask the AI to run torpor status)"
        );
        let one = "Torpor: save point 3 · turn 1 changed 2 files (2 edits) · undo it: torpor undo 1 · 1 file not saved (torpor status)";
        assert_eq!(
            user("Stop", one),
            "Torpor: save point 3 · turn 1 changed 2 files (2 edits) · 1 file not saved (ask the AI to run torpor status)"
        );
        // a line that only looks like it is left as it is (a tail with no number)
        let odd = "Torpor: save point 3. undo it: torpor undo x";
        assert_eq!(user("Stop", odd), odd);
        // the welcome
        assert_eq!(
            user(
                "SessionStart",
                "Torpor is saving this folder at the start and end of every turn. See the turns: torpor log. Undo one: torpor undo N."
            ),
            "Torpor is saving this folder at the start and end of every turn."
        );
        // a failed check: "check", no "shadow mode" sentence, the command is something to ask the AI for
        assert_eq!(
            user(
                "Stop",
                "Torpor: promise #1 (proof page answers) failed after this turn (3 of 3 runs; it passed on the kept version (save point 3)). It would have stopped the AI, but it is in shadow mode. torpor promise show 1"
            ),
            "Torpor: check #1 (proof page answers) failed after this turn (3 of 3 runs; it passed on the kept version (save point 3)). (ask the AI: torpor promise show 1)"
        );
        assert_eq!(
            user(
                "Stop",
                "Torpor: promise #12 (a (b) failed after this turn (x) c) failed after this turn (3 of 3 runs; it passed on the version from the end of turn 4). The AI was not stopped. torpor promise show 12"
            ),
            "Torpor: check #12 (a (b) failed after this turn (x) c) failed after this turn (3 of 3 runs; it passed on the version from the end of turn 4). The AI was not stopped. (ask the AI: torpor promise show 12)"
        );
        // the other promise lines, a "new promise" line and a failure line cut short are the engine's own words
        for same in [
            "Torpor: promise #2 (api answers) passed and failed on the same version, so it is flaky and advisory only. torpor promise show 2",
            "Torpor: promise #6 (w) failed after this turn (3 of 3 runs). torpor promise show six",
            "Torpor: new promise #1 \"x\" (shadow), from your words at turn 1. Drop it: torpor promise drop 1",
        ] {
            assert_eq!(user("Stop", same), same);
        }
        // what the model reads: it runs `recover` itself
        let (o, _) = clean(
            "UserPromptSubmit",
            r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"Torpor: a restore was interrupted and some files are mixed. Ask the user to run torpor recover before editing."}}"#,
        );
        let o = o.unwrap();
        assert!(
            o.contains("Run torpor recover yourself before editing; it is safe.")
                && !o.contains("Ask the user"),
            "{o}"
        );
        // the nudge to run a check after an edit is not passed on; nothing else is lost, and nothing is noted
        let nudge = "Torpor: edit touches promises #1, #2; `torpor check 1 2` before finishing";
        for event in ["PostToolUse", "PostToolUseFailure"] {
            let engine = json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": nudge}});
            assert_eq!(clean(event, &engine.to_string()), (None, vec![]));
            let engine = json!({"hookSpecificOutput": {"hookEventName": "PostToolUse", "additionalContext": format!("Torpor: first\n{nudge}\nTorpor: last")}});
            let (o, n) = clean(event, &engine.to_string());
            assert!(n.is_empty());
            assert_eq!(
                o.unwrap(),
                format!(
                    r#"{{"hookSpecificOutput":{{"hookEventName":"{event}","additionalContext":"Torpor: first\nTorpor: last"}}}}"#
                )
            );
        }
        // on the other events that text is not touched
        let engine = json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "permissionDecision": "deny", "permissionDecisionReason": nudge}});
        assert!(
            clean("PreToolUse", &engine.to_string())
                .0
                .unwrap()
                .contains("edit touches promises")
        );
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

    #[test]
    fn the_engine_is_named_by_its_path_for_the_model_only() {
        let at = "/opt/e/torpor";
        let run = |event: &str, engine: &str| {
            sanitise(event, engine.as_bytes(), at, || "[H]".into())
                .0
                .unwrap()
        };
        // context and a block reason are for the model; the user's systemMessage keeps the plain name
        let engine = json!({"systemMessage": "run `torpor status`", "hookSpecificOutput": {"hookEventName": "UserPromptSubmit",
            "additionalContext": "`torpor check 3` before finishing; then torpor undo."}})
        .to_string();
        let o = run("UserPromptSubmit", &engine);
        assert!(
            o.contains(r#""systemMessage":"run `torpor status`""#),
            "{o}"
        );
        assert!(
            o.contains("`/opt/e/torpor check 3` before finishing; then /opt/e/torpor undo."),
            "{o}"
        );
        let o = run(
            "Stop",
            r#"{"decision":"block","reason":"`torpor check 2` failed"}"#,
        );
        assert!(o.contains("`/opt/e/torpor check 2` failed"), "{o}");
        // the default reasons are for the model too
        assert!(run("Stop", r#"{"decision":"block"}"#).contains("Run `/opt/e/torpor status`"));
        let deny = r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"ask","permissionDecisionReason":"`torpor undo` first"}}"#;
        assert!(run("PreToolUse", deny).contains("`/opt/e/torpor undo` first"));
    }

    #[test]
    fn only_a_torpor_command_is_rewritten() {
        let at = "'/a b/torpor'";
        assert_eq!(
            runnable(
                "run `torpor check 3`, `torpor find-break` or torpor gate",
                at
            ),
            "run `'/a b/torpor' check 3`, `'/a b/torpor' find-break` or '/a b/torpor' gate"
        );
        for same in [
            "torpor-muse doctor",
            "/usr/bin/torpor check",
            "~/.local/bin/torpor undo",
            "my_torpor undo",
            "not-torpor status",
            "mytorpor status",
            "torpor is off",
            "Torpor status",
            "torpor  status",
            "torpor",
            "torpor ",
            "torpor statuses",
            "the torpor\nstatus",
        ] {
            assert_eq!(runnable(same, at), same);
        }
        assert_eq!(runnable("torpor check", "torpor"), "torpor check");
        assert_eq!(
            runnable("\u{e9}torpor check \u{5d0}torpor undo", at),
            "\u{e9}torpor check \u{5d0}torpor undo"
        );
    }

    #[test]
    fn engine_text_says_muse_code() {
        let engine = json!({"systemMessage": "Start Claude Code inside a project folder", "hookSpecificOutput": {"hookEventName": "SessionStart",
            "additionalContext": "If Claude Code asked whether you trust this folder, answer yes."}})
        .to_string();
        let o = clean("SessionStart", &engine).0.unwrap();
        assert!(
            o.contains("Start Muse Code inside") && o.contains("If Muse Code asked"),
            "{o}"
        );
        assert!(!o.contains("Claude"), "{o}");
    }

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

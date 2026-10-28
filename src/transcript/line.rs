//! One line of Muse's session log turned into the engine transcript lines it stands for.

use super::State;
use serde::Deserialize;
use serde_json::{Value, json};

/// Cheap test on the text first: most lines are of no interest, and a prompt line can be megabytes. The records are
/// then told apart by what they say about themselves, not by this text: a compaction checkpoint and `session.fork.
/// compaction` carry a whole accepted record inside. A record stored inside a `retained_frame` has escaped quotes and
/// does not pass (none of the wanted kinds were seen in a frame on 1.4.13).
fn wanted(s: &str) -> bool {
    s.contains(r#""payload_type":"runtime.user_intent.accepted""#)
        || (s.contains(r#""payload_type":"runtime.session""#)
            && [
                r#""kind":"context_compaction_installed""#,
                r#""kind":"context_block_updated""#,
                r#""kind":"context_block_update_batch_committed""#,
            ]
            .iter()
            .any(|k| s.contains(k)))
}

/// Only what is needed: the prompt text, the model messages and the replayed messages (megabytes) are skipped.
#[derive(Deserialize)]
struct Rec {
    id: Option<String>,
    payload_type: Option<String>,
    payload: Option<Payload>,
}

#[derive(Deserialize)]
struct Payload {
    intent_id: Option<String>,
    semantic_kind: Option<Sem>,
    surface: Option<String>,
    event: Option<Event>,
}

#[derive(Deserialize)]
struct Sem {
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Event {
    kind: Option<String>,
    trigger: Option<String>,
    source: Option<String>,
    text: Option<String>,
    entries: Option<Vec<Entry>>,
}

#[derive(Deserialize)]
struct Entry {
    update: Option<Block>,
}

#[derive(Deserialize)]
struct Block {
    source: Option<String>,
    text: Option<String>,
}

fn put(v: Value, out: &mut Vec<u8>) {
    out.extend_from_slice(v.to_string().as_bytes());
    out.push(b'\n');
}

pub(super) fn translate_line(s: &str, out: &mut Vec<u8>, st: &mut State) {
    if !wanted(s) {
        return;
    }
    let Ok(rec) = serde_json::from_str::<Rec>(s) else {
        st.bad += 1;
        return;
    };
    let (Some(id), Some(pl)) = (rec.id, rec.payload) else {
        return;
    };
    match (rec.payload_type.as_deref(), pl.intent_id, pl.event) {
        (Some("runtime.user_intent.accepted"), Some(intent), _) => {
            let surface = pl.surface.as_deref();
            let sem = pl.semantic_kind.and_then(|k| k.kind).unwrap_or_default();
            let human = surface == Some("main")
                && matches!(sem.as_str(), "chat" | "skill_invocation" | "plugin_command");
            let mut v = json!({
                "type": "user", "uuid": intent, "promptId": intent,
                "isSidechain": surface.is_some_and(|s| s != "main"),
                "turnOrigin": if human { "human" } else { "unknown" },
            });
            if human {
                v["promptSource"] = json!("typed");
            }
            st.users += 1;
            put(v, out);
        }
        (Some("runtime.user_intent.accepted"), None, _) => st.bad += 1,
        (Some("runtime.session"), _, Some(ev)) => match ev.kind.as_deref() {
            Some("context_compaction_installed") => {
                let manual = ev.trigger.as_deref() == Some("manual_command");
                st.bounds += 1;
                put(
                    json!({
                        "type": "system", "subtype": "compact_boundary", "uuid": id,
                        "content": "Conversation compacted",
                        "compactMetadata": { "trigger": if manual { "manual" } else { "auto" } },
                    }),
                    out,
                );
            }
            Some("context_block_updated") => context(&ev.source, &ev.text, &id, out, st),
            Some("context_block_update_batch_committed") => {
                for (i, e) in ev.entries.iter().flatten().enumerate() {
                    if let Some(b) = &e.update {
                        context(&b.source, &b.text, &format!("{id}:{i}"), out, st);
                    }
                }
            }
            _ => {}
        },
        _ => {}
    }
}

/// A block Muse took from a hook: the attachment the engine's delivery check looks for.
fn context(
    source: &Option<String>,
    text: &Option<String>,
    uuid: &str,
    out: &mut Vec<u8>,
    st: &mut State,
) {
    if source.as_deref() != Some("runtime_hook") {
        return; // session_identity and the like are Muse's own, not a hook's
    }
    let Some(text) = text else {
        st.bad += 1;
        return;
    };
    st.ctx += 1;
    put(
        json!({
            "type": "attachment", "uuid": uuid,
            "attachment": { "type": "hook_additional_context", "content": [text] },
        }),
        out,
    );
}

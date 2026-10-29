//! Muse installs some compactions by itself, in the idle time after a turn, and runs no SessionStart hook after them:
//! the model would never get the guide or the engine's note that names the saved checks. Torpor makes it up at the
//! first hook after the compaction whose text Muse puts in front of the model (a prompt, or a tool result): it asks
//! the engine as the missing SessionStart (source compact) would have and gives the answer ahead of that hook's own
//! output, once. Once = the compactions in the translated log (`transcript::State::bounds`) against the number the
//! model was told about (`Session::compact_told`). Every real SessionStart brings the second up to the first, so a
//! compaction that did get its SessionStart is not told again.
//!
//! Asking the engine twice for one compaction is safe: its SessionStart(compact) handling is keyed on the compaction
//! (the transcript's newest boundary), so the second ask for the same one prints nothing and writes nothing new.

use super::db_note::db_note;
use super::handle::reply;
use super::paths::engine_cmd;
use super::state;
use crate::{engine, liveness, schema, transcript};
use serde_json::{Map, Value};
use std::path::Path;
use std::time::Duration;

/// The longest the stand-in SessionStart may take; the hook's own engine call comes after it.
const ASK: Duration = Duration::from_millis(1500);

/// What the missing SessionStart would have told the model, until the hook's own output is there to go behind it.
pub(super) struct Lead {
    /// The engine's systemMessage and additionalContext (headed `[Torpor note #n ...]`) of that SessionStart.
    system: String,
    note: String,
    /// The guide, and the database note when the folder has database turns.
    guide: String,
}

/// SessionStart: the compactions so far count as told. UserPromptSubmit or PostToolUse of the main session: when a
/// compaction came since, the stand-in answer, and that compaction counts as told at once (of two parallel tool
/// results only one takes it; a stand-in that fails is not asked again, that would cost every hook the wait). Called
/// after `transcript::path_for` and before the hook's own engine call: the engine builds its note for a compaction
/// before it opens the next turn, so it must see the compaction first. The guide and the database note are drawn
/// here too, so the hook's own `db_note` finds nothing due and does not tell the database note a second time.
pub(super) fn lead(
    dir: &Path,
    event: &str,
    sid: &str,
    main: bool,
    payload: &Value,
) -> Option<Lead> {
    if !main || !matches!(event, "SessionStart" | "UserPromptSubmit" | "PostToolUse") {
        return None;
    }
    let bounds = transcript::read_state(&liveness::vault(), sid).map_or(0, |s| s.bounds);
    let told = state::with_session(dir, sid, |s| s.compact_told.replace(bounds)).flatten();
    if event == "SessionStart" || !told.is_some_and(|t| bounds > t) {
        return None;
    }
    let bin = engine::locate().ok()?;
    let mut ask = Map::new();
    for k in [
        "cwd",
        "model",
        "model_provider",
        "permission_mode",
        "session_id",
        "transcript_path",
    ] {
        if let Some(v) = payload.get(k) {
            ask.insert(k.into(), v.clone());
        }
    }
    ask.insert("hook_event_name".into(), "SessionStart".into());
    ask.insert("source".into(), "compact".into());
    let miss = |why: &str| {
        let msg = format!("no compaction note from torpor hook SessionStart (compact): {why}");
        state::problem(dir, event, sid, &msg);
    };
    let bytes = Value::Object(ask).to_string();
    let said = match engine::run(&["hook", "SessionStart"], Some(bytes.as_bytes()), ASK) {
        Ok(out) if out.code == 0 => {
            reply(dir, "SessionStart", sid, "", &engine_cmd(&bin), &out.stdout)
        }
        Ok(out) => {
            miss(&format!(
                "exited {}: {}",
                out.code,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
            None
        }
        Err(e) => {
            miss(&e);
            None
        }
    };
    let cwd = payload["cwd"].as_str().unwrap_or("");
    let guide = db_note(dir, "SessionStart", sid, cwd, &bin, None);
    let (system, note) = parts("SessionStart", said.as_deref());
    Some(Lead {
        system,
        note,
        guide: parts("SessionStart", guide.as_deref()).1,
    })
}

impl Lead {
    /// `body`, what the hook itself printed, behind the stand-in's note and the guide: the `#n` headers then rise in
    /// reading order, as they would with a real SessionStart before the hook. When the engine's note and the rest do
    /// not fit Muse's limit, the end of the engine's note is cut (`with_note_fit`), never the guide or the database
    /// note. None of that can be made: `body` goes out as it is.
    pub(super) fn join(self, event: &str, body: Option<String>) -> Option<String> {
        let (system, context) = parts(event, body.as_deref());
        let behind = [self.guide, context].into_iter().filter(|s| !s.is_empty());
        let behind = behind.collect::<Vec<_>>().join("\n\n");
        let first = schema::notice(event, &self.system, &self.note);
        schema::with_note_fit(event, first.as_deref(), &system, &behind, false).or(body)
    }
}

/// The systemMessage and the additionalContext of a hook's output (empty where it has none).
fn parts(event: &str, out: Option<&str>) -> (String, String) {
    let Some(o) = out.and_then(|t| schema::validate(event, t).ok()) else {
        return Default::default();
    };
    let context = o.hook_specific_output.and_then(|h| h.additional_context);
    (
        o.system_message.unwrap_or_default(),
        context.unwrap_or_default(),
    )
}

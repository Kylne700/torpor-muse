//! Typed Muse Code hook payloads (input, unknown keys kept) and outputs (exact, camelCase). Owner: S3a-core.
//!
//! Output rules come from research/59 D5 and from the strings of Muse Code 1.4.13: stdout is parsed only when it starts
//! with `{`, the key set is closed and camelCase, and ONE unknown key rejects the whole output (the hook then counts as
//! failed and the turn goes on, so a Torpor block would silently vanish). `validate` is that rule set; `sanitise` turns
//! what the engine printed (Claude-style JSON) into something `validate` accepts.

mod compose;
mod input;
mod output;
mod sanitise;

pub(crate) use compose::{block, deny, notice, with_note, with_note_fit};
#[cfg(test)]
pub(crate) use input::EVENT_NAMES;
pub(crate) use input::{Payload, is_child};
pub(crate) use output::validate;
pub(crate) use sanitise::sanitise;

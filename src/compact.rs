//! SessionStart(source=compact): rebuild context from the session log found by session_id (transcript_path is null,
//! research/59 D3); returns the additionalContext text, or None. Owner: S3a-shim.
#![allow(dead_code)] // skeleton: remove when implemented

pub fn on_compact(_payload: &serde_json::Value) -> Option<String> {
    None
}

//! Claude-style transcript for the engine. Muse sends `transcript_path: null` (research/59 D3), but the engine reads a
//! transcript for the human origin of promise words, compaction boundaries and the delivery check (research/60 AD4).
//! This module keeps one per main session, translated from Muse's own session log (never invented). Owner: S3a-shim.
#![allow(dead_code)] // skeleton: remove when implemented

/// hook.rs calls this on every main-session event before the engine call; Some(path) = put it in `transcript_path`.
pub fn path_for(_event: &str, _payload: &serde_json::Value) -> Option<String> {
    None
}

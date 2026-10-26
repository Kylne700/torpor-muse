//! Per-event heartbeats and the turn-open/turn-closed counter (research/60 AD7, AD9). Owner: S3a-shim.
#![allow(dead_code)] // skeleton: remove when implemented

/// hook.rs calls this first for every event it parsed, observer child sessions too (`main` = false, research/59 D10).
pub fn beat(_event: &str, _session_id: &str, _main: bool) {}

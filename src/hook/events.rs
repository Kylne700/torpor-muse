//! The table of the Muse hook events Torpor registers, with the settings each one gets (`install` writes them).

use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    /// The payload goes to `torpor hook <Event>`.
    Engine,
    /// Only the heartbeat is recorded (`liveness::beat`); the engine is not called.
    Beat,
}

/// One Muse hook event we register: `install` writes the settings from this table. Muse events we do not register:
/// PreLLMCall and PostLLMCall (8-9 KB of payload per model step, the engine has no use for it) and PermissionRequest,
/// Notification, Interrupt, SessionFork (never fired in G0; an interrupted tool call arrives as PostToolUseFailure, a
/// fork as SessionStart `fork`).
pub struct Ev {
    pub name: &'static str,
    pub act: Act,
    /// Muse matcher. Only SessionStart and the three tool events take one (research/59 D5).
    pub matcher: Option<&'static str>,
    /// `timeout` of the settings entry: INTEGER SECONDS (research/59 D7).
    pub timeout_s: u32,
    /// We kill the engine after this many ms: its own deadline for the event plus 1.5 s.
    pub engine_ms: u64,
}

const TOOLS: Option<&str> = Some("bash|edit_file|write_file");

/// Events earlier versions registered for a heartbeat only. `init` and `uninstall` remove their rows; a settings file
/// that still has them is answered with exit 0 and no output, and nothing is recorded.
pub const RETIRED: &[&str] = &["PostToolBatch", "PostCompact"];

pub const EVENTS: &[Ev] = &[
    Ev {
        name: "SessionStart",
        act: Act::Engine,
        matcher: Some("startup|resume|clear|compact|fork"),
        timeout_s: 10,
        engine_ms: 4_500,
    },
    Ev {
        name: "UserPromptSubmit",
        act: Act::Engine,
        matcher: None,
        timeout_s: 10,
        engine_ms: 6_500,
    },
    Ev {
        name: "PreToolUse",
        act: Act::Engine,
        matcher: TOOLS,
        timeout_s: 6,
        engine_ms: 3_500,
    },
    Ev {
        name: "PostToolUse",
        act: Act::Engine,
        matcher: TOOLS,
        timeout_s: 6,
        engine_ms: 3_500,
    },
    Ev {
        name: "PostToolUseFailure",
        act: Act::Engine,
        matcher: TOOLS,
        timeout_s: 6,
        engine_ms: 3_500,
    },
    Ev {
        name: "Stop",
        act: Act::Engine,
        matcher: None,
        timeout_s: 35,
        engine_ms: 31_500,
    },
    Ev {
        name: "StopFailure",
        act: Act::Engine,
        matcher: None,
        timeout_s: 10,
        engine_ms: 6_500,
    },
    Ev {
        name: "SessionEnd",
        act: Act::Engine,
        matcher: None,
        timeout_s: 5,
        engine_ms: 2_700,
    },
    Ev {
        name: "PreCompact",
        act: Act::Engine,
        matcher: None,
        timeout_s: 5,
        engine_ms: 2_500,
    },
    Ev {
        name: "SubagentStart",
        act: Act::Beat,
        matcher: None,
        timeout_s: 3,
        engine_ms: 0,
    },
    Ev {
        name: "SubagentStop",
        act: Act::Engine,
        matcher: None,
        timeout_s: 5,
        engine_ms: 2_500,
    },
];

/// The longest a Stop waits for a check that is being saved before it asks the engine: 12 s, and never more than the
/// hook's `timeout_s` less 5 s. Muse fails an overdue hook open (research/59 D7, T6): the engine's answer would be lost.
pub fn save_wait_cap(timeout_s: u32) -> Duration {
    let room = Duration::from_secs(u64::from(timeout_s)).saturating_sub(Duration::from_secs(5));
    Duration::from_secs(12).min(room)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema;

    #[test]
    fn every_registered_event_is_in_the_table_once_and_a_retired_one_is_not() {
        let mut names: Vec<&str> = EVENTS.iter().map(|e| e.name).collect();
        names.sort();
        let mut want = schema::EVENT_NAMES.to_vec();
        want.sort();
        assert_eq!(names, want, "table and schema must name the same 11 events");
        assert_eq!(names.len(), 11);
        for r in RETIRED {
            assert!(!names.contains(r), "{r} is retired and registered");
        }
    }

    #[test]
    fn settings_timeouts_are_whole_seconds_and_outlast_the_engine_kill_time() {
        for e in EVENTS {
            assert!(e.timeout_s >= 1, "{}", e.name);
            if e.act == Act::Engine {
                // the kill time, plus the pipe grace (0.3 s), the state lock (0.5 s) and the transcript step
                assert!(
                    e.engine_ms + 2_000 <= u64::from(e.timeout_s) * 1_000,
                    "{}: Muse would cut us off before our own kill",
                    e.name
                );
            }
        }
        let stop = EVENTS.iter().find(|e| e.name == "Stop").unwrap();
        assert_eq!(stop.timeout_s, 35);
        assert!(
            stop.engine_ms > 30_000,
            "the gate may use 20 s plus its save"
        );
    }

    #[test]
    fn the_stops_wait_for_a_save_is_12_s_and_leaves_5_s_of_the_hooks_timeout() {
        let stop = EVENTS.iter().find(|e| e.name == "Stop").unwrap();
        assert_eq!(save_wait_cap(stop.timeout_s), Duration::from_secs(12));
        assert_eq!(save_wait_cap(10), Duration::from_secs(5));
        assert_eq!(save_wait_cap(17), Duration::from_secs(12));
        assert_eq!(save_wait_cap(3), Duration::ZERO);
    }

    #[test]
    fn matchers_only_where_muse_allows_them() {
        for e in EVENTS {
            let tool = matches!(e.name, "PreToolUse" | "PostToolUse" | "PostToolUseFailure");
            if tool {
                assert_eq!(e.matcher, TOOLS, "{}", e.name);
            } else if e.name == "SessionStart" {
                assert!(e.matcher.unwrap().split('|').any(|m| m == "fork"));
            } else {
                assert!(e.matcher.is_none(), "{}", e.name);
            }
        }
        assert!(EVENTS.iter().all(|e| e.name != "state"));
    }
}

//! A false Done (71c P4): the AI's last message says the tests pass or the build succeeds, and this session's own record
//! says otherwise.
//!
//! - `payload` keeps, from the hooks this adapter gets, the last test run and the last build (their exit code and turn,
//!   the first 80 characters of the command) and the last edit to a file that is not documentation, in the session's
//!   state file (`log`, `Session.done`). What counts as a test run or a build is the brief's list (`commands`).
//! - At a Stop, `verdict` reads the AI's last message for an explicit claim (`claim`) and compares it with the record.
//!   A claim is false when, for its kind: (a) nothing of that kind ran in this session, (b) the last run exited non-zero
//!   and no passing run came after it, (c) the last passing run came before the last edit.
//! - The answer is ONE sentence for the model, added to the first Stop block of the turn (`stop_note`): in the same reason
//!   as the Stop's other items, or as the block on its own. Nothing reaches the model on any other Stop, and nothing when
//!   the engine blocked the Stop, after `torpor off`, or at the Stop that follows a block (the second block of a turn, SN2,
//!   is about a check that still fails and says nothing of the claim).
//!
//! Precision before recall: what cannot be read for sure stays silent. Known blind spots, each with its upgrade (the
//! engine's tool_events after Nov 4: 71c M5 `stop_line`, M6 `exit_code_of` / `post_tool` give the turn's own commands
//! and exit codes, and this module should read those instead of the hooks):
//!
//! shortcut: a run that hides its exit code (`npm test | tail`, `npm test || true`, a background job, a subshell) is
//! unsure, and an unsure run silences the claims of its kind after it; upgrade with the engine's exit codes per command.
//!
//! shortcut: edits that a command makes (`sed -i`, a formatter, `git checkout`) are not seen, so (c) misses them; edits
//! to documents, to files outside the folder and to `.git` and `node_modules` do not count; upgrade with the engine's
//! per-turn file list.
//!
//! shortcut: tests that the user ran outside Muse are not seen; the sentence therefore also lets the model say they were
//! not run. A run of a subset (`npm test -- parser`, `pytest tests/a.py`) counts as the tests; upgrade when a drill shows
//! a false catch from it.
//!
//! shortcut: turns are counted here from the turn ids this module saw, so "3 turns ago" is not `torpor show N`; a state
//! file that cannot be locked within 500 ms, or was reset, loses a record: (a) is only judged for a session that started
//! here (`startup`, `clear`), and a resumed or forked session has no (a); upgrade with the engine's turn numbers.
//!
//! shortcut: only the end-of-turn message is read, in English, and a sentence about production, staging, CI, a Docker
//! image or the docs is skipped; a runner that is neither listed nor named with a test word is not seen as a test run.
//! Only the first false kind is named (tests, then build). A subagent that outlives its parent's Stop is not seen.

mod claim;
mod commands;
mod log;
mod payload;
mod programs;
mod segment;
mod split;
#[cfg(test)]
mod tests;
mod verdict;

pub use log::Log;
pub(super) use payload::{record, said};
pub(super) use verdict::{join, verdict};

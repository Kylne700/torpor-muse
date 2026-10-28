//! What one session has run and edited, kept in its state file (`Session.done`): bounded, a few numbers and two short
//! command lines.

use super::claim::Topic;
use super::commands::Seen;
use serde::{Deserialize, Serialize};

/// A test run or a build whose exit code is known.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub(super) struct Run {
    /// Its place in the order of everything the log saw.
    pub(super) seq: u64,
    /// The turn it ran in, counted by the log; 0 = not known.
    pub(super) turn: u32,
    pub(super) code: i64,
    /// The first 80 characters of the command line, made safe to show.
    pub(super) cmd: String,
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub(super) struct Track {
    pub(super) last: Option<Run>,
    /// The place of the last run of this kind whose exit code is not known (`commands::Seen::Unsure`).
    pub(super) unsure: u64,
}

#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Log {
    seq: u64,
    turn_id: String,
    pub(super) turn: u32,
    /// The log has the whole session: it started here (`startup`, `clear`), so no run in it means no run happened.
    whole: bool,
    test: Track,
    build: Track,
    /// The place of the last edit to a file that is not documentation.
    edit: u64,
}

/// Why a claim is false.
pub(super) enum Why<'a> {
    /// (a) nothing of the kind ran in this session
    NoRun,
    /// (b) the last run exited non-zero and no passing run came after it
    Failed(&'a Run),
    /// (c) the last passing run came before the last edit
    Edited(&'a Run),
}

impl Log {
    /// A turn id seen: a new one is the next turn.
    pub(super) fn see(&mut self, id: &str) {
        if !id.is_empty() && id != self.turn_id {
            self.turn_id = id.to_owned();
            self.turn = self.turn.saturating_add(1);
        }
    }

    pub(super) fn start(&mut self, source: &str) {
        self.whole |= matches!(source, "startup" | "clear");
    }

    pub(super) fn ran(&mut self, tests: Seen, build: Seen, cmd: &str) {
        self.seq += 1;
        let (seq, turn) = (self.seq, self.turn);
        for (track, seen) in [(&mut self.test, tests), (&mut self.build, build)] {
            match seen {
                Seen::No => {}
                Seen::Unsure => track.unsure = seq,
                Seen::Ran(code) => {
                    track.last = Some(Run {
                        seq,
                        turn,
                        code,
                        cmd: cmd.to_owned(),
                    });
                }
            }
        }
    }

    pub(super) fn edited(&mut self) {
        self.seq += 1;
        self.edit = self.seq;
    }

    /// Does the record contradict a claim about `topic`? An unsure run after the evidence silences it: that run may have
    /// changed the answer.
    pub(super) fn why(&self, topic: Topic) -> Option<Why<'_>> {
        let t = match topic {
            Topic::Test => &self.test,
            Topic::Build => &self.build,
        };
        match &t.last {
            None => (self.whole && t.unsure == 0).then_some(Why::NoRun),
            Some(r) if r.code != 0 => (t.unsure < r.seq).then_some(Why::Failed(r)),
            Some(r) => (self.edit > r.seq && self.edit > t.unsure).then_some(Why::Edited(r)),
        }
    }
}

//! The values a run passes around: the kind of outcome, the outcome, the run's context and its plan.

use super::client::{self, Key};
use super::schema::Start;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Kind {
    Proven,
    Advisory,
    Rejected,
    Skipped,
    Error,
}

impl Kind {
    pub(super) fn word(self) -> &'static str {
        match self {
            Kind::Proven => "proven",
            Kind::Advisory => "advisory",
            Kind::Rejected => "rejected",
            Kind::Skipped => "skipped",
            Kind::Error => "error",
        }
    }
}

pub(super) struct Outcome {
    pub(super) kind: Kind,
    pub(super) saved: bool,
    pub(super) promise: Option<u32>,
    pub(super) message: String,
    /// What the hook may tell the user at the next prompt: `key`, `cap`, `api`, `slow` (the watchdog stopped the run),
    /// `rejected` (the model's answer could not be used), `app`, `fails` (the engine: the check fails on the code as it
    /// is), `engine` (the engine timed out or ended oddly), `nosave` (any other end without a saved check: the engine
    /// turned the check down or refused it, every step needed a record already in the data, no check was made; the hook
    /// tells these apart by how the words begin, see `read.rs`) ("" = nothing to tell: no claim, a claim that cannot be
    /// checked, skipped, saved).
    pub(super) code: &'static str,
}

impl Outcome {
    pub(super) fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            kind,
            saved: false,
            promise: None,
            message: message.into(),
            code: "",
        }
    }

    pub(super) fn code(mut self, code: &'static str) -> Self {
        self.code = code;
        self
    }
}

/// What a run knows about itself, for the result file and for keeping the key out of every text it shows.
pub(super) struct Ctx {
    pub(super) started: Instant,
    pub(super) deadline: Instant,
    pub(super) result: Option<PathBuf>,
    pub(super) session: Option<String>,
    pub(super) key: Option<Key>,
    pub(super) model: String,
    pub(super) calls: u32,
    pub(super) usd: f64,
}

impl Ctx {
    pub(super) fn time_left(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Text from the model, the server or the engine, made safe to print and to store.
    pub(super) fn safe(&self, s: &str, max: usize) -> String {
        let t = client::clean(s, max);
        self.key.as_ref().map_or_else(|| t.clone(), |k| k.scrub(&t))
    }
}

pub(super) struct Plan {
    pub(super) root: PathBuf,
    pub(super) ledger_dir: PathBuf,
    pub(super) model: String,
    pub(super) key: Key,
    pub(super) base: client::Base,
    pub(super) cap: f64,
    pub(super) starts: Vec<Start>,
    pub(super) excerpt: String,
    pub(super) words: String,
    pub(super) http: Duration,
}

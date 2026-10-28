//! `suggest "<what works>"`: Muse Spark (Responses API, text.format json_schema) writes a check from the user's words;
//! the engine proves it or keeps it advisory. `trigger` starts a detached run from UserPromptSubmit. Owner: S3b-writer.
//!
//! Exit codes of `run`: 0 a promise was saved (proven or advisory), 3 nothing saved (the reason is printed),
//! 1 the run could not work (no key, no engine, no state folder, another run holds the lock), 2 wrong use.
//! Every run that got past the lock leaves `<state>/ws/<project>/result.json` (written whole, mode 0600). Its `code`
//! (key, cap, api, slow, rejected, app; empty for the rest) names the failures hook.rs tells the user at the next
//! prompt.
mod attach;
mod client;
mod finish;
mod plan;
mod prompt;
mod read;
mod repo;
mod schema;
mod state;
mod trigger;
mod types;
mod voice;
mod work;

pub use read::{gaps, last_result, scrub};
pub use state::Gap;
pub use trigger::{trigger, writer_off};
pub use voice::say;

use finish::{finish, watchdog};
use plan::plan;
use state::env;
use std::time::{Duration, Instant};
use types::Ctx;
use work::work;

const DEFAULT_MODEL: &str = "muse-spark-1.3";
const HARD_TIMEOUT: Duration = Duration::from_secs(720);
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);

fn usage() -> u8 {
    eprintln!("usage: torpor-muse suggest \"<what works, in your words>\"");
    2
}

fn bail(msg: &str) -> u8 {
    eprintln!("torpor-muse suggest: {msg}");
    1
}

fn seconds(name: &str) -> Option<Duration> {
    env(name)?
        .parse::<u64>()
        .ok()
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
}

pub fn run(words: &str) -> u8 {
    let started = Instant::now();
    let hook = env("TORPOR_MUSE_FROM_HOOK");
    let words = match (words.trim(), &hook) {
        ("-", Some(_)) => env("TORPOR_MUSE_WORDS").unwrap_or_default(),
        ("-", None) => String::new(),
        (w, _) => w.to_string(),
    };
    if words.trim().is_empty() {
        return usage();
    }
    let base = match client::api_base() {
        Ok(b) => b,
        Err(e) => return bail(&e),
    };
    // The short test limits count only against a loopback address; a real run cannot be given them by accident.
    let (http, hard) = if base.loopback {
        (
            seconds("TORPOR_MUSE_HTTP_TIMEOUT_S").unwrap_or(HTTP_TIMEOUT),
            seconds("TORPOR_MUSE_HARD_TIMEOUT_S").unwrap_or(HARD_TIMEOUT),
        )
    } else {
        (HTTP_TIMEOUT, HARD_TIMEOUT)
    };
    let root = match repo::resolve_root(None) {
        Ok(r) => r,
        Err(e) => return bail(&e),
    };
    let ws = match state::ws_dir(&root) {
        Ok(d) => d,
        Err(e) => return bail(&e),
    };
    let _lock = match state::try_lock(&ws) {
        Ok(Some(l)) => l,
        Ok(None) => return bail("another run is already working on this folder"),
        Err(e) => return bail(&e),
    };
    let mut c = Ctx {
        started,
        deadline: started + hard,
        result: Some(ws.join("result.json")),
        session: hook.clone(),
        key: None,
        model: DEFAULT_MODEL.into(),
        calls: 0,
        usd: 0.0,
    };
    watchdog(&c);
    let outcome = match plan(&mut c, &root, &words, hook.is_some(), base, http) {
        Ok(p) => work(&mut c, &p),
        Err(o) => o,
    };
    finish(&c, outcome)
}

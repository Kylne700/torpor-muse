//! The end of a run: the watchdog that stops a stuck run and the final write with the exit code.

use super::types::{Ctx, Kind, Outcome};
use super::{attach, state};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const WATCHDOG_GRACE: Duration = Duration::from_secs(3);

/// Set by whoever writes the final result first (the run, or the watchdog); the other then stands aside.
static DONE: AtomicBool = AtomicBool::new(false);

/// Ends the run if it takes too long: stops the engine call, writes the result, exits.
pub(super) fn watchdog(c: &Ctx) {
    let (deadline, result, session) = (c.deadline, c.result.clone(), c.session.clone());
    std::thread::spawn(move || {
        // A call in flight times out at the deadline itself and is written to the ledger; the watchdog is for what
        // does not (a stuck engine, a stuck write), so it waits a little longer.
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()) + WATCHDOG_GRACE);
        if DONE.swap(true, Ordering::SeqCst) {
            return;
        }
        let left = attach::kill_running();
        let message = format!(
            "the run took too long and was stopped{}",
            left.map(|why| format!("; {why}")).unwrap_or_default()
        );
        if let Some(p) = result {
            let row = json!({"v": 1, "at": crate::util::iso(crate::util::now_secs()), "outcome": Kind::Advisory.word(),
                "saved": false, "promise": null, "message": message, "code": "slow", "session": session});
            let _ = state::write_atomic(&p, row.to_string().as_bytes());
        }
        println!("{}: {message}", Kind::Advisory.word());
        std::process::exit(3);
    });
}

pub(super) fn finish(c: &Ctx, o: Outcome) -> u8 {
    if DONE.swap(true, Ordering::SeqCst) {
        // the watchdog is ending the process
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    let message = c.safe(&o.message, 1500);
    if let Some(p) = &c.result {
        let row = json!({
            "v": 1, "at": crate::util::iso(crate::util::now_secs()), "outcome": o.kind.word(), "saved": o.saved,
            "promise": o.promise, "message": message, "code": o.code, "session": c.session, "model": c.model, "calls": c.calls,
            "usd": (c.usd * 1e6).round() / 1e6, "ms": u64::try_from(c.started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
        if let Err(e) = state::write_atomic(p, row.to_string().as_bytes()) {
            eprintln!("torpor-muse suggest: {e}");
        }
    }
    if o.kind == Kind::Error {
        eprintln!("torpor-muse suggest: {message}");
    } else {
        println!("{}: {message}", o.kind.word());
    }
    match (o.saved, o.kind) {
        (true, _) => 0,
        (false, Kind::Error) => 1,
        (false, _) => 3,
    }
}

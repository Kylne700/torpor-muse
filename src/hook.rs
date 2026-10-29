//! `torpor-muse hook <Event>`: reads the Muse Code hook payload on stdin, normalises it, calls `torpor hook <Event>`,
//! writes Muse's closed output schema (research/59 D5). Owner: S3a-core.
//!
//! Policy: a hook never fails a Muse turn. Whatever goes wrong (bad payload, engine missing or of the wrong version,
//! timeout, output Muse would reject) is written to `<TORPOR_HOME>/muse/problems.log`, the hook exits 0, and the user
//! is told once per session that Torpor is not saving. The only way a hook stops a turn is a Stop block in the JSON
//! form with a reason (never exit 2).

mod busy;
mod compact_note;
mod db_note;
mod done;
mod events;
mod gate;
mod handle;
mod normalise;
mod outside;
mod paths;
mod state;
mod stop;
mod stop_note;
mod writer_note;

pub(crate) use events::{EVENTS, Ev, RETIRED};

use crate::{engine, suggest};
use handle::handle;
use std::io::{Read, Write};
use std::path::Path;

/// What a hook prints, and what it starts after printing.
#[derive(Default)]
struct Done {
    stdout: Option<String>,
    /// (session id, cwd, prompt) for `suggest::trigger`: a main-session UserPromptSubmit the engine took.
    prompt: Option<(String, String, String)>,
}

/// With no TORPOR_HOME and a HOME that is unset, empty or not a full path, the state folder would be `.torpor/` in
/// whatever folder the hook runs in (the user's project).
fn home_usable() -> bool {
    let set = |k| std::env::var_os(k).filter(|v| !v.is_empty());
    set("TORPOR_HOME").is_some() || set("HOME").is_some_and(|h| Path::new(&h).is_absolute())
}

pub fn run(event: &str) -> u8 {
    let mut input = Vec::new();
    let read = std::io::stdin().lock().read_to_end(&mut input);
    let done = if !home_usable() {
        eprintln!(
            "torpor-muse: hook {}: TORPOR_HOME is not set and HOME is not a full path; Torpor is not saving this project",
            event.escape_default()
        );
        Done::default()
    } else {
        let dir = engine::state_dir();
        match read {
            Ok(_) => handle(&dir, event, &input),
            Err(e) => {
                state::problem(&dir, event, "", &format!("cannot read stdin: {e}"));
                Done::default()
            }
        }
    };
    if let Some(text) = &done.stdout {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{text}");
        let _ = out.flush();
    }
    if let Some((sid, cwd, prompt)) = done.prompt {
        // Muse waits for its pipe to close, and anything started now would inherit it: close ours first.
        close_stdio();
        if let Some(kind) = suggest::trigger(&sid, &cwd, &prompt) {
            state::with_session(&engine::state_dir(), &sid, |s| s.writer_skip = kind.into());
        }
    }
    0
}

/// Points stdout and stderr at /dev/null, so nothing we start later can hold Muse's pipes open.
fn close_stdio() {
    // SAFETY: open(2) and dup2(2) on descriptors this process owns; the output was flushed before.
    unsafe {
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null >= 0 {
            libc::dup2(null, 1);
            libc::dup2(null, 2);
            if null > 2 {
                libc::close(null);
            }
        }
    }
}

//! `torpor-muse hook <Event>`: reads the Muse Code hook payload on stdin, normalises it, calls `torpor hook <Event>`,
//! writes Muse's closed output schema (research/59 D5). Owner: S3a-core.

pub fn run(event: &str) -> u8 {
    eprintln!("torpor-muse hook {event}: not implemented yet");
    0 // fail open until implemented
}

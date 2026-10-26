//! `suggest "<what works>"`: Muse Spark (Responses API, text.format json_schema) writes a check from the user's words;
//! the engine proves it or keeps it advisory. `trigger` starts a detached run from UserPromptSubmit. Owner: S3b-writer.
#![allow(dead_code)] // skeleton: remove when implemented

pub fn run(_words: &str) -> u8 {
    eprintln!("torpor-muse suggest: not implemented yet");
    1
}

pub fn trigger(_prompt: &str) {}

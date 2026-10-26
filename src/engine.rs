//! Finds the released `torpor` CLI (env TORPOR_BIN, then PATH), checks it against engine.lock, runs it with a hard
//! timeout. Never passes a key on argv. Owner: S3a-core.
#![allow(dead_code)] // skeleton: remove when implemented

pub struct Output {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub fn run(
    _args: &[&str],
    _stdin: Option<&[u8]>,
    _timeout: std::time::Duration,
) -> Result<Output, String> {
    Err("engine::run not implemented yet".into())
}

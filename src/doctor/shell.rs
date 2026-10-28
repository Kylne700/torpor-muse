//! Runs a command the way Muse runs a hook: cleared environment, payload on stdin, own process group, killed at a deadline.

use crate::util::spawn_retry;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Muse keeps at most this much of a hook's stdout or stderr (it fails the hook beyond it); so does the probe.
const PIPE_CAP: usize = 16 * 1024;

pub(crate) struct Ran {
    pub(crate) code: Option<i32>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) timed_out: bool,
}

fn drain(mut r: impl Read + Send + 'static) -> (Arc<Mutex<Vec<u8>>>, std::thread::JoinHandle<()>) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let b = buf.clone();
    let h = std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = r.read(&mut chunk)
            && n > 0
        {
            if let Ok(mut v) = b.lock() {
                let room = PIPE_CAP.saturating_sub(v.len());
                v.extend_from_slice(&chunk[..n.min(room)]);
            }
        }
    });
    (buf, h)
}

/// Runs `shell -c command` the way Muse runs a hook: cleared environment, payload on stdin, own process group.
pub(crate) fn run_shell(
    shell: &str,
    command: &str,
    env: &[(String, String)],
    cwd: &Path,
    stdin: &[u8],
    timeout: Duration,
) -> Result<Ran, String> {
    let mut cmd = Command::new(shell);
    cmd.arg("-c")
        .arg(command)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = spawn_retry(&mut cmd).map_err(|e| format!("cannot start {shell}: {e}"))?;
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(stdin); // small; a hook that does not read it is fine
    }
    let (out, h1) = drain(child.stdout.take().ok_or("no stdout")?);
    let (err, h2) = drain(child.stderr.take().ok_or("no stderr")?);
    let t0 = Instant::now();
    let (code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(s)) => break (s.code(), false),
            Ok(None) if t0.elapsed() > timeout => {
                // The only thing doctor ever kills: the process group of the probe it started itself.
                let _ = Command::new("kill")
                    .args(["-s", "KILL", "--", &format!("-{}", child.id())])
                    .status();
                let _ = child.kill();
                let _ = child.wait();
                break (None, true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("wait: {e}")),
        }
    };
    // A background process that kept the pipe open must not hold doctor: take what arrived within a moment.
    let t1 = Instant::now();
    while !(h1.is_finished() && h2.is_finished()) && t1.elapsed() < Duration::from_millis(500) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let take = |b: &Arc<Mutex<Vec<u8>>>| b.lock().map(|v| v.clone()).unwrap_or_default();
    Ok(Ran {
        code,
        stdout: take(&out),
        stderr: take(&err),
        timed_out,
    })
}

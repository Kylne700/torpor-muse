//! The engine side: ask it for its status, hand it a check on `torpor promise add -`, and read what it answers.
//! Private on purpose: `engine.rs` has no working folder and cannot stop a whole process group. Owner: S3b-writer.
//! shortcut: swap `exec` for `engine::run` once that has a working folder and a group kill.
use super::state::env;
use serde_json::Value;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const OUT_CAP: usize = 64 * 1024;
const NAP: Duration = Duration::from_millis(20);

/// The process group of the engine call that is running now (0 = none), for the watchdog.
static GROUP: AtomicI32 = AtomicI32::new(0);

pub struct Run {
    pub code: Option<i32>,
    pub out: String,
    pub err: String,
    pub timed_out: bool,
}

struct Pipe {
    buf: Arc<Mutex<Vec<u8>>>,
    done: Arc<AtomicBool>,
}

fn drain<R: Read + Send + 'static>(r: Option<R>) -> Pipe {
    let p = Pipe {
        buf: Arc::default(),
        done: Arc::default(),
    };
    let (buf, done) = (Arc::clone(&p.buf), Arc::clone(&p.done));
    match r {
        None => done.store(true, Ordering::SeqCst),
        Some(mut r) => {
            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                while let Ok(n) = r.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    if let Ok(mut g) = buf.lock() {
                        let room = OUT_CAP.saturating_sub(g.len());
                        g.extend_from_slice(&chunk[..n.min(room)]);
                    }
                }
                done.store(true, Ordering::SeqCst);
            });
        }
    }
    p
}

impl Pipe {
    /// What was read so far. A grandchild that keeps the pipe open must not hold us, so the wait is short.
    fn text(&self) -> String {
        let until = Instant::now() + Duration::from_millis(1500);
        while !self.done.load(Ordering::SeqCst) && Instant::now() < until {
            std::thread::sleep(NAP);
        }
        let g = self.buf.lock().map(|g| g.clone()).unwrap_or_default();
        String::from_utf8_lossy(&g).into_owned()
    }
}

fn signal_group(pgid: i32, sig: i32) {
    if pgid > 1 {
        // SAFETY: a plain signal to a process group this process created and has not yet reaped.
        unsafe {
            libc::kill(-pgid, sig);
        }
    }
}

/// For the watchdog: stop the engine call that is running now, with everything it started.
pub fn kill_running() {
    signal_group(GROUP.load(Ordering::SeqCst), libc::SIGKILL);
}

fn binary() -> String {
    env("TORPOR_BIN").unwrap_or_else(|| "torpor".into())
}

pub fn exec(
    root: &Path,
    args: &[&str],
    stdin: Option<&str>,
    limit: Duration,
) -> Result<Run, String> {
    exec_with(&binary(), root, args, stdin, limit)
}

/// Runs `bin` in `root`, in its own process group, without the API key or the marker that says "inside Claude".
/// A call that passes `limit` is stopped with its whole group.
fn exec_with(
    bin: &str,
    root: &Path,
    args: &[&str],
    stdin: Option<&str>,
    limit: Duration,
) -> Result<Run, String> {
    let mut child = Command::new(bin)
        .args(args)
        .current_dir(root)
        .env_remove("META_API_KEY")
        .env_remove("CLAUDECODE")
        .env_remove("TORPOR_MUSE_WORDS")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot start the Torpor engine `{bin}` ({e}): set TORPOR_BIN or put `torpor` on the PATH"))?;
    let pgid = i32::try_from(child.id()).unwrap_or(0);
    GROUP.store(pgid, Ordering::SeqCst);
    if let (Some(text), Some(mut si)) = (stdin, child.stdin.take()) {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = si.write_all(text.as_bytes());
        });
    }
    let (out, err) = (drain(child.stdout.take()), drain(child.stderr.take()));
    let begun = Instant::now();
    let (code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(s)) => break (s.code(), false),
            Ok(None) if begun.elapsed() >= limit => {
                signal_group(pgid, libc::SIGTERM);
                let grace = Instant::now();
                while matches!(child.try_wait(), Ok(None))
                    && grace.elapsed() < Duration::from_secs(3)
                {
                    std::thread::sleep(NAP);
                }
                if matches!(child.try_wait(), Ok(None)) {
                    signal_group(pgid, libc::SIGKILL);
                    let _ = child.wait();
                }
                break (None, true);
            }
            Ok(None) => std::thread::sleep(NAP),
            Err(e) => {
                GROUP.store(0, Ordering::SeqCst);
                return Err(format!("cannot wait for the Torpor engine: {e}"));
            }
        }
    };
    GROUP.store(0, Ordering::SeqCst);
    Ok(Run {
        code,
        out: out.text(),
        err: err.text(),
        timed_out,
    })
}

const SHORT: Duration = Duration::from_secs(20);

/// Is there an engine at all? Asked before any money is spent.
pub fn present(root: &Path) -> Result<(), String> {
    match exec(root, &["version"], None, SHORT)? {
        r if r.code == Some(0) => Ok(()),
        r => Err(format!(
            "the Torpor engine did not answer `version` (exit {:?})",
            r.code
        )),
    }
}

/// What the engine knows about this turn, read before a model call from the hook path.
pub struct Turn {
    pub n: u64,
}

/// The turn of the prompt that started this run, when Torpor is saving here. `Err` is the reason to do nothing.
pub fn hook_turn(root: &Path) -> Result<Turn, String> {
    let r = exec(root, &["status", "--json"], None, SHORT)?;
    if r.code != Some(0) {
        return Err("the Torpor engine could not tell its status".into());
    }
    let v: Value = serde_json::from_str(r.out.trim())
        .map_err(|_| "the Torpor status is not JSON".to_string())?;
    if v["off"].as_bool() == Some(true) || v["saving"].as_bool() != Some(true) {
        return Err("Torpor is not saving in this folder".into());
    }
    let n = v["last_turn"]["n"]
        .as_u64()
        .ok_or("Torpor has no turn to attach a check to")?;
    Ok(Turn { n })
}

/// Has a promise from the user's words at this turn been saved already (by the assistant, on the same praise)?
pub fn promised_at(root: &Path, turn: u64) -> bool {
    let Ok(r) = exec(root, &["promise", "list"], None, SHORT) else {
        return false;
    };
    let tail = format!("your words, turn {turn}");
    r.out.lines().any(|l| l.trim_end().ends_with(&tail))
}

/// What `promise add` said.
#[derive(Debug, PartialEq)]
pub enum Added {
    /// Saved with the engine's strongest proof (it passed on the good version and failed on the bad one).
    Proven { id: Option<u32>, line: String },
    /// Saved, but the engine keeps it advisory (flaky, a guard only, or a reason the line gives).
    Advisory { id: Option<u32>, line: String },
    /// The engine's lint turned the check down (exit 3): its own words.
    Lint(String),
    /// Not saved. `retry`: the check itself is at fault (it could not run, or it fails on the good version).
    NotSaved { why: String, retry: bool },
    /// Anything else (a timeout, an unexpected exit).
    Failed(String),
}

/// Text of the engine, safe to show and to quote back to the model.
fn tidy(s: &str) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    flat.trim().chars().take(1500).collect()
}

fn promise_id(line: &str) -> Option<u32> {
    let rest = line.split("Promise #").nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

pub fn classify(r: &Run) -> Added {
    if r.timed_out {
        return Added::Failed("the Torpor engine did not finish in time and was stopped".into());
    }
    match r.code {
        Some(0) => {
            let line = tidy(
                r.out
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or(""),
            );
            let id = promise_id(&line);
            if line.contains("In shadow for now") {
                Added::Proven { id, line }
            } else {
                Added::Advisory { id, line }
            }
        }
        Some(3) => Added::Lint(tidy(&r.err)),
        Some(1) if r.err.contains("Not saved: ") => {
            let why = tidy(r.err.split("Not saved: ").nth(1).unwrap_or(""));
            let retry =
                why.starts_with("the check could not run") || why.starts_with("it fails on");
            Added::NotSaved { why, retry }
        }
        code => Added::Failed(format!(
            "the Torpor engine ended with exit {code:?}: {}",
            tidy(&r.err)
        )),
    }
}

/// Hands the check to the engine, which lints it, qualifies it on the saved versions, and saves it.
pub fn add(root: &Path, check: &Value, limit: Duration) -> Result<Added, String> {
    let r = exec(
        root,
        &["promise", "add", "-"],
        Some(&check.to_string()),
        limit,
    )?;
    Ok(classify(&r))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn run(code: Option<i32>, out: &str, err: &str) -> Run {
        Run {
            code,
            out: out.into(),
            err: err.into(),
            timed_out: false,
        }
    }

    #[test]
    fn the_engines_answers_are_told_apart() {
        let proven = "progress\nPromise #3 \"login works\" saved. Proof: passed 5 of 5 on A and failed 2 of 2 on B. In shadow for now: x";
        assert_eq!(
            classify(&run(Some(0), proven, "")),
            Added::Proven {
                id: Some(3),
                line: proven.lines().last().unwrap().into()
            }
        );
        let adv = "Promise #4 \"x\" saved. Proof: passed 5 of 5 on A and failed 2 of 2 on B. Advisory for now: Torpor shows failures but does not stop the AI, because praise written only in Hebrew stays advisory.";
        assert!(matches!(
            classify(&run(Some(0), adv, "")),
            Added::Advisory { id: Some(4), .. }
        ));
        let flaky = "Promise #5 \"x\" saved but flaky: it failed 1 of 5 runs on A.";
        assert!(matches!(
            classify(&run(Some(0), flaky, "")),
            Added::Advisory { .. }
        ));
        assert!(matches!(
            classify(&run(Some(3), "", "Rejected: step 1 uses an absolute URL\n")),
            Added::Lint(m) if m.contains("absolute URL")
        ));
        let nope = classify(&run(
            Some(1),
            "",
            "Not saved: the check could not run on A: no answer\n",
        ));
        assert_eq!(
            nope,
            Added::NotSaved {
                why: "the check could not run on A: no answer".into(),
                retry: true
            }
        );
        let late = classify(&run(
            Some(1),
            "",
            "Not saved: the checks took longer than 9 minutes.\n",
        ));
        assert!(matches!(late, Added::NotSaved { retry: false, .. }));
        assert!(matches!(
            classify(&run(Some(2), "", "usage")),
            Added::Failed(_)
        ));
        let mut t = run(None, "", "");
        t.timed_out = true;
        assert!(matches!(classify(&t), Added::Failed(m) if m.contains("in time")));
    }

    fn script(dir: &Path, body: &str) -> String {
        let p = dir.join("fake-torpor");
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.to_string_lossy().into_owned()
    }

    // `ps` exists on Linux and macOS (no /proc on macOS); a zombie counts as dead.
    fn alive(pid: &str) -> bool {
        let out = std::process::Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        !s.is_empty() && !s.starts_with('Z')
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("tm-attach-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_engine_gets_its_input_and_not_the_key() {
        let d = tmp("env");
        let bin = script(
            &d,
            "cat; echo \"key=${META_API_KEY:-none} claude=${CLAUDECODE:-none} dir=$(pwd)\" 1>&2",
        );
        let _g = super::super::state::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // SAFETY: every test that touches the environment holds ENV_LOCK.
        unsafe {
            std::env::set_var("META_API_KEY", "not-a-real-key");
            std::env::set_var("CLAUDECODE", "1");
        }
        let r = exec_with(&bin, &d, &["x"], Some("hello"), Duration::from_secs(10)).unwrap();
        unsafe {
            std::env::remove_var("META_API_KEY");
            std::env::remove_var("CLAUDECODE");
        }
        assert_eq!(r.out, "hello");
        assert!(r.err.contains("key=none claude=none"), "{}", r.err);
        assert!(
            r.err.contains("tm-attach-env"),
            "runs in the given folder: {}",
            r.err
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_stuck_engine_is_stopped_with_its_children() {
        let d = tmp("hang");
        let bin = script(&d, "sleep 30 & echo $! > pidfile; wait");
        let t = Instant::now();
        let r = exec_with(&bin, &d, &[], None, Duration::from_millis(1500)).unwrap();
        assert!(
            r.timed_out && t.elapsed() < Duration::from_secs(8),
            "{:?}",
            t.elapsed()
        );
        let pid = std::fs::read_to_string(d.join("pidfile"))
            .unwrap_or_default()
            .trim()
            .to_string();
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !pid.is_empty() && !alive(&pid),
            "the child of the engine must be gone (pid {pid})"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_engine_says_how_to_fix_it() {
        let e = exec_with(
            "/nonexistent/torpor",
            Path::new("/tmp"),
            &[],
            None,
            Duration::from_secs(1),
        )
        .err()
        .unwrap();
        assert!(e.contains("TORPOR_BIN"), "{e}");
    }
}

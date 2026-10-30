//! Finds the released `torpor` CLI (env TORPOR_BIN, then PATH), checks it against engine.lock, runs it with a hard
//! timeout. The callers hand over fixed words only (`hook <Event>`, `version`, the read-only
//! looks of src/hook/handle.rs, stop_note.rs and gate.rs, and `gate off` once per folder in gate.rs). Owner:
//! S3a-core.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

pub struct Output {
    /// The binary that ran (as `locate` found it).
    pub bin: PathBuf,
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

const LOCK: &str = include_str!("../engine.lock");
/// After the engine is gone a grandchild may still hold its pipes: wait this long for them, then keep what we have.
const PIPE_GRACE: Duration = Duration::from_millis(300);
/// Output we keep per pipe; the rest is read and dropped, so a full pipe does not block the engine.
const KEEP: usize = 8 << 20;

/// The `version` line of engine.lock: the one place the engine version is written down.
pub fn pinned_version() -> &'static str {
    LOCK.lines()
        .filter_map(|l| l.strip_prefix("version "))
        .map(str::trim)
        .next()
        .unwrap_or("")
}

/// The Torpor store home as the engine resolves it: TORPOR_HOME, else ~/.torpor.
pub fn home_dir() -> PathBuf {
    match std::env::var_os("TORPOR_HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h),
        _ => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".torpor"),
    }
}

/// Our own files live in `<home>/muse/`; the engine skips entries that are not 16/20-hex store directories.
pub fn state_dir() -> PathBuf {
    home_dir().join("muse")
}

/// Writes `path` (mode 0600, its folder made 0700) through a temp file and a rename, so a reader gets a whole
/// file. Not fsynced: this is for state a hook can lose.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        crate::util::private_dir(dir)?;
    }
    crate::util::write_atomic(path, bytes, 0o600, false, || Ok(()))
}

fn is_exec(p: &Path) -> bool {
    fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// The engine binary: env TORPOR_BIN, else the first `torpor` on PATH.
pub fn locate() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("TORPOR_BIN").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(p));
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|d| d.join("torpor"))
        .find(|c| is_exec(c))
        .ok_or_else(|| "torpor not found: set TORPOR_BIN or put it on PATH".to_string())
}

/// The cache file of `verify`, in `state_dir()`; `uninstall` removes it.
pub const ENGINE_OK: &str = "engine-ok";

/// Ok when `bin` prints `torpor <version line of engine.lock>`. The answer is cached in `<state>/engine-ok` by path,
/// size and mtime, so a hook normally spends one stat and one small read on it.
pub fn verify(bin: &Path) -> Result<(), String> {
    let want = format!("torpor {}", pinned_version());
    if pinned_version().is_empty() {
        return Err("engine.lock has no version line".into());
    }
    let meta = fs::metadata(bin).map_err(|e| format!("torpor engine {}: {e}", bin.display()))?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    let stamp = format!("{}\n{}\n{mtime}\n{want}\n", bin.display(), meta.len());
    let cache = state_dir().join(ENGINE_OK);
    if fs::read_to_string(&cache).is_ok_and(|s| s == stamp) {
        return Ok(());
    }
    let out = exec(bin, &["version"], None, None, Duration::from_secs(5))?;
    let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.code != 0 || got != want {
        return Err(format!(
            "torpor engine {} prints {got:?}, engine.lock pins {want:?}",
            bin.display()
        ));
    }
    let _ = write_private(&cache, stamp.as_bytes()); // a cache that cannot be written only costs one more `version`
    Ok(())
}

/// Runs `torpor <args>` (after `locate` and `verify`) with `stdin` and a hard `timeout`. Err = it could not run:
/// missing, wrong version, spawn failure, or killed at the timeout (with what it started: see `killtree`).
pub fn run(args: &[&str], stdin: Option<&[u8]>, timeout: Duration) -> Result<Output, String> {
    let bin = locate()?;
    verify(&bin)?;
    exec(&bin, args, stdin, None, timeout)
}

/// One pipe read to the end in the background; the bytes collected so far stay readable even if it does not end.
struct Drain {
    buf: Arc<Mutex<Vec<u8>>>,
    thread: thread::JoinHandle<()>,
}

impl Drain {
    fn start(pipe: Option<impl Read + Send + 'static>) -> Drain {
        let buf = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&buf);
        let thread = thread::spawn(move || {
            let Some(mut pipe) = pipe else { return };
            let mut chunk = [0u8; 16 << 10];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let mut b = shared.lock().unwrap_or_else(|e| e.into_inner());
                let room = KEEP.saturating_sub(b.len()).min(n);
                b.extend_from_slice(&chunk[..room]);
            }
        });
        Drain { buf, thread }
    }

    fn finish(self, until: Instant) -> Vec<u8> {
        while !self.thread.is_finished() && Instant::now() < until {
            thread::sleep(Duration::from_millis(2));
        }
        let mut b = self.buf.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *b)
    }
}

/// Spawns `bin args` (in folder `dir`, if given) in its own process group, feeds `stdin`, and waits at most `timeout`.
pub(crate) fn exec(
    bin: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    dir: Option<&Path>,
    timeout: Duration,
) -> Result<Output, String> {
    exec_without(bin, args, stdin, dir, timeout, &[])
}

/// `exec` with the variables in `unset` taken out of the child's environment. Its one caller (`hook::gate::gate_default`)
/// drops CLAUDECODE: the engine refuses the user's own commands, `gate off` among them, where it is set (cli.rs).
pub(crate) fn exec_without(
    bin: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    dir: Option<&Path>,
    timeout: Duration,
    unset: &[&str],
) -> Result<Output, String> {
    let mut cmd = Command::new(bin);
    for k in unset {
        cmd.env_remove(k);
    }
    if let Some(d) = dir {
        cmd.current_dir(d);
    }
    cmd.args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0); // own group, so that a timeout can stop it and what it started (`killtree`)
    let mut child = crate::util::spawn_retry(&mut cmd)
        .map_err(|e| format!("cannot run {}: {e}", bin.display()))?;
    let pgid = child.id() as libc::pid_t;
    if let (Some(mut pipe), Some(data)) = (child.stdin.take(), stdin) {
        let data = data.to_vec();
        thread::spawn(move || {
            let _ = pipe.write_all(&data); // a broken pipe just means the engine did not want the rest
        });
    }
    let out = Drain::start(child.stdout.take());
    let err = Drain::start(child.stderr.take());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(child.wait());
    });
    let status = rx.recv_timeout(timeout);
    let timed_out = status.is_err();
    let mut left = None;
    if timed_out {
        left = crate::killtree::kill(pgid); // the child is not reaped yet, so its group id cannot have been reused
        let _ = rx.recv();
    }
    let until = Instant::now() + PIPE_GRACE;
    let (stdout, stderr) = (out.finish(until), err.finish(until));
    if timed_out {
        return Err(format!(
            "{} {} timed out after {} ms and was killed{}",
            bin.display(),
            args.join(" "),
            timeout.as_millis(),
            left.map(|why| format!("; {why}")).unwrap_or_default()
        ));
    }
    let code = match status {
        Ok(Ok(s)) => s.code().unwrap_or(-1),
        _ => -1,
    };
    Ok(Output {
        bin: bin.to_path_buf(),
        code,
        stdout,
        stderr,
    })
}

#[cfg(test)]
pub mod testing {
    //! Shared by the unit tests of this crate: scratch directories and fake engines.
    use super::*;

    pub fn scratch(name: &str) -> PathBuf {
        let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/tm-test")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    pub fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    /// True while `pid` is a live process (a zombie counts as gone).
    pub fn alive(pid: &str) -> bool {
        let o = Command::new("ps")
            .args(["-o", "stat=", "-p", pid])
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&o.stdout);
        !s.trim().is_empty() && !s.trim().starts_with('Z')
    }

    /// A fake engine that starts `sleep 30` as its child, the way `how` says (perl code that runs before the exec), writes
    /// the sleeper's pid to the file named by its first argument once that is done, and waits. `how` moves the sleeper out
    /// of the engine's group (`setpgrp(0,0);`) or session (`use POSIX; POSIX::setsid();`).
    pub fn sleeper_engine(dir: &Path, how: &str) -> PathBuf {
        let body = format!(
            "perl -e '{how} open F, \">$ARGV[0]\" or die; print F $$; close F; exec \"sleep\", \"30\"' \"$1\" &\nwait"
        );
        script(dir, "slow", &body)
    }

    /// The sleeper of a `sleeper_engine`, by pid. Dropping it kills that exact pid, and only while it still runs
    /// `sleep 30`, so a failing test leaves nothing behind.
    pub struct Sleeper(pub String);

    impl Drop for Sleeper {
        fn drop(&mut self) {
            let ps = Command::new("ps")
                .args(["-o", "args=", "-p", &self.0])
                .output();
            let ours = ps.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "sleep 30");
            if let (true, Ok(pid)) = (ours, self.0.parse::<i32>()) {
                // SAFETY: SIGKILL to the one pid a test started, read again as its `sleep 30` just now.
                unsafe { libc::kill(pid, libc::SIGKILL) };
            }
        }
    }

    /// The sleeper whose pid the fake engine wrote to `pidfile`, waiting at most `secs` seconds for it.
    pub fn sleeper(pidfile: &Path, secs: u64) -> Option<Sleeper> {
        for _ in 0..=secs * 50 {
            let text = fs::read_to_string(pidfile).unwrap_or_default();
            if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) {
                return Some(Sleeper(text));
            }
            thread::sleep(Duration::from_millis(20));
        }
        None
    }

    /// Waits up to two seconds for `pid` to be gone.
    pub fn gone(pid: &str) -> bool {
        for _ in 0..100 {
            if !alive(pid) {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        !alive(pid)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn pinned_version_comes_from_the_lock() {
        assert!(!pinned_version().is_empty());
        assert!(LOCK.contains(&format!("\nversion {}\n", pinned_version())));
    }

    #[test]
    fn timeout_kills_the_engine_and_what_it_started() {
        let d = scratch("kill");
        let pidfile = d.join("pid");
        let s = script(&d, "slow", "sleep 30 &\necho $! > \"$1\"\nwait");
        let t0 = Instant::now();
        let r = exec(
            &s,
            &[pidfile.to_str().unwrap()],
            None,
            None,
            Duration::from_millis(600),
        );
        assert!(r.is_err_and(|e| e.contains("timed out")));
        assert!(t0.elapsed() < Duration::from_secs(5));
        let pid = fs::read_to_string(&pidfile).unwrap().trim().to_string();
        for _ in 0..100 {
            if !alive(&pid) {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!alive(&pid), "the grandchild survived the timeout");
        let _ = fs::remove_dir_all(&d);
    }

    // The engine starts the app under test in a process group of its own, so killing the engine's group is not enough.
    #[test]
    fn a_child_in_a_group_of_its_own_dies_at_the_timeout_too() {
        let d = scratch("own-group");
        let pidfile = d.join("pid");
        let s = sleeper_engine(&d, "setpgrp(0,0);");
        let r = exec(
            &s,
            &[pidfile.to_str().unwrap()],
            None,
            None,
            Duration::from_secs(3),
        );
        let kid = sleeper(&pidfile, 2).expect("the fake engine never started its child");
        assert!(r.is_err_and(|e| e.contains("timed out")));
        assert!(
            gone(&kid.0),
            "a child in its own group survived the timeout"
        );
        let _ = fs::remove_dir_all(&d);
    }

    // The engine starts `torpor gc --if-quiet` with setsid on purpose, so that it outlives the hook: it must not be killed.
    #[test]
    fn a_child_that_started_its_own_session_survives_the_timeout() {
        let d = scratch("own-session");
        let pidfile = d.join("pid");
        let s = sleeper_engine(&d, "use POSIX; POSIX::setsid();");
        let r = exec(
            &s,
            &[pidfile.to_str().unwrap()],
            None,
            None,
            Duration::from_secs(3),
        );
        let kid = sleeper(&pidfile, 2).expect("the fake engine never started its child");
        assert!(r.is_err_and(|e| e.contains("timed out")));
        thread::sleep(Duration::from_millis(300)); // a kill would be done by now
        assert!(
            alive(&kid.0),
            "a child with a session of its own was killed"
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn exit_code_stdout_stderr_and_a_large_stdin() {
        let d = scratch("io");
        let s = script(&d, "e", "cat >/dev/null\necho out\necho err >&2\nexit 3");
        let big = vec![b'x'; 3 << 20];
        let o = exec(&s, &["a"], Some(&big), None, Duration::from_secs(20)).unwrap();
        assert_eq!(o.code, 3);
        assert_eq!(o.stdout, b"out\n");
        assert_eq!(o.stderr, b"err\n");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_grandchild_holding_the_pipe_does_not_hang_us() {
        let d = scratch("pipe");
        let s = script(&d, "p", "(sleep 3 &)\necho done");
        let t0 = Instant::now();
        let o = exec(&s, &[], None, None, Duration::from_secs(20)).unwrap();
        assert_eq!(o.stdout, b"done\n");
        assert!(t0.elapsed() < Duration::from_secs(2));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn verify_rejects_the_wrong_version() {
        let d = scratch("ver");
        let s = script(&d, "torpor", "echo torpor 0.0.0");
        let e = verify(&s).unwrap_err();
        assert!(e.contains("engine.lock pins"), "{e}");
        let _ = fs::remove_dir_all(&d);
    }

    // Linux only: macOS starts a script that is still open for writing, so there is no ETXTBSY to retry (CI run 40439904535).
    #[cfg(target_os = "linux")]
    #[test]
    fn a_program_still_open_for_writing_is_started_once_it_is_closed() {
        let d = scratch("etxtbsy");
        let s = script(&d, "p", "exit 0");
        let writer = fs::OpenOptions::new().append(true).open(&s).unwrap();
        let busy = Command::new(&s).spawn().unwrap_err(); // without the retry this is the flake
        assert_eq!(busy.raw_os_error(), Some(26), "{busy}");
        let closer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            drop(writer);
        });
        let o = exec(&s, &[], None, None, Duration::from_secs(20)).unwrap();
        assert_eq!(o.code, 0);
        closer.join().unwrap();
        let _ = fs::remove_dir_all(&d);
    }
}

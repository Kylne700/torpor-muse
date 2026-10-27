//! Finds the released `torpor` CLI (env TORPOR_BIN, then PATH), checks it against engine.lock, runs it with a hard
//! timeout. Never passes a key on argv: the callers only hand over `hook <Event>` or `version`. Owner: S3a-core.

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};

pub struct Output {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

const LOCK: &str = include_str!("../engine.lock");
/// After the engine is gone a grandchild may still hold its pipes: wait this long for them, then keep what we have.
const PIPE_GRACE: Duration = Duration::from_millis(300);
/// Output we keep per pipe; the rest is read and dropped so the engine never blocks on a full pipe.
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

/// Creates `dir` (and parents) with mode 0700, like the engine does for its own directories.
pub fn private_dir(dir: &Path) -> std::io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// Writes `path` (mode 0600) through a temp file and a rename, so a reader never sees half a file.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        private_dir(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let put = || -> std::io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        fs::rename(&tmp, path)
    };
    put().inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
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
    let cache = state_dir().join("engine-ok");
    if fs::read_to_string(&cache).is_ok_and(|s| s == stamp) {
        return Ok(());
    }
    let out = exec(bin, &["version"], None, Duration::from_secs(5))?;
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
/// missing, wrong version, spawn failure, or killed at the timeout (the whole process group dies).
pub fn run(args: &[&str], stdin: Option<&[u8]>, timeout: Duration) -> Result<Output, String> {
    let bin = locate()?;
    verify(&bin)?;
    exec(&bin, args, stdin, timeout)
}

/// One pipe read to the end in the background; the bytes collected so far stay readable even if it never ends.
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

/// Spawns `bin args` in its own process group, feeds `stdin`, and waits at most `timeout`.
fn exec(
    bin: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    timeout: Duration,
) -> Result<Output, String> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0); // own group: a timeout can kill the engine and everything it started in it
    let mut child = crate::liveness::spawn_retry(&mut cmd)
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
    if timed_out {
        // SAFETY: kill(2) on the group we created; the child is not reaped yet, so the id cannot have been reused.
        unsafe { libc::kill(-pgid, libc::SIGKILL) };
        let _ = rx.recv();
    }
    let until = Instant::now() + PIPE_GRACE;
    let (stdout, stderr) = (out.finish(until), err.finish(until));
    if timed_out {
        return Err(format!(
            "{} {} timed out after {} ms and was killed",
            bin.display(),
            args.join(" "),
            timeout.as_millis()
        ));
    }
    let code = match status {
        Ok(Ok(s)) => s.code().unwrap_or(-1),
        _ => -1,
    };
    Ok(Output {
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

    #[test]
    fn exit_code_stdout_stderr_and_a_large_stdin() {
        let d = scratch("io");
        let s = script(&d, "e", "cat >/dev/null\necho out\necho err >&2\nexit 3");
        let big = vec![b'x'; 3 << 20];
        let o = exec(&s, &["a"], Some(&big), Duration::from_secs(20)).unwrap();
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
        let o = exec(&s, &[], None, Duration::from_secs(20)).unwrap();
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
        let o = exec(&s, &[], None, Duration::from_secs(20)).unwrap();
        assert_eq!(o.code, 0);
        closer.join().unwrap();
        let _ = fs::remove_dir_all(&d);
    }
}

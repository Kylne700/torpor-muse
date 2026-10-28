//! Copying the new lines of Muse's session log into our transcript: the lock, the offset, the batch and its state.

use super::line::translate_line;
use super::{Paths, State, dir_of, find_log};
use crate::util::{private_dir, write_atomic};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// A first look at a log longer than this reads only its newest part: older prompts and compactions are history.
const COLD_TAIL: u64 = 8 << 20;

/// At most this much new log per call; the rest waits for the next call (offsets only move over whole lines).
const CALL_MAX: u64 = 32 << 20;

/// A longer line is skipped unread (the longest seen on 1.4.13 is 3 MB: a 1.5 MB prompt, twice).
const LINE_MAX: usize = 16 << 20;

const LOCK_WAIT: Duration = Duration::from_millis(300);

/// Brings `<vault>/muse/transcripts/<sid>.jsonl` up to date with Muse's log. Err = it could not (the old file stays).
pub fn translate(p: &Paths, sid: &str) -> Result<PathBuf, String> {
    let dir = dir_of(&p.vault);
    private_dir(&dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
    let tpath = dir.join(format!("{sid}.jsonl"));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(dir.join(format!("{sid}.lock")))
        .map_err(|e| format!("lock file: {e}"))?;
    let t0 = Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if t0.elapsed() < LOCK_WAIT => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(TryLockError::WouldBlock) => {
                return Err(
                    "another hook holds the translation lock; kept the file as it is".into(),
                );
            }
            Err(TryLockError::Error(e)) => return Err(format!("lock: {e}")),
        }
    }
    let spath = dir.join(format!("{sid}.state"));
    let mut st: State = fs::read(&spath)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut log = PathBuf::from(&st.log);
    let mut reset = !log.is_file();
    if reset {
        log = find_log(&p.sessions, sid)
            .ok_or_else(|| format!("no session log of {sid} under {}", p.sessions.display()))?;
    }
    let log_len = fs::metadata(&log)
        .map_err(|e| format!("{}: {e}", log.display()))?
        .len();
    reset |= st.off > log_len; // the log was replaced: start over (the engine copes with a shorter file)
    if reset {
        st = State {
            log: log.to_string_lossy().into_owned(),
            ..State::default()
        };
        if tpath.is_file() {
            File::create(&tpath).map_err(|e| format!("reset transcript: {e}"))?;
        }
    } else {
        // A batch whose state was not saved (crash between the two writes): cut back to the saved length and redo it.
        let tl = fs::metadata(&tpath).map_or(0, |m| m.len());
        if tl > st.tlen {
            OpenOptions::new()
                .write(true)
                .open(&tpath)
                .and_then(|f| f.set_len(st.tlen))
                .map_err(|e| format!("cut transcript: {e}"))?;
        } else {
            st.tlen = tl;
        }
    }

    // Nothing translated yet and a long log: only its newest part (older prompts are history). Later calls go on from
    // the offset, at most CALL_MAX at a time.
    let (mut pos, align) = if st.off == 0 && log_len > COLD_TAIL {
        (log_len - COLD_TAIL - 1, true) // one byte early: when that is a line end, the first line of the tail is whole
    } else {
        (st.off, false)
    };
    let mut f = File::open(&log).map_err(|e| format!("{}: {e}", log.display()))?;
    f.seek(SeekFrom::Start(pos))
        .map_err(|e| format!("seek: {e}"))?;
    let mut rd = BufReader::with_capacity(1 << 20, f);
    let mut out = Vec::new();
    let mut ok = true;
    if align {
        // Landed inside a line: drop the rest of it. No line end in the whole tail: wait for one.
        match skip_line(&mut rd).map_err(|e| format!("read: {e}"))? {
            Some(n) => pos += n,
            None => (ok, pos) = (false, st.off),
        }
    }
    let (mut buf, limit) = (Vec::new(), pos.saturating_add(CALL_MAX));
    while ok && pos < limit {
        buf.clear();
        let n = (&mut rd)
            .take(LINE_MAX as u64 + 1)
            .read_until(b'\n', &mut buf)
            .map_err(|e| format!("read: {e}"))?;
        if buf.last() == Some(&b'\n') {
            pos += n as u64;
            st.lines += 1;
            if let Ok(s) = std::str::from_utf8(&buf[..n - 1]) {
                translate_line(s, &mut out, &mut st);
            }
        } else if n > LINE_MAX {
            match skip_line(&mut rd).map_err(|e| format!("read: {e}"))? {
                Some(k) => {
                    pos += n as u64 + k;
                    st.lines += 1;
                    st.bad += 1;
                }
                None => break,
            }
        } else {
            break; // end of file, or a line Muse is still writing: only whole lines are consumed
        }
    }
    if !out.is_empty() || !tpath.is_file() {
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&tpath)
            .and_then(|mut f| f.write_all(&out))
            .map_err(|e| format!("append transcript: {e}"))?;
    }
    st.off = pos;
    st.tlen = fs::metadata(&tpath).map_or(0, |m| m.len());
    write_atomic(
        &spath,
        &serde_json::to_vec(&st).unwrap_or_default(),
        0o600,
        false,
        || Ok(()),
    )
    .map_err(|e| format!("save state: {e}"))?;
    Ok(tpath)
}

/// Consumes up to and including the next newline. None: the file ended first.
fn skip_line<R: BufRead>(r: &mut R) -> std::io::Result<Option<u64>> {
    let mut n = 0u64;
    loop {
        let b = r.fill_buf()?;
        if b.is_empty() {
            return Ok(None);
        }
        let (used, found) = match b.iter().position(|c| *c == b'\n') {
            Some(i) => (i + 1, true),
            None => (b.len(), false),
        };
        r.consume(used);
        n += used as u64;
        if found {
            return Ok(Some(n));
        }
    }
}

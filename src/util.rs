//! What more than one module needs, once: shell quoting, the FNV-1a hash, UTC dates, private folders, atomic writes
//! and `spawn` with a retry. The test crates that compile doctor.rs, liveness.rs or transcript.rs on their own compile
//! this file too; its own tests are tests/shim_util.rs.
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::Path;
use std::process::{Child, Command};
use std::time::{SystemTime, UNIX_EPOCH};

/// Single quotes only when the shell needs them. Works in sh, bash, zsh and fish.
pub fn quote(s: &str) -> String {
    let plain = !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-./:=+,@%".contains(&b));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// FNV-1a, 64 bit: the same hash on every run (std's hasher promises none). File names are made from it, so a change
/// here would leave the files already written unfound.
pub fn fnv1a(b: &[u8]) -> u64 {
    b.iter().fold(0xcbf2_9ce4_8422_2325, |h, &x| {
        (h ^ u64::from(x)).wrapping_mul(0x0100_0000_01b3)
    })
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// "2026-10-26T19:25:46Z" from Unix seconds (UTC, no leap seconds).
pub fn iso(secs: u64) -> String {
    let (days, rem) = ((secs / 86_400) as i64, secs % 86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Creates `dir` (and parents) with mode 0700, like the engine does for its own directories.
pub fn private_dir(dir: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

/// Writes `bytes` to `path` through a temp file in the same directory, then renames it over `path` (one `rename` call).
/// `mode` is for the new file; `sync`: fsync it before the rename; `before_rename` runs after that.
/// A failure (or a crash) before the rename leaves `path` as it was. The folder must exist.
pub fn write_atomic(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    sync: bool,
    before_rename: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.tmp{}", std::process::id()));
    let _ = fs::remove_file(&tmp); // left over from a crash of an earlier run with the same process id
    let put = || -> io::Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&tmp)?;
        f.write_all(bytes)?;
        if sync {
            f.sync_all()?;
        }
        before_rename()?;
        fs::rename(&tmp, path)
    };
    put().inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

/// `cmd.spawn()`, tried again for a moment while the program is still open for writing somewhere (ETXTBSY, 26: an
/// engine being replaced, or a script just written while another thread's forked child still holds its write fd).
/// Everything that starts the engine, a hook command or the writer goes through this and keeps its own error text.
pub fn spawn_retry(cmd: &mut Command) -> io::Result<Child> {
    for _ in 0..6 {
        match cmd.spawn() {
            Err(e) if e.raw_os_error() == Some(26) => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            r => return r,
        }
    }
    cmd.spawn()
}

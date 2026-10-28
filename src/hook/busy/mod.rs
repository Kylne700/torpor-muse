//! F4 item 4 (drill D07): a Bash command that would delete, move or overwrite a SQLite database that is in use.
//!
//! The engine saves no database whose `-wal` file is not empty or whose `-journal` file exists (its capture.rs
//! `db_is_busy`), so once such a file is gone `torpor back --with-data` has nothing to bring back ("Not touched ...
//! its database was busy"). `targets` reads a command line roughly as a shell does and names the busy databases it would
//! remove, move or overwrite; the hook (hook.rs `busy_guard`) then denies the command and tells the model why.
//!
//! What counts: `rm`, `unlink`, `shred`, `git rm`; `mv` (what it moves and what it overwrites); `cp` and `install`
//! (what they overwrite, also with `-t DIR`); `truncate`; a `>`, `>>` or `>|` redirect; `find ... -delete` or
//! `-exec rm` unless its `-name`, `-iname` and `-type` tests cannot reach the database; `git clean` unless it is a dry
//! run or git itself says it leaves the database and its `-wal`, `-shm` and `-journal` files alone (`.gitignore`, `-e`);
//! also inside `sh -c '...'`, after `sudo`, `env` and the like, and after a plain `cd DIR`. A word counts when it names
//! the database or one of its `-wal`, `-shm` or `-journal` files, is a glob (`{a,b}` is expanded as a shell does) that
//! matches one, or (for `rm -r`, `mv`, `find` and `git clean`) names a folder that holds one. Only databases inside the
//! project folder count: the engine saves nothing outside it.
//! shortcut: SQL on a busy database (`sqlite3 app.db "DROP TABLE t"`), `dd`, `tee`, `sed -i`, `rsync`, `xargs rm`,
//! `git stash -u`, `git checkout`/`restore`/`reset --hard` over a tracked busy database, a script, `make clean` or any
//! other program that removes it, words made with `$VAR`, `$(...)` or `~`, a wrapper option with a separate value
//! (`sudo -u x rm ...`, `nice -n 10 rm ...`: the value is read as the command), a `cd` inside `( )` that goes on into
//! the commands after it, a busy database in an ignored folder (the engine would not have saved it either), a link to a
//! busy database (named like the database itself) and anything past `LOOK` directory entries all pass or are judged
//! wrongly; upgrade when a drill or a live run shows one of them. A here-document body is not read as commands unless
//! the input runs a script or a shell (`bash`, `source`, `eval`, any `/` command), so `python3 <<EOF` hides what it runs.
//! A `find` with `-o`, `!`, `-not`, a parenthesis or `-prune` in front of its action (the common `-prune -o` idiom) is
//! denied as if it had no tests; its `-path` and `-regex` tests are not judged. One `git clean` is judged whole, and
//! `-ff` (nested repositories) is always denied.

mod args;
mod glob;
mod lexer;
mod spy;

use spy::Spy;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// Directory entries one command may make us read (glob matches and folder walks together); the rest passes.
const LOOK: usize = 20_000;

/// The engine's own test for a database file (its scan.rs): .db, .sqlite, .sqlite3, any case.
pub fn is_db_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [".db", ".sqlite", ".sqlite3"]
        .iter()
        .any(|x| n.ends_with(x))
}

/// The engine's test for "in use" (capture.rs `db_is_busy`): a SQLite file with a non-empty `-wal` or a `-journal`.
fn busy(p: &Path) -> bool {
    if !p
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(is_db_name)
        || !std::fs::metadata(p).is_ok_and(|m| m.is_file())
    // a FIFO called x.db would block the open
    {
        return false;
    }
    let mut magic = [0u8; 16];
    if std::fs::File::open(p)
        .and_then(|mut f| f.read_exact(&mut magic))
        .is_err()
        || &magic != MAGIC
    {
        return false;
    }
    let side = |s: &str| {
        let mut o = p.as_os_str().to_os_string();
        o.push(s);
        PathBuf::from(o)
    };
    std::fs::metadata(side("-wal")).is_ok_and(|m| m.len() > 0) || side("-journal").exists()
}

/// The busy databases inside `root` (the project folder) that `cmd`, run in `dir`, would remove, move or overwrite,
/// as paths relative to `root`. Empty = nothing the engine could not save is touched.
pub fn targets(cmd: &str, dir: &Path, root: &Path) -> Vec<String> {
    let Ok(root) = root.canonicalize() else {
        return Vec::new();
    };
    let mut spy = Spy {
        root,
        left: LOOK,
        hits: Vec::new(),
    };
    spy.scan(cmd, &mut dir.to_path_buf(), 0);
    spy.hits.sort(); // the folder order is the disk's; the reason shown should not change from run to run
    spy.hits
}

#[cfg(test)]
mod tests;

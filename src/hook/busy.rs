//! F4 item 4 (drill D07): a Bash command that would delete, move or overwrite a SQLite database that is in use.
//!
//! The engine saves no database whose `-wal` file is not empty or whose `-journal` file exists (its capture.rs
//! `db_is_busy`), so once such a file is gone `torpor back --with-data` has nothing to bring back ("Not touched ...
//! its database was busy"). `targets` reads a command line roughly as a shell does and names the busy databases it would
//! remove, move or overwrite; the hook (hook.rs `busy_guard`) then denies the command and tells the model why.
//!
//! What counts: `rm`, `unlink`, `shred`, `git rm`; `mv` (what it moves and what it overwrites); `cp` and `install`
//! (what they overwrite); `truncate`; a `>`, `>>` or `>|` redirect; `find ... -delete` or `-exec rm`; `git clean` unless
//! it is a dry run; also inside `sh -c '...'`, after `sudo`, `env` and the like, and after a plain `cd DIR`. A word
//! counts when it names the database or one of its `-wal`, `-shm` or `-journal` files, is a glob (or `{a,b}`) that
//! matches one, or (for `rm -r`, `mv`, `find` and `git clean`) names a folder that holds one. Only databases inside the
//! project folder count: the engine saves nothing outside it.
//! shortcut: SQL on a busy database (`sqlite3 app.db "DROP TABLE t"`), `dd`, `tee`, `sed -i`, `rsync`, `xargs rm`,
//! `git stash -u`, `git checkout`/`restore`/`reset --hard` over a tracked busy database, a script, `make clean` or any
//! other program that removes it, words made with `$VAR`, `$(...)` or `~`, a wrapper option with a separate value
//! (`sudo -u x rm ...`, `nice -n 10 rm ...`: the value is read as the command), a `cd` inside `( )` that goes on into
//! the commands after it, here-document text read as commands, a busy database in an ignored folder (the engine would
//! not have saved it either), a link to a busy database (named like the database itself) and anything past `LOOK`
//! directory entries all pass or are judged wrongly; upgrade when a drill or a live run shows one of them.

use std::io::Read;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// Directory entries one command may make us read (glob matches and folder walks together); the rest passes.
const LOOK: usize = 20_000;
const DEPTH: usize = 12;
/// Words that only run the next one: skipped, with the `-x` options right after them, to find the real command.
const WRAP: &[&str] = &[
    "sudo", "doas", "command", "builtin", "exec", "env", "nohup", "time", "nice", "ionice",
    "stdbuf", "setsid", "if", "then", "else", "elif", "do", "while", "until", "!", "{", "}",
];

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

/// A word and whether it holds an unquoted glob character (`*`, `?`, `[`, `{`, `}`).
type W = (String, bool);

enum Tok {
    Word(String, bool),
    /// `;`, `&&`, `|`, a newline, a parenthesis or a backtick: what follows is a new command.
    Sep,
    /// A redirect; the next word is its target (a file when `true`, else an input or a file descriptor).
    Redir(bool),
}

#[allow(unused_assignments)] // the last `flush!` resets what nothing reads again
fn lex(s: &str) -> Vec<Tok> {
    let c: Vec<char> = s.chars().collect();
    let (mut out, mut w, mut glob, mut inw, mut i) = (Vec::new(), String::new(), false, false, 0);
    macro_rules! flush {
        () => {
            if inw {
                out.push(Tok::Word(std::mem::take(&mut w), glob));
                (glob, inw) = (false, false);
            }
        };
    }
    while i < c.len() {
        match c[i] {
            ' ' | '\t' | '\r' => flush!(),
            '\n' | ';' | '|' | '(' | ')' | '`' => {
                flush!();
                out.push(Tok::Sep);
            }
            '&' if c.get(i + 1) == Some(&'>') => {
                flush!();
                out.push(Tok::Redir(true));
                i += if c.get(i + 2) == Some(&'>') { 2 } else { 1 };
            }
            '&' => {
                flush!();
                out.push(Tok::Sep);
            }
            '>' | '<' => {
                if inw && w.chars().all(|d| d.is_ascii_digit()) {
                    // `2>`: a file descriptor, not a word
                    w.clear();
                    (glob, inw) = (false, false);
                } else {
                    flush!();
                }
                let (out_op, next) = (c[i] == '>', c.get(i + 1).copied());
                let write = match next {
                    Some('&') => {
                        i += 1; // `>&2`, `2>&1`: a copy of a descriptor
                        false
                    }
                    Some('>' | '|') if out_op => {
                        i += 1;
                        true
                    }
                    Some('<') if !out_op => {
                        i += 1 + usize::from(matches!(c.get(i + 2), Some('<' | '-')));
                        false
                    }
                    _ => out_op,
                };
                out.push(Tok::Redir(write));
            }
            '#' if !inw => {
                while i + 1 < c.len() && c[i + 1] != '\n' {
                    i += 1;
                }
            }
            '\'' => {
                inw = true;
                i += 1;
                while i < c.len() && c[i] != '\'' {
                    w.push(c[i]);
                    i += 1;
                }
            }
            '"' => {
                inw = true;
                i += 1;
                while i < c.len() && c[i] != '"' {
                    if c[i] == '\\' && matches!(c.get(i + 1), Some('"' | '\\' | '$' | '`')) {
                        i += 1;
                    }
                    w.push(c[i]);
                    i += 1;
                }
            }
            '\\' => {
                i += 1;
                if let Some(&n) = c.get(i).filter(|n| **n != '\n') {
                    w.push(n);
                    inw = true;
                }
            }
            ch => {
                inw = true;
                glob |= matches!(ch, '*' | '?' | '[' | '{' | '}');
                w.push(ch);
            }
        }
        i += 1;
    }
    flush!();
    out
}

#[derive(Default)]
struct Cmd {
    words: Vec<W>,
    /// Targets of `>`, `>>`, `>|` and `&>`.
    writes: Vec<W>,
}

fn commands(s: &str) -> Vec<Cmd> {
    let (mut all, mut cur, mut redir) = (Vec::new(), Cmd::default(), None);
    for t in lex(s) {
        match t {
            Tok::Word(w, g) => match redir.take() {
                Some(true) => cur.writes.push((w, g)),
                Some(false) => {}
                None => cur.words.push((w, g)),
            },
            Tok::Redir(write) => redir = Some(write),
            Tok::Sep => {
                redir = None;
                all.push(std::mem::take(&mut cur));
            }
        }
    }
    all.push(cur);
    all
}

fn base(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or(w)
}

fn is_assign(w: &str) -> bool {
    w.split_once('=').is_some_and(|(k, _)| {
        k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !k.is_empty()
            && !k.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// Options (`-x`, `--long`, up to a `--`) and operands of a command's arguments.
fn split(args: &[W]) -> (Vec<&str>, Vec<&W>) {
    let (mut opts, mut ops, mut end) = (Vec::new(), Vec::new(), false);
    for a in args {
        if !end && a.0 == "--" {
            end = true;
        } else if !end && a.0.len() > 1 && a.0.starts_with('-') {
            opts.push(a.0.as_str());
        } else {
            ops.push(a);
        }
    }
    (opts, ops)
}

/// `{a,b}` is read as `*` (more than it matches, which is the safe side).
fn unbrace(p: &str) -> String {
    let (mut out, mut depth) = (String::new(), 0);
    for ch in p.chars() {
        match ch {
            '{' => {
                if depth == 0 {
                    out.push('*');
                }
                depth += 1;
            }
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Shell glob on one name: `*`, `?`, `[a-z]`, `[!x]`; a leading dot is matched only by a leading dot.
fn glob_match(pat: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pat.chars().collect(), name.chars().collect());
    if n.first() == Some(&'.') && p.first() != Some(&'.') {
        return false;
    }
    // how many pattern characters (from `i`, not a `*`) stand for the character `c`; None = it does not match
    let one = |i: usize, c: char| -> Option<usize> {
        match p[i] {
            '?' => Some(1),
            '[' => match p.iter().skip(i + 2).position(|&x| x == ']') {
                Some(e) => {
                    let e = i + 2 + e;
                    let (neg, set) = match p[i + 1] {
                        '!' | '^' => (true, &p[i + 2..e]),
                        _ => (false, &p[i + 1..e]),
                    };
                    let (mut hit, mut k) = (false, 0);
                    while k < set.len() {
                        if k + 2 < set.len() && set[k + 1] == '-' {
                            hit |= set[k] <= c && c <= set[k + 2];
                            k += 3;
                        } else {
                            hit |= set[k] == c;
                            k += 1;
                        }
                    }
                    (hit != neg).then_some(e + 1 - i)
                }
                None => (c == '[').then_some(1), // no closing bracket: a plain `[`
            },
            x => (x == c).then_some(1),
        }
    };
    // one pass with the last `*` to go back to: no more than pattern times name steps, whatever the stars
    let (mut pi, mut ni, mut back) = (0, 0, None);
    while ni < n.len() {
        if p.get(pi) == Some(&'*') {
            back = Some((pi, ni));
            pi += 1;
        } else if let Some(len) = (pi < p.len()).then(|| one(pi, n[ni])).flatten() {
            (pi, ni) = (pi + len, ni + 1);
        } else if let Some((sp, sn)) = back {
            (pi, ni, back) = (sp + 1, sn + 1, Some((sp, sn + 1)));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

struct Spy {
    /// The project folder, as the kernel names it; a database outside it is none of our business.
    root: PathBuf,
    left: usize,
    hits: Vec<String>,
}

impl Spy {
    /// The paths a word names: itself, or the entries a glob matches (read from the folders, within `LOOK`).
    /// Nothing for a word we cannot resolve (`$VAR`, `$(..)`, `~`).
    fn paths(&mut self, dir: &Path, w: &str, glob: bool) -> Vec<PathBuf> {
        if w.is_empty() || w.contains(['$', '`']) || w.starts_with('~') {
            return Vec::new();
        }
        if !glob {
            return vec![dir.join(w)];
        }
        let pat = unbrace(w);
        let mut cur = vec![if pat.starts_with('/') {
            PathBuf::from("/")
        } else {
            dir.to_path_buf()
        }];
        for comp in pat.split('/').filter(|c| !c.is_empty()) {
            let mut next = Vec::new();
            for b in &cur {
                if !comp.contains(['*', '?', '[']) {
                    next.push(b.join(comp));
                    continue;
                }
                for e in std::fs::read_dir(b).into_iter().flatten().flatten() {
                    if self.left == 0 {
                        break;
                    }
                    self.left -= 1;
                    let name = e.file_name();
                    if glob_match(comp, &name.to_string_lossy()) {
                        next.push(b.join(name));
                    }
                }
            }
            next.truncate(4096);
            cur = next;
        }
        cur
    }

    fn add(&mut self, p: &Path) {
        if !busy(p) {
            return;
        }
        let c = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
        if c.starts_with(&self.root) {
            let shown = c
                .strip_prefix(&self.root)
                .unwrap_or(&c)
                .display()
                .to_string();
            if !self.hits.contains(&shown) {
                self.hits.push(shown);
            }
        }
    }

    /// `p` is removed, moved or overwritten: it, the database its name is a `-wal`, `-shm` or `-journal` file of, and
    /// (when `deep`) every database in it if it is a folder.
    fn visit(&mut self, p: &Path, deep: bool) {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        for s in ["-wal", "-shm", "-journal"] {
            if let Some(b) = name.strip_suffix(s).filter(|b| is_db_name(b)) {
                self.add(&p.with_file_name(b));
            }
        }
        self.add(p);
        if deep && std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()) {
            self.walk(p, 0);
        }
    }

    fn walk(&mut self, dir: &Path, depth: usize) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if self.left == 0 {
                return;
            }
            self.left -= 1;
            let Ok(t) = e.file_type() else { continue };
            if t.is_file() {
                self.add(&e.path());
            } else if t.is_dir()
                && depth < DEPTH
                && !matches!(e.file_name().to_str(), Some(".git" | "node_modules"))
            {
                self.walk(&e.path(), depth + 1);
            }
        }
    }

    fn all(&mut self, dir: &Path, ops: &[&W], deep: bool) {
        for (w, g) in ops {
            for p in self.paths(dir, w, *g) {
                self.visit(&p, deep);
            }
        }
    }

    /// `verb args`: the commands that remove, move or overwrite what their operands name.
    fn verb(&mut self, verb: &str, args: &[W], dir: &Path) {
        let (opts, ops) = split(args);
        match verb {
            "rm" | "unlink" | "shred" => {
                let deep = opts.iter().any(|o| {
                    *o == "--recursive" || (!o.starts_with("--") && o[1..].contains(['r', 'R']))
                });
                self.all(dir, &ops, deep);
            }
            "truncate" => self.all(dir, &ops, false),
            "mv" | "cp" | "install" => {
                let Some((last, srcs)) = ops.split_last() else {
                    return;
                };
                let srcs: Vec<PathBuf> = srcs
                    .iter()
                    .flat_map(|(w, g)| self.paths(dir, w, *g))
                    .collect();
                if verb == "mv" {
                    for s in &srcs {
                        self.visit(s, true);
                    }
                }
                for d in self.paths(dir, &last.0, last.1) {
                    if d.is_dir() {
                        for n in srcs
                            .iter()
                            .filter(|s| s.exists())
                            .filter_map(|s| s.file_name())
                        {
                            self.visit(&d.join(n), false); // moved or copied into the folder under its own name
                        }
                    } else {
                        self.visit(&d, false);
                    }
                }
            }
            "find" => {
                let del = args.iter().any(|(w, _)| w == "-delete")
                    || args.windows(2).any(|p| {
                        p[0].0.starts_with("-exec")
                            && matches!(base(&p[1].0), "rm" | "unlink" | "shred" | "mv")
                    });
                if del {
                    let mut starts: Vec<&W> = args
                        .iter()
                        .take_while(|(w, _)| {
                            !w.starts_with('-') || matches!(w.as_str(), "-H" | "-L" | "-P")
                        })
                        .filter(|(w, _)| !w.starts_with('-'))
                        .collect();
                    let here = (".".to_string(), false);
                    if starts.is_empty() {
                        starts.push(&here);
                    }
                    self.all(dir, &starts, true);
                }
            }
            _ => {}
        }
    }

    fn scan(&mut self, cmd: &str, dir: &mut PathBuf, depth: u8) {
        for c in commands(cmd) {
            for (w, g) in &c.writes {
                for p in self.paths(dir, w, *g) {
                    self.visit(&p, false);
                }
            }
            let mut ws = c.words.as_slice();
            let mut wrapped = false;
            while let Some((w, _)) = ws.first() {
                if is_assign(w) || WRAP.contains(&base(w)) {
                    wrapped = true;
                } else if !(wrapped && w.starts_with('-')) {
                    break;
                }
                ws = &ws[1..];
            }
            let Some((v, _)) = ws.first() else { continue };
            let (verb, args) = (base(v), &ws[1..]);
            match verb {
                "cd" => {
                    if let Some((w, false)) = split(args).1.first() {
                        let d = dir.join(w);
                        if d.is_dir() {
                            *dir = d;
                        }
                    }
                }
                "sh" | "bash" | "zsh" | "dash" | "ksh" if depth < 2 => {
                    let (opts, ops) = split(args);
                    if let (true, Some((script, _))) = (
                        opts.iter().any(|o| !o.starts_with("--") && o.contains('c')),
                        ops.first(),
                    ) {
                        self.scan(script, &mut dir.clone(), depth + 1);
                    }
                }
                "git" => {
                    let (mut d, mut i) = (dir.clone(), 0);
                    while args.get(i).is_some_and(|(w, _)| w.starts_with('-')) {
                        match args[i].0.as_str() {
                            "-C" => {
                                d = d.join(args.get(i + 1).map_or("", |(w, _)| w.as_str()));
                                i += 2;
                            }
                            "-c" | "--git-dir" | "--work-tree" | "--namespace" => i += 2,
                            _ => i += 1,
                        }
                    }
                    let rest = args.get(i + 1..).unwrap_or(&[]);
                    match args.get(i).map(|(w, _)| w.as_str()) {
                        Some("clean") => {
                            let (opts, ops) = split(rest);
                            let dry = opts.iter().any(|o| {
                                *o == "--dry-run" || (!o.starts_with("--") && o.contains('n'))
                            });
                            if !dry && ops.is_empty() {
                                self.walk(&d, 0);
                            } else if !dry {
                                self.all(&d, &ops, true);
                            }
                        }
                        Some(g @ ("rm" | "mv")) if !rest.iter().any(|(w, _)| w == "--cached") => {
                            self.verb(g, rest, &d);
                        }
                        _ => {}
                    }
                }
                _ => self.verb(verb, args, dir),
            }
        }
    }
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
mod tests {
    use super::*;
    use std::fs;

    /// A project with a busy database (`data/app.db` with a `-wal` that holds rows), one that is idle, one that is
    /// not SQLite, a busy one in a deep folder (journal), and a busy one next to the project.
    fn project(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tm-busy-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        let root = d.join("proj");
        for sub in ["data", "sub/deep", "lone"] {
            fs::create_dir_all(root.join(sub)).unwrap();
        }
        fs::create_dir_all(d.join("beside")).unwrap();
        let sqlite = |p: PathBuf| fs::write(p, [MAGIC.as_slice(), &[0u8; 100]].concat()).unwrap();
        sqlite(root.join("data/app.db"));
        fs::write(root.join("data/app.db-wal"), "frames").unwrap();
        fs::write(root.join("data/app.db-shm"), "").unwrap();
        sqlite(root.join("idle.db"));
        fs::write(root.join("idle.db-wal"), "").unwrap(); // an empty wal is not "in use"
        fs::write(root.join("plain.db"), "just text, not SQLite").unwrap();
        fs::write(root.join("plain.db-wal"), "frames").unwrap();
        sqlite(root.join("sub/deep/x.SQLITE"));
        fs::write(root.join("sub/deep/x.SQLITE-journal"), "j").unwrap();
        fs::write(root.join("lone/a.txt"), "a").unwrap();
        fs::write(root.join("lone/app.db"), "not SQLite").unwrap();
        fs::write(root.join("notes.txt"), "n").unwrap();
        sqlite(d.join("beside/other.db"));
        fs::write(d.join("beside/other.db-wal"), "frames").unwrap();
        root
    }

    fn hits(root: &Path, cmd: &str) -> Vec<String> {
        targets(cmd, root, root)
    }

    #[test]
    fn a_command_that_removes_moves_or_overwrites_a_busy_database_is_named() {
        let root = project("denied");
        let app = "data/app.db".to_string();
        for cmd in [
            "rm -f data/app.db data/app.db-wal data/app.db-shm",
            "rm data/app.db",
            "rm -f data/app.db-wal",
            "rm -f data/app.db-shm",
            "rm -rf data",
            "rm -rf data/",
            "rm -rf ./data/.",
            "rm -f data/*",
            "rm -f data/*.db",
            "rm -f data/app.d?",
            "rm -f data/app.[a-z]b",
            "rm -f data/app.db{,-wal,-shm}",
            "rm -f data/{app,x}.db",
            "rm -f sub/../data/app.db",
            "cd data && rm -f app.db",
            "cd data; rm app.db",
            "cd ./data\nrm app.db",
            "echo clean up; rm data/app.db",
            "ls || rm data/app.db",
            "true & rm data/app.db",
            "sudo rm -f data/app.db",
            "sudo -n rm -f data/app.db",
            "FOO=1 BAR=2 rm data/app.db",
            "env FOO=1 rm data/app.db",
            "nohup rm data/app.db &",
            "if true; then rm data/app.db; fi",
            "bash -c 'rm -f data/app.db'",
            "sh -lc \"cd data && rm app.db\"",
            "rm -- data/app.db",
            "\\rm data/app.db",
            "/bin/rm data/app.db",
            "rm \"data/app.db\"",
            "rm 'data/app.db'",
            "rm -v data/app.db 2>/dev/null",
            "unlink data/app.db",
            "shred -u data/app.db",
            "mv data/app.db /tmp/app.db.old",
            "mv data /tmp/data.old",
            "mv notes.txt data/app.db",
            "mv -f lone/app.db data/",
            "cp lone/app.db data/",
            "cp notes.txt data/app.db",
            "cp -f idle.db data/app.db",
            "install -m 644 notes.txt data/app.db",
            "truncate -s 0 data/app.db",
            "> data/app.db",
            ": > data/app.db",
            "echo x >> data/app.db",
            "cat notes.txt >| data/app.db",
            "echo x &> data/app.db",
            "echo $(rm data/app.db)",
            "echo `rm data/app.db`",
            "find data -name '*.db' -delete",
            "find . -name '*.db' -exec rm {} \\;",
            "find data -type f -exec rm -f {} +",
            "git clean -fdx",
            "git clean -f data",
            "git -C data clean -fd",
            "cd data && git clean -fd",
            "git rm -f data/app.db",
            "git mv data/app.db data/old.db",
        ] {
            let got = hits(&root, cmd);
            assert!(
                got.contains(&app),
                "{cmd:?} must name data/app.db, got {got:?}"
            );
        }
        // a glob over the project and a folder in it also find the deep one
        for cmd in ["rm -rf *", "rm -rf sub", "rm -r ./sub/", "git clean -fd"] {
            assert!(
                hits(&root, cmd).contains(&"sub/deep/x.SQLITE".to_string()),
                "{cmd:?}"
            );
        }
        assert!(
            hits(&root, "rm -f sub/deep/x.SQLITE-journal")
                .contains(&"sub/deep/x.SQLITE".to_string())
        );
        assert_eq!(
            hits(&root, "rm -rf *"),
            vec!["data/app.db".to_string(), "sub/deep/x.SQLITE".to_string()]
        );
        fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn commands_that_name_no_busy_database_or_only_read_one_pass() {
        let root = project("passed");
        for cmd in [
            "ls data",
            "cat data/app.db > /dev/null",
            "cat data/app.db | head -c 16",
            "sqlite3 data/app.db .dump > dump.sql",
            "head -c 100 data/app.db >/tmp/x.bin",
            "cp data/app.db /tmp/backup.db",
            "cp data/app.db backup.db",
            "mv notes.txt other.txt",
            "mv lone/a.txt lone/b.txt",
            "rm notes.txt",
            "rm -f lone/a.txt",
            "rm -rf lone",
            "rm idle.db idle.db-wal",   // not in use: the engine saves it
            "rm plain.db plain.db-wal", // not SQLite
            "rm -f missing.db",
            "rm data", // a folder without -r: rm refuses
            "rm -f *.txt",
            "rm -f lone/*",
            "rm -rf .hidden*",
            "echo rm data/app.db",
            "echo 'rm -f data/app.db'",
            "grep -r app.db data",
            "git status",
            "git clean -n",
            "git clean -fdn",
            "git clean --dry-run -fd",
            "git -C lone clean -fd",
            "git rm --cached data/app.db",
            "git log -- data/app.db",
            "ls # rm data/app.db",
            "find data -name '*.db'",
            "find data -name '*.db' -print",
            "rm $DB",
            "rm \"$HOME/data/app.db\"",
            "rm ~/data/app.db",
            "rm -f ../beside/other.db", // next to the project, not in it
            "rm -rf ../beside",
            "rm -rf lone && cd data",
            "sqlite3 data/app.db \"DROP TABLE t\"", // a known limit, see the module comment
            "",
            "   ",
            "rm -f 'unterminated",
        ] {
            let got = hits(&root, cmd);
            assert!(got.is_empty(), "{cmd:?} must pass, got {got:?}");
        }
        fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_folder_or_glob_walk_stays_inside_its_budget_and_a_stopped_app_passes() {
        let root = project("budget");
        // a database that stops being busy (the app closed it: no wal left) is not named any more
        assert_eq!(hits(&root, "rm data/app.db"), ["data/app.db"]);
        fs::remove_file(root.join("data/app.db-wal")).unwrap();
        assert!(hits(&root, "rm -f data/app.db data/app.db-shm").is_empty());
        // the workdir decides what a relative word means
        assert_eq!(
            targets("rm -f app.db", &root.join("sub/deep"), &root),
            Vec::<String>::new()
        );
        assert_eq!(
            targets("rm -f x.SQLITE", &root.join("sub/deep"), &root),
            ["sub/deep/x.SQLITE"]
        );
        // a glob over a huge folder reads at most LOOK entries
        let big = root.join("big");
        fs::create_dir_all(&big).unwrap();
        for i in 0..50 {
            fs::write(big.join(format!("f{i}.txt")), "x").unwrap();
        }
        let mut spy = Spy {
            root: root.canonicalize().unwrap(),
            left: 10,
            hits: vec![],
        };
        let got = spy.paths(&root, "big/*", true);
        assert!(
            got.len() <= 10 && spy.left == 0,
            "{} {}",
            got.len(),
            spy.left
        );
        fs::remove_dir_all(root.parent().unwrap()).unwrap();
    }

    #[test]
    fn globs_and_words_are_read_like_a_shell_does() {
        for (pat, name, yes) in [
            ("*.db", "app.db", true),
            ("*.db", ".hidden.db", false),
            (".*", ".hidden", true),
            ("app.d?", "app.db", true),
            ("app.[a-c]b", "app.db", false),
            ("app.[!a-c]b", "app.db", true),
            ("a*b*c", "aXbYc", true),
            ("a*b*c", "aXbY", false),
            ("[", "[", true),
            ("a*", "a", true),
            ("*", "", true),
            ("?", "", false),
            ("a*b?c*", "aXXb1cYY", true),
        ] {
            assert_eq!(glob_match(pat, name), yes, "{pat} vs {name}");
        }
        // several stars on a name that cannot match must not take forever
        assert!(!glob_match("a*a*a*a*a*a*a*b", &"a".repeat(60)));
        assert_eq!(unbrace("app.db{,-wal,-shm}"), "app.db*");
        assert_eq!(unbrace("{a,{b,c}}.db"), "*.db");
        let words = |s: &str| {
            lex(s)
                .into_iter()
                .filter_map(|t| match t {
                    Tok::Word(w, g) => Some((w, g)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            words(r#"rm "a b" 'c*' d\ e *.f # note"#),
            [
                ("rm", false),
                ("a b", false),
                ("c*", false),
                ("d e", false),
                ("*.f", true)
            ]
            .map(|(w, g)| (w.to_string(), g))
        );
        assert!(
            is_assign("A_1=x") && !is_assign("=x") && !is_assign("1A=x") && !is_assign("a-b=1")
        );
    }
}

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

use crate::engine;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MAGIC: &[u8; 16] = b"SQLite format 3\0";
/// Directory entries one command may make us read (glob matches and folder walks together); the rest passes.
const LOOK: usize = 20_000;
const DEPTH: usize = 12;
/// Words one `{a,b}` word may be expanded into; past it a group is read as `*`.
const BRACES: usize = 256;
/// How long the one `git` call of a `git clean` may take; the hook has 6 s and its engine call may have used some of it.
const GIT_WAIT: Duration = Duration::from_millis(1500);
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

/// Where the text goes on after the here-documents `open` (the end words of the ones the line before started, in order,
/// and whether tabs may stand in front of an end word): past the line that holds the last end word. No such line (the
/// `<<` was a shift, `$((1 << x))`): `from`, nothing is left out.
fn skip_docs(c: &[char], from: usize, open: &mut Vec<(String, bool)>) -> usize {
    let mut at = from;
    for (end, tabs) in open.drain(..) {
        loop {
            if at >= c.len() {
                return from;
            }
            let stop = c[at..]
                .iter()
                .position(|&x| x == '\n')
                .map_or(c.len(), |p| at + p);
            let line: String = c[at..stop].iter().collect();
            at = (stop + 1).min(c.len());
            if (if tabs {
                line.trim_start_matches('\t')
            } else {
                line.as_str()
            }) == end
            {
                break;
            }
        }
    }
    at
}

/// `docs`: leave out the bodies of here-documents (`cat <<EOF` ... `EOF`), which are text and not commands.
#[allow(unused_assignments)] // the last `flush!` resets what nothing reads again
fn lex(s: &str, docs: bool) -> Vec<Tok> {
    let c: Vec<char> = s.chars().collect();
    let (mut out, mut w, mut glob, mut inw, mut i) = (Vec::new(), String::new(), false, false, 0);
    // the here-documents this line opens: `want` is set by `<<` (tabs allowed in front of the end word: `<<-`) and the
    // next word is the end word
    let (mut open, mut want): (Vec<(String, bool)>, Option<bool>) = (Vec::new(), None);
    macro_rules! flush {
        () => {
            if inw {
                if let Some(tabs) = want.take() {
                    open.push((w.clone(), tabs));
                }
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
                want = None;
                if c[i] == '\n' && !open.is_empty() {
                    i = skip_docs(&c, i + 1, &mut open) - 1; // the loop steps to the line after the end word
                }
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
                        let third = c.get(i + 2).copied();
                        if docs && third != Some('<') {
                            want = Some(third == Some('-')); // `<<EOF`, `<<-EOF`; `<<<` is a here-string, no body
                        }
                        i += 1 + usize::from(matches!(third, Some('<' | '-')));
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

fn commands(s: &str, docs: bool) -> Vec<Cmd> {
    let (mut all, mut cur, mut redir) = (Vec::new(), Cmd::default(), None);
    for t in lex(s, docs) {
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

/// The words from the command a list runs on: without the assignments and wrappers (`sudo`, `env`, ...) and their
/// options in front.
fn head(mut ws: &[W]) -> &[W] {
    let mut wrapped = false;
    while let Some((w, _)) = ws.first() {
        if is_assign(w) || WRAP.contains(&base(w)) {
            wrapped = true;
        } else if !(wrapped && w.starts_with('-')) {
            break;
        }
        ws = &ws[1..];
    }
    ws
}

/// True when the command could run text a here-document wrote: a shell, `source`, `.`, `eval` or anything given as a path.
fn runs_script(c: &Cmd) -> bool {
    head(&c.words).first().is_some_and(|(v, _)| {
        v.contains('/')
            || matches!(
                v.as_str(),
                "." | "source" | "eval" | "sh" | "bash" | "zsh" | "dash" | "ksh"
            )
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

/// `cp`, `mv` and `install` put what they copy in the folder that `-t DIR`, `-tDIR`, `-rt DIR`, `--target-directory DIR`
/// or `--target-directory=DIR` names. Returns it and the other arguments.
/// shortcut: a short option with a value (`-S x`) in the same cluster in front of the `t` is read as flags; upgrade when
/// a real command shows it.
fn target_dir(args: &[W]) -> (Option<W>, Vec<W>) {
    let (mut dir, mut rest, mut i) = (None, Vec::new(), 0);
    while i < args.len() {
        let (a, glob) = &args[i];
        i += 1;
        if a == "--" {
            rest.extend_from_slice(&args[i - 1..]);
            break;
        }
        // None: not the `-t` option; Some(None): the folder is the next word; Some(Some(v)): it is stuck to the option
        let value = if let Some(long) = a.strip_prefix("--") {
            let (name, v) = long
                .split_once('=')
                .map_or((long, None), |(n, v)| (n, Some(v)));
            (!name.is_empty() && "target-directory".starts_with(name)).then_some(v)
        } else if a.len() > 1 && a.starts_with('-') {
            a[1..]
                .find('t')
                .map(|k| Some(&a[k + 2..]).filter(|v| !v.is_empty()))
        } else {
            None
        };
        match value {
            None => rest.push(args[i - 1].clone()),
            Some(Some(v)) => dir = Some((v.to_string(), *glob)),
            Some(None) => {
                dir = args.get(i).cloned();
                i += 1;
            }
        }
    }
    (dir, rest)
}

/// What the arguments of `git clean` say.
#[derive(Default)]
struct Clean<'a> {
    /// `-n`, `--dry-run`: nothing is removed.
    dry: bool,
    /// `-x`: files the ignore rules name are removed too (not the ones `-e` names).
    x: bool,
    /// `-X`: only files the ignore rules name are removed.
    only_ignored: bool,
    /// `-f` twice: nested repositories are removed too.
    nested: bool,
    /// The `-e` and `--exclude` patterns.
    exclude: Vec<String>,
    /// The paths it was told to clean (none: the whole folder).
    paths: Vec<&'a W>,
}

fn clean_args(args: &[W]) -> Clean<'_> {
    let (mut c, mut force, mut end, mut i) = (Clean::default(), 0, false, 0);
    while i < args.len() {
        let a = args[i].0.as_str();
        i += 1;
        if end || a == "-" || !a.starts_with('-') {
            c.paths.push(&args[i - 1]);
        } else if a == "--" {
            end = true;
        } else if let Some(long) = a.strip_prefix("--") {
            let (name, v) = long
                .split_once('=')
                .map_or((long, None), |(n, v)| (n, Some(v)));
            match name {
                "dry-run" => c.dry = true,
                "force" => force += 1,
                "exclude" => {
                    let p = match v {
                        Some(v) => Some(v.to_string()),
                        None => {
                            i += 1;
                            args.get(i - 1).map(|x| x.0.clone())
                        }
                    };
                    c.exclude.extend(p);
                }
                _ => {}
            }
        } else {
            for (k, ch) in a.char_indices().skip(1) {
                match ch {
                    'n' => c.dry = true,
                    'x' => c.x = true,
                    'X' => c.only_ignored = true,
                    'f' => force += 1,
                    'e' => {
                        // the rest of the word is the pattern, else the next word is
                        if a[k + 1..].is_empty() {
                            i += 1;
                            c.exclude.extend(args.get(i - 1).map(|x| x.0.clone()));
                        } else {
                            c.exclude.push(a[k + 1..].to_string());
                        }
                        break;
                    }
                    _ => {}
                }
            }
        }
    }
    c.nested = force > 1;
    c
}

/// The words a shell makes of one with `{a,b}` groups: `app.db{,-wal}` is `app.db` and `app.db-wal`. A group with no comma
/// (`{x}`), one that is never closed, and every group past `BRACES` words are read as `*` (more than they match, which is
/// the safe side).
fn expand(w: &str) -> Vec<String> {
    let mut out = Vec::new();
    brace(w, &mut out);
    out
}

fn brace(w: &str, out: &mut Vec<String>) {
    let Some(open) = w.find('{') else {
        out.push(w.to_string());
        return;
    };
    let (head, rest) = (&w[..open], &w[open + 1..]);
    // the `}` that closes this group, and the commas on its own level
    let (mut depth, mut commas, mut close) = (0, Vec::new(), None);
    for (k, ch) in rest.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' if depth == 0 => {
                close = Some(k);
                break;
            }
            '}' => depth -= 1,
            ',' if depth == 0 => commas.push(k),
            _ => {}
        }
    }
    let Some(close) = close else {
        out.push(format!("{head}*"));
        return;
    };
    let tail = &rest[close + 1..];
    if commas.is_empty() || out.len() >= BRACES {
        brace(&format!("{head}*{tail}"), out);
        return;
    }
    let mut from = 0;
    for end in commas.into_iter().chain([close]) {
        brace(&format!("{head}{}{tail}", &rest[from..end]), out);
        from = end + 1;
    }
}

/// Shell glob on one name: `*`, `?`, `[a-z]`, `[!x]`; a leading dot is matched only by a leading dot.
fn glob_match(pat: &str, name: &str) -> bool {
    !(name.starts_with('.') && !pat.starts_with('.')) && fnmatch(pat, name)
}

/// `glob_match` without the dot rule: what `find -name` does (its `*` also matches a leading dot).
fn fnmatch(pat: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pat.chars().collect(), name.chars().collect());
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
        let mut found = Vec::new();
        for pat in expand(w) {
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
            found.extend(cur);
        }
        found
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
                let (target, args) = target_dir(args);
                let (_, ops) = split(&args);
                let (dest, srcs) = match (&target, ops.split_last()) {
                    (Some(t), _) => (t, ops.as_slice()),
                    (None, Some((last, srcs))) => (*last, srcs),
                    (None, None) => return,
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
                for d in self.paths(dir, &dest.0, dest.1) {
                    if d.is_dir() {
                        for n in srcs
                            .iter()
                            .filter(|s| s.exists())
                            .filter_map(|s| s.file_name())
                        {
                            self.visit(&d.join(n), false); // moved or copied into the folder under its own name
                        }
                    } else if target.is_none() {
                        self.visit(&d, false);
                    }
                }
            }
            "find" => self.find(args, dir),
            _ => {}
        }
    }

    /// What this call finds, apart from the hits so far: a command judges the databases it reaches, not earlier ones.
    fn fresh(&mut self, find: impl FnOnce(&mut Self)) -> Vec<String> {
        let before = std::mem::take(&mut self.hits);
        find(self);
        std::mem::replace(&mut self.hits, before)
    }

    /// Adds the hits a `fresh` call found.
    fn keep(&mut self, found: Vec<String>) {
        for h in found {
            if !self.hits.contains(&h) {
                self.hits.push(h);
            }
        }
    }

    /// `find START... TESTS (-delete | -exec rm|mv ...)`: the databases under the start points that the tests can reach.
    fn find(&mut self, args: &[W], dir: &Path) {
        let n = args
            .iter()
            .take_while(|(w, _)| !w.starts_with('-') || matches!(w.as_str(), "-H" | "-L" | "-P"))
            .count();
        let (front, expr) = args.split_at(n);
        let deletes = |k: usize| {
            expr[k].0 == "-delete"
                || (expr[k].0.starts_with("-exec")
                    && expr
                        .get(k + 1)
                        .is_some_and(|(x, _)| matches!(base(x), "rm" | "unlink" | "shred" | "mv")))
        };
        let Some(k) = (0..expr.len()).find(|&k| deletes(k)) else {
            return;
        };
        let exec = expr[k].0 != "-delete";
        let mut starts: Vec<&W> = front.iter().filter(|(w, _)| !w.starts_with('-')).collect();
        let here = (".".to_string(), false);
        if starts.is_empty() {
            starts.push(&here);
        }
        // a start point that is the project folder or holds it (not `.`, which `rm` refuses): a folder test could
        // take the whole folder, database and all
        let whole = starts.iter().any(|(w, _)| {
            !matches!(w.as_str(), "." | "./")
                && dir
                    .join(w)
                    .canonicalize()
                    .is_ok_and(|c| self.root.starts_with(c))
        });
        let found = self.fresh(|s| s.all(dir, &starts, true));
        let reached = found
            .into_iter()
            .filter(|db| self.find_reaches(db, &expr[..k], exec, whole))
            .collect();
        self.keep(reached);
    }

    /// Can a `find` with these `tests` in front of its deleting action take the database `db` (path from the project
    /// folder) or one of its `-wal`, `-shm` or `-journal` files? `exec`: the action is `-exec rm|mv`, which takes a
    /// matching folder with everything in it; `-delete` takes files and empty folders. Doubt means yes.
    /// shortcut: `-o`, `!`, `-not`, parentheses, `-prune` and other actions among the tests are not judged (the common
    /// `-prune -o` idiom stays a false alarm), nor are `-path`, `-regex` and the like (they only narrow); a `-name`
    /// pattern with `\`, `$` or a backtick counts as matching. Upgrade when a live run shows a false alarm of these.
    fn find_reaches(&self, db: &str, tests: &[W], exec: bool, whole: bool) -> bool {
        let (mut names, mut kind, mut i) = (Vec::new(), None, 0);
        while i < tests.len() {
            let value = tests.get(i + 1).map(|(v, _)| v.as_str());
            match tests[i].0.as_str() {
                "-o" | "-or" | "!" | "-not" | "(" | ")" | "-prune" | "-exec" | "-execdir"
                | "-ok" | "-okdir" => return true,
                t @ ("-name" | "-iname") => {
                    names.extend(value.map(|v| (v, t == "-iname")));
                    i += 1;
                }
                "-type" => {
                    kind = value;
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        let (file_ok, dir_ok) =
            kind.map_or((true, exec), |k| (k.contains('f'), exec && k.contains('d')));
        // `-name` tests the last word of a path; every one of them must fit
        let fits = |cand: &str| {
            names.iter().all(|&(p, nocase)| {
                p.contains(['$', '`', '\\'])
                    || (nocase && p.contains('['))
                    || if nocase {
                        fnmatch(&p.to_lowercase(), &cand.to_lowercase())
                    } else {
                        fnmatch(p, cand)
                    }
            })
        };
        let leaf = db.rsplit('/').next().unwrap_or(db);
        (file_ok
            && ["", "-wal", "-shm", "-journal"].iter().any(|s| {
                self.root.join(format!("{db}{s}")).exists() && fits(&format!("{leaf}{s}"))
            }))
            || (dir_ok && (whole || db.split('/').rev().skip(1).any(fits)))
    }

    /// True when git, asked as `git clean` would be, lists none of the databases `found` (paths from the project folder)
    /// or their `-wal`, `-shm` and `-journal` files: it leaves them alone (`.gitignore`, `-e`, or tracked). Any doubt
    /// (not a repository, git missing or slow) is false, and so is `-ff`, which also takes nested repositories.
    fn git_leaves(&self, d: &Path, c: &Clean, found: &[String]) -> bool {
        if c.nested {
            return false;
        }
        let mut a = vec![
            "--literal-pathspecs".to_string(),
            "ls-files".into(),
            "--others".into(),
        ];
        if !c.x {
            a.push("--exclude-standard".into());
            if c.only_ignored {
                a.push("--ignored".into());
            }
        }
        a.extend(c.exclude.iter().map(|e| format!("--exclude={e}")));
        a.push("--".into());
        for h in found {
            for s in ["", "-wal", "-shm", "-journal"] {
                a.push(self.root.join(format!("{h}{s}")).display().to_string());
            }
        }
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        engine::exec(Path::new("git"), &a, None, Some(d), GIT_WAIT)
            .is_ok_and(|o| o.code == 0 && o.stdout.is_empty())
    }

    fn scan(&mut self, cmd: &str, dir: &mut PathBuf, depth: u8) {
        let mut cmds = commands(cmd, true);
        if cmds.iter().any(runs_script) {
            cmds = commands(cmd, false); // a here-document may be the script it runs: read its lines as commands
        }
        for c in cmds {
            for (w, g) in &c.writes {
                for p in self.paths(dir, w, *g) {
                    self.visit(&p, false);
                }
            }
            let ws = head(&c.words);
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
                            let c = clean_args(rest);
                            if !c.dry {
                                let found = self.fresh(|s| {
                                    if c.paths.is_empty() {
                                        s.walk(&d, 0);
                                    } else {
                                        s.all(&d, &c.paths, true);
                                    }
                                });
                                if !found.is_empty() && !self.git_leaves(&d, &c, &found) {
                                    self.keep(found);
                                }
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
        assert!(fnmatch("*.db", ".hidden.db")); // find's -name, unlike the shell, lets a star match a leading dot
        let words = |s: &str| {
            lex(s, true)
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

    #[test]
    fn braces_expand_like_a_shell_and_stay_bounded() {
        assert_eq!(
            expand("app.db{,-wal,-shm}"),
            ["app.db", "app.db-wal", "app.db-shm"]
        );
        assert_eq!(expand("{a,{b,c}}.db"), ["a.db", "b.db", "c.db"]);
        assert_eq!(expand("{a,b}{c,d}"), ["ac", "ad", "bc", "bd"]);
        assert_eq!(expand("plain"), ["plain"]);
        assert_eq!(expand("x{y}z"), ["x*z"]); // no comma: not an expansion, read as a star
        assert_eq!(expand("x{y,z"), ["x*"]); // never closed
        assert!(expand(&"{a,b}".repeat(12)).len() < 300);
        let root = project("braces");
        let app = "data/app.db".to_string();
        assert!(hits(&root, "rm data/app.{log,txt}").is_empty());
        assert!(hits(&root, "rm data/app.{log,db-wal}").contains(&app));
        assert!(hits(&root, "rm data/{idle,app}.db").contains(&app));
    }

    #[test]
    fn a_target_directory_is_where_the_files_go_and_only_what_is_there_is_overwritten() {
        let root = project("target");
        let app = "data/app.db".to_string();
        // lone/app.db is not SQLite and notes.txt is not in data: nothing in use is overwritten
        for cmd in [
            "cp -t lone data/app.db",
            "cp -t lone/ data/app.db",
            "cp -t backup data/app.db",
            "cp -t data notes.txt",
            "cp --target-directory=data notes.txt",
            "install -m 644 -t data notes.txt",
        ] {
            assert!(hits(&root, cmd).is_empty(), "{cmd:?}");
        }
        for cmd in [
            "cp -t data lone/app.db",
            "cp -tdata lone/app.db",
            "cp -rt data lone/app.db",
            "cp -at data lone/app.db",
            "cp lone/app.db -t data",
            "cp --target-directory=data lone/app.db",
            "cp --target-directory data lone/app.db",
            "cp --target data lone/app.db",
            "cp -v -- lone/app.db data/",
            "mv -t data lone/app.db",
            "mv -t lone data/app.db", // what mv moves away counts, as without -t
            "install -t data lone/app.db",
        ] {
            assert!(hits(&root, cmd).contains(&app), "{cmd:?}");
        }
        let w = |s: &str| (s.to_string(), false);
        assert_eq!(
            target_dir(&[w("-rt"), w("d"), w("a")]),
            (Some(w("d")), vec![w("a")])
        );
        assert_eq!(target_dir(&[w("a"), w("b")]), (None, vec![w("a"), w("b")]));
    }

    #[test]
    fn find_tests_that_cannot_reach_a_database_let_the_command_through() {
        let root = project("find");
        let (app, deep) = ("data/app.db".to_string(), "sub/deep/x.SQLITE".to_string());
        for cmd in [
            "find . -name .DS_Store -delete",
            "find . -name '*.pyc' -delete",
            "find data -iname '*.PYC' -delete",
            "find . -type f -name '*.orig' -exec rm {} +",
            "find . -type d -name __pycache__ -exec rm -rf {} +",
            "find . -type d -name zzz -exec mv {} /tmp \\;",
            "find ./ -type d -name zzz -exec rm -rf {} +",
            "find . -type d -delete", // a database is a file: rmdir of a folder that holds one fails
            "find . -type l -delete",
            "find . -name '*.db' -type d -delete",
            "find . -name 'app.dbx' -delete",
            "find data -name 'app.db' -type d -exec rm -rf {} +",
            "find . -maxdepth 1 -name '*.log' -delete",
            "find . -name 'app.d[a]' -delete",
            "find -L . -name zzz -delete",
        ] {
            assert!(
                hits(&root, cmd).is_empty(),
                "{cmd:?} -> {:?}",
                hits(&root, cmd)
            );
        }
        // a name test is on the last word of a path, case as asked, and it sees the -wal, -shm and -journal files too
        for cmd in [
            "find . -name '*.db' -delete",
            "find . -name 'app.db' -delete",
            "find . -iname 'APP.DB' -delete",
            "find . -name 'app.d?' -type f -delete",
            "find . -name 'app.db-wal' -delete",
            "find . -name '*-shm' -exec rm {} +",
            "find . -name '*.db' -a -type f -delete",
            "find data -type f -delete",
            "find data -delete",
            "find . -delete -name '*.pyc'", // the action runs before the test
            "find . -name '[a]pp.db' -delete",
            "find . -name 'app\\.db' -delete", // a pattern we cannot read counts as matching
            "find . -name \"$X\" -delete",
        ] {
            assert!(hits(&root, cmd).contains(&app), "{cmd:?}");
        }
        assert!(hits(&root, "find . -name 'x.SQLITE' -iname 'X.*' -delete").contains(&deep));
        assert!(!hits(&root, "find . -name 'x.sqlite' -delete").contains(&deep)); // -name is case sensitive
        // tests this parser does not judge: as if there were none (the common `-prune -o` idiom stays a false alarm)
        for cmd in [
            "find . -name '*.pyc' -o -name '*.db' -delete",
            "find . ! -name '*.db' -delete",
            "find . -not -name zzz -delete",
            "find . \\( -name a -o -name b \\) -delete",
            "find . -name node_modules -prune -o -name '*.pyc' -delete",
            "find . -path './data/*' -delete",
            "find . -regex '.*app.*' -delete",
            "find . -name zzz -exec echo {} \\; -delete",
        ] {
            assert!(hits(&root, cmd).contains(&app), "{cmd:?}");
        }
        // a folder test with rm -rf / mv takes the database along with the folder it is in
        assert_eq!(
            hits(&root, "find . -type d -name data -exec rm -rf {} +"),
            std::slice::from_ref(&app)
        );
        assert_eq!(
            hits(&root, "find . -type d -name deep -exec rm -rf {} +"),
            std::slice::from_ref(&deep)
        );
        assert_eq!(
            hits(&root, "find . -name sub -exec mv {} /tmp/x \\;"),
            std::slice::from_ref(&deep)
        );
        assert!(hits(&root, "find . -type d -exec rm -rf {} +").contains(&app));
        // a start point that holds the project folder, other than `.`, is not judged
        for start in [
            root.display().to_string(),
            "..".to_string(),
            "../proj".to_string(),
        ] {
            let got = hits(
                &root,
                &format!("find {start} -type d -name zzz -exec rm -rf {{}} +"),
            );
            assert!(got.contains(&app), "{start} -> {got:?}");
        }
    }

    #[test]
    fn here_document_text_is_not_a_command_unless_a_shell_or_script_reads_it() {
        let root = project("heredoc");
        let app = "data/app.db".to_string();
        for cmd in [
            "cat > reset.sh <<'EOF'\nrm data/app.db\nEOF",
            "cat > reset.sh <<EOF\nrm data/app.db\nEOF\nchmod +x reset.sh",
            "cat <<-EOF\n\trm data/app.db\n\tEOF\necho done",
            "cat <<A <<B\nrm data/app.db\nA\nrm data/app.db\nB",
            "cat <<EOF > notes.txt\nrm data/app.db\nEOF",
            "python3 <<EOF\nrm data/app.db\nEOF", // shortcut: not shell, so not read
            "grep x <<< 'rm data/app.db'",
        ] {
            assert!(hits(&root, cmd).is_empty(), "{cmd:?}");
        }
        for cmd in [
            "cat <<EOF\nhello\nEOF\nrm data/app.db",
            "bash <<EOF\nrm data/app.db\nEOF",
            "sh <<'EOF'\nrm data/app.db\nEOF",
            "sudo bash <<EOF\nrm data/app.db\nEOF",
            "cat <<EOF | sh\nrm data/app.db\nEOF",
            "cat > x.sh <<EOF\nrm data/app.db\nEOF\nsh x.sh",
            "cat > x.sh <<EOF\nrm data/app.db\nEOF\n./x.sh",
            "cat > x.sh <<EOF\nrm data/app.db\nEOF\nbash scripts/x.sh",
            "source /dev/stdin <<EOF\nrm data/app.db\nEOF",
            "cat <<EOF\nrm data/app.db\n", // no end word: not a here-document
            "echo $((1 << 2))\nrm data/app.db",
            "cat <<EOF\nrm data/app.db\nEOFX\n",
        ] {
            assert!(hits(&root, cmd).contains(&app), "{cmd:?}");
        }
    }

    /// `git init` in the project folder with this `.gitignore`.
    fn repo(name: &str, ignore: &str) -> PathBuf {
        let root = project(name);
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .status()
            .is_ok_and(|s| s.success());
        assert!(ok, "git init");
        fs::write(root.join(".gitignore"), ignore).unwrap();
        root
    }

    #[test]
    fn git_clean_is_denied_unless_git_itself_says_the_database_is_left_alone() {
        let app = "data/app.db".to_string();
        let deep = "sub/deep/x.SQLITE".to_string();
        // not a repository, no way to ask git: the deny of before
        let none = project("clean-none");
        assert!(hits(&none, "git clean -fd").contains(&app));
        assert!(hits(&none, "git clean -fd -e '*.db*'").contains(&app));
        // every file of both databases ignored
        let all = repo("clean-all", "*.db*\n*.SQLITE*\n");
        for cmd in [
            "git clean -fd",
            "git clean -f data",
            "git clean -fdq",
            "git -C data clean -fd",
            "git clean -fdn",
        ] {
            assert!(hits(&all, cmd).is_empty(), "{cmd:?}");
        }
        for cmd in [
            "git clean -fdx",
            "git clean -ffd",
            "git clean -fdf",
            "git clean -fdX",
            "git clean -f -d --force",
        ] {
            let got = hits(&all, cmd);
            // -x: the rules are not used; -ff: nested repositories go too; -X: only ignored files go
            assert!(
                got.contains(&app) && got.contains(&deep),
                "{cmd:?} -> {got:?}"
            );
        }
        // only the database file is ignored, not its -wal and -shm: git would take those
        let part = repo("clean-part", "*.db\n*.SQLITE\n");
        assert!(hits(&part, "git clean -fd").contains(&app));
        // -e names, in every spelling, also keep the -wal and -shm files
        let plain = repo("clean-e", "");
        for cmd in [
            "git clean -fdx -e '*.db*' -e '*.SQLITE*'",
            "git clean -fdx -e'*.db*' -e'*.SQLITE*'",
            "git clean -fdxe '*.db*' -e '*.SQLITE*'",
            "git clean -fdx --exclude '*.db*' --exclude='*.SQLITE*'",
            "git clean -fd -e '*.db*' -e '*.SQLITE*'",
            "git clean -e '*.db*' -e '*.SQLITE*' -fd",
        ] {
            let got = hits(&plain, cmd);
            assert!(got.is_empty(), "{cmd:?} -> {got:?}");
        }
        for cmd in [
            "git clean -fdx -e dev.db",
            "git clean -fdx -e '*.db' -e '*.SQLITE'",
            "git clean -fdx -e '*.db*'", // the deep one is not excluded
            "git clean -fd",
            "git clean -fdx",
        ] {
            assert!(!hits(&plain, cmd).is_empty(), "{cmd:?}");
        }
        assert!(hits(&plain, "git clean -fdx -e '*.db*'").contains(&deep));
        // -X takes only what the rules ignore: nothing is ignored here
        assert!(hits(&plain, "git clean -fdX").is_empty());
        assert!(hits(&all, "git clean -fdX -e zzz").contains(&app));
        // an -e pattern is not a path to clean
        assert!(hits(&plain, "git clean -fd -e '*.db*' -e '*.SQLITE*' notes.txt").is_empty());
        assert!(hits(&plain, "git clean -fd notes.txt data").contains(&app));
    }
}

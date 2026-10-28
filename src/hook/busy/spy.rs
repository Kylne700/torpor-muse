//! Judges the commands of one line: collects the busy databases they would remove, move or overwrite.

use super::args::{Clean, base, clean_args, head, runs_script, split, target_dir};
use super::glob::{expand, fnmatch, glob_match};
use super::lexer::{W, commands};
use super::{busy, is_db_name};
use crate::engine;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEPTH: usize = 12;
/// How long the one `git` call of a `git clean` may take; the hook has 6 s and its engine call may have used some of it.
const GIT_WAIT: Duration = Duration::from_millis(1500);

pub(super) struct Spy {
    /// The project folder, as the kernel names it; a database outside it is none of our business.
    pub(super) root: PathBuf,
    pub(super) left: usize,
    pub(super) hits: Vec<String>,
}

impl Spy {
    /// The paths a word names: itself, or the entries a glob matches (read from the folders, within `LOOK`).
    /// Nothing for a word we cannot resolve (`$VAR`, `$(..)`, `~`).
    pub(super) fn paths(&mut self, dir: &Path, w: &str, glob: bool) -> Vec<PathBuf> {
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

    pub(super) fn scan(&mut self, cmd: &str, dir: &mut PathBuf, depth: u8) {
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

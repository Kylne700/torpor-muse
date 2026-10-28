//! Where a word of a command points and whether Torpor must stop a delete or move there. Read as text: `~`, `$HOME` and
//! `${HOME}` expanded, `..` resolved, no link followed.

use crate::hook::busy::glob::{expand, glob_match};
use std::path::{Component, Path, PathBuf};

/// The folders the rules compare with.
pub(super) struct Env {
    /// The project folder as the engine names it, and as the kernel does (a link resolved) when that differs.
    pub(super) root: Vec<PathBuf>,
    pub(super) home: Option<PathBuf>,
    /// `$TMPDIR` when it may be trusted (`usable_tmp`).
    pub(super) tmp: Option<PathBuf>,
    /// Torpor's store (`engine::home_dir`).
    pub(super) store: Option<PathBuf>,
}

/// What a word names.
#[derive(Debug, PartialEq)]
pub(super) enum Target {
    Path(PathBuf),
    /// Any path whose text starts with `.0`: a `$VAR`, `$(..)` or glob cut the word short. `.1`: the whole word when
    /// only glob characters cut it, which tells the paths it can name.
    Prefix(String, Option<String>),
}

/// `/a/./b/../c` as `/a/c`, without asking the disk.
pub(super) fn norm(p: &Path) -> PathBuf {
    let mut out = PathBuf::from("/");
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(n) => out.push(n),
            _ => {}
        }
    }
    out
}

/// `$TMPDIR` counts only when it is not `/` and neither the home folder nor a folder that holds it.
pub(super) fn usable_tmp(tmp: PathBuf, home: Option<&Path>) -> Option<PathBuf> {
    (tmp != Path::new("/") && !home.is_some_and(|h| h.starts_with(&tmp))).then_some(tmp)
}

/// What word `w` (`glob`: it holds an unquoted glob character) names when run in `dir` (None: after a `cd` we could not
/// follow). None = we cannot tell: no fixed start (`$X/..`, `$(..)`, `~user`), or relative in an unknown folder.
pub(super) fn resolve(w: &str, glob: bool, dir: Option<&Path>, env: &Env) -> Option<Target> {
    let rest = if w == "~" || w.starts_with("~/") {
        Some(&w[1..])
    } else if let Some(r) = w.strip_prefix("${HOME}") {
        Some(r)
    } else if w.starts_with('~') {
        return None; // `~user`: another user's home, which is not looked up
    } else {
        let r = w.strip_prefix("$HOME");
        r.filter(|r| !r.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
    };
    let s = match rest {
        Some(r) => format!("{}{r}", env.home.as_ref()?.display()),
        None => w.to_string(),
    };
    let cut = s.find(|c: char| c == '$' || c == '`' || (glob && "*?[{".contains(c)));
    if s.is_empty() || cut == Some(0) {
        return None;
    }
    let base = if s.starts_with('/') {
        String::new()
    } else {
        format!("{}/", dir?.display())
    };
    let full = format!("{base}{s}");
    let Some(cut) = cut.map(|c| base.len() + c) else {
        return Some(Target::Path(norm(Path::new(&full))));
    };
    // the folders in front are resolved; the last name, cut short, stays as typed
    let (front, last) = full[..cut].rsplit_once('/')?;
    let front = norm(Path::new(if front.is_empty() { "/" } else { front }));
    let front = front.to_string_lossy();
    let front = front.trim_end_matches('/');
    let pat = !s.contains(['$', '`']);
    let pat = pat.then(|| format!("{front}/{}", &full[cut - last.len()..]));
    Some(Target::Prefix(format!("{front}/{last}"), pat))
}

/// Whether glob `pat` (a full path) can name `p`: name by name, as the shell matches.
fn names(pat: &str, p: &Path) -> bool {
    let pat: Vec<&str> = pat.split('/').filter(|n| !n.is_empty()).collect();
    let p: Vec<_> = p.iter().skip(1).map(|n| n.to_string_lossy()).collect();
    pat.len() == p.len()
        && pat
            .iter()
            .zip(&p)
            .all(|(g, n)| expand(g).iter().any(|g| glob_match(g, n)))
}

/// Every path whose text starts with `s` lies below folder `d`.
fn all_below(s: &str, d: &Path) -> bool {
    let d = d.to_string_lossy();
    s.strip_prefix(d.trim_end_matches('/'))
        .is_some_and(|r| r.starts_with('/'))
}

/// True when a delete or move of `t` must be stopped. The rules, first match wins: (1) inside the project: left to
/// the engine; (2) `/`, the home folder, a folder that holds it, Torpor's store, a folder that holds it or anything in
/// it: stopped; (3) below a usable `$TMPDIR`: allowed; (4) below the home folder: stopped; (5) below /tmp or /var/tmp:
/// allowed; (6) anywhere else: stopped. A cut-short target is left to the engine when every path it could be is inside
/// the project, stopped when any of them is (2), else judged by the folder all of them lie in.
pub(super) fn stopped(t: &Target, env: &Env) -> bool {
    let temp = [Path::new("/tmp"), Path::new("/var/tmp")];
    match t {
        Target::Path(p) => {
            let holds = |x: &Option<PathBuf>| x.as_ref().is_some_and(|x| x.starts_with(p));
            let below = |d: &Path| p.starts_with(d) && p != d;
            if env.root.iter().any(|r| p.starts_with(r)) {
                return false;
            }
            if p == Path::new("/")
                || holds(&env.home)
                || holds(&env.store)
                || env.store.as_ref().is_some_and(|s| p.starts_with(s))
            {
                return true;
            }
            if env.tmp.as_deref().is_some_and(below) {
                return false;
            }
            env.home.as_ref().is_some_and(|h| p.starts_with(h)) || !temp.into_iter().any(below)
        }
        Target::Prefix(s, pat) => {
            // it could be `x` or a folder that holds it
            let could = |x: &Option<PathBuf>| {
                x.as_ref().is_some_and(|x| {
                    x.ancestors().any(|a| match pat {
                        Some(g) => names(g, a),
                        None => a.to_string_lossy().starts_with(s.as_str()),
                    })
                })
            };
            let below = |d: &Path| all_below(s, d);
            if env.root.iter().any(|r| below(r)) {
                return false;
            }
            if could(&env.home) || could(&env.store) || env.store.as_deref().is_some_and(below) {
                return true;
            }
            if env.tmp.as_deref().is_some_and(below) {
                return false;
            }
            env.home.as_deref().is_some_and(below) || !temp.into_iter().any(below)
        }
    }
}

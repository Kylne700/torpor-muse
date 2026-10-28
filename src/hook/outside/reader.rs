//! Reads a Bash command line for what it would delete or move: `rm`, `rmdir`, `unlink`, `shred`, `truncate`, what `mv`
//! moves, `dd of=`, `find -delete` / `-exec rm`, `xargs rm`; also inside `sh -c`, `eval` and `npm run` scripts, after
//! `&&`, `;` and `|`, and from the folder an earlier `cd` went to.
//! shortcut: a `cd` inside `( )` goes on into the commands after it, a script run by name (`./clean.sh`, `make`), a
//! program (`cargo test`, `python -c`), a wrapper option with a value (`sudo -u x rm`), `cp`, `rsync --delete` and a `>`
//! onto a file are not read; upgrade when a live run shows one.

use super::place::{Env, Target, resolve};
use crate::hook::busy::args::{base, head, runs_script, split, target_dir};
use crate::hook::busy::lexer::{W, commands};
use serde_json::Value;
use std::path::{Path, PathBuf};

const DELETE: &[&str] = &["rm", "rmdir", "unlink", "shred"];
/// `npm run NAME` runs these from package.json, in this order.
const RUNS: [&str; 3] = ["pre", "", "post"];

/// What `cmd`, run in folder `dir`, would delete or move.
pub(super) fn targets(cmd: &str, dir: PathBuf, env: &Env) -> Vec<Target> {
    let mut out = Vec::new();
    scan(cmd, &mut Some(dir), 0, env, &mut out);
    out
}

fn ops(args: &[W]) -> Vec<W> {
    split(args).1.into_iter().cloned().collect()
}

fn scan(cmd: &str, dir: &mut Option<PathBuf>, depth: u8, env: &Env, out: &mut Vec<Target>) {
    let mut cmds = commands(cmd, true);
    if cmds.iter().any(runs_script) {
        cmds = commands(cmd, false); // a here-document may be the script it runs: read its lines as commands
    }
    let mut prev: Vec<W> = Vec::new(); // the command in front: what `xargs` reads
    for c in cmds {
        let ws = head(&c.words);
        let Some((v, _)) = ws.first() else {
            prev.clear(); // `&&`, `||`: no pipe from the command in front
            continue;
        };
        let (verb, args) = (base(v), &ws[1..]);
        // `words` name the paths; `deep` name folders whose contents go
        let (mut words, mut deep) = (Vec::new(), Vec::new());
        match verb {
            v if DELETE.contains(&v) => words = ops(args),
            "mv" => {
                let (to, rest) = target_dir(args);
                words = ops(&rest);
                if to.is_none() {
                    words.pop(); // where they go
                }
            }
            "truncate" => words = truncated(args),
            "dd" => {
                let of = args
                    .iter()
                    .filter_map(|(w, g)| Some((w.strip_prefix("of=")?, *g)));
                let real =
                    of.filter(|(p, _)| !matches!(*p, "/dev/null" | "/dev/stdout" | "/dev/stderr"));
                words = real.map(|(p, g)| (p.to_string(), g)).collect();
            }
            "find" if kills(args) => deep = starts(args),
            "xargs" => (words, deep) = xargs(args, &prev),
            "cd" | "pushd" => *dir = cd(args, dir.as_deref(), env),
            "popd" => *dir = None,
            "sh" | "bash" | "zsh" | "dash" | "ksh" if depth < 2 => {
                let (opts, script) = split(args);
                if opts.iter().any(|o| !o.starts_with("--") && o.contains('c'))
                    && let Some((s, _)) = script.first()
                {
                    scan(s, &mut dir.clone(), depth + 1, env, out);
                }
            }
            "eval" if depth < 2 => {
                let s: Vec<&str> = args.iter().map(|(w, _)| w.as_str()).collect();
                scan(&s.join(" "), dir, depth + 1, env, out);
            }
            "npm" | "pnpm" | "yarn" | "bun" if depth < 2 => {
                for s in scripts(args, dir.as_deref()) {
                    scan(&s, &mut dir.clone(), depth + 1, env, out);
                }
            }
            _ => {}
        }
        let at = dir.as_deref();
        out.extend(words.iter().filter_map(|(w, g)| resolve(w, *g, at, env)));
        out.extend(
            deep.iter()
                .filter_map(|(w, g)| match resolve(w, *g, at, env)? {
                    Target::Path(p) => {
                        let p = p.to_string_lossy();
                        Some(Target::Prefix(
                            format!("{}/", p.trim_end_matches('/')),
                            None,
                        ))
                    }
                    t => Some(t),
                }),
        );
        prev = ws.to_vec();
    }
}

/// `truncate`'s files: its operands, not the values of `-s`, `-r`, `--size` and `--reference`.
fn truncated(args: &[W]) -> Vec<W> {
    let (mut out, mut it) = (Vec::new(), args.iter());
    while let Some(a) = it.next() {
        if matches!(a.0.as_str(), "-s" | "-r" | "--size" | "--reference") {
            it.next();
        } else if !a.0.starts_with('-') || a.0 == "-" {
            out.push(a.clone());
        }
    }
    out
}

/// A `find` that removes what it finds: `-delete`, or `-exec`/`-ok` of `rm`, `rmdir`, `unlink`, `shred` or `mv`.
fn kills(args: &[W]) -> bool {
    args.iter().enumerate().any(|(i, (w, _))| {
        w == "-delete"
            || (matches!(w.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir")
                && args
                    .get(i + 1)
                    .is_some_and(|(x, _)| DELETE.contains(&base(x)) || base(x) == "mv"))
    })
}

/// The folders `find` starts from (none written: `.`).
fn starts(args: &[W]) -> Vec<W> {
    let skip = args
        .iter()
        .take_while(|(w, _)| matches!(w.as_str(), "-H" | "-L" | "-P"))
        .count();
    let args = &args[skip..];
    let n = args
        .iter()
        .position(|(w, _)| w.starts_with(['-', '(', '!']))
        .unwrap_or(args.len());
    if n == 0 {
        vec![(".".into(), false)]
    } else {
        args[..n].to_vec()
    }
}

/// What `xargs rm ...` removes: its own operands, and what the command in front prints when we can tell (what `find`
/// finds, the words of `echo`). (paths, folders whose contents go)
fn xargs(args: &[W], prev: &[W]) -> (Vec<W>, Vec<W>) {
    let mut i = 0;
    while let Some((w, _)) = args
        .get(i)
        .filter(|(w, _)| w.len() > 1 && w.starts_with('-'))
    {
        let value = matches!(
            w.as_str(),
            "-a" | "-d" | "-E" | "-I" | "-L" | "-n" | "-P" | "-s"
        );
        i += if value { 2 } else { 1 };
    }
    let run = args.get(i..).unwrap_or(&[]);
    if !run.first().is_some_and(|(v, _)| DELETE.contains(&base(v))) {
        return (Vec::new(), Vec::new());
    }
    let mut words = ops(&run[1..]);
    match prev.first().map(|(v, _)| base(v)) {
        Some("find") => return (words, starts(&prev[1..])),
        Some("echo") => words.extend(ops(&prev[1..])),
        _ => {}
    }
    (words, Vec::new())
}

/// The folder after `cd ARGS` in `dir`; None when we cannot tell (`cd -`, `cd $X`).
fn cd(args: &[W], dir: Option<&Path>, env: &Env) -> Option<PathBuf> {
    match split(args).1.first() {
        None => env.home.clone(),
        Some((w, _)) if w == "-" => None,
        Some((w, g)) => match resolve(w, *g, dir, env)? {
            Target::Path(p) => Some(p),
            Target::Prefix(..) => None,
        },
    }
}

/// The scripts `npm run NAME` (`run-script`; also pnpm, yarn, bun) runs from `dir/package.json`.
fn scripts(args: &[W], dir: Option<&Path>) -> Vec<String> {
    let ops = split(args).1;
    let (Some(dir), [run, name, ..]) = (dir, ops.as_slice()) else {
        return Vec::new();
    };
    let file = dir.join("package.json");
    // a FIFO would block the read; a huge file is not a package.json
    if !matches!(run.0.as_str(), "run" | "run-script")
        || !std::fs::metadata(&file).is_ok_and(|m| m.is_file() && m.len() <= 1 << 20)
    {
        return Vec::new();
    }
    let Ok(Value::Object(pkg)) = std::fs::read(&file)
        .map_err(|_| ())
        .and_then(|b| serde_json::from_slice::<Value>(&b).map_err(|_| ()))
    else {
        return Vec::new();
    };
    RUNS.iter()
        .filter_map(|p| pkg.get("scripts")?.get(format!("{p}{}", name.0))?.as_str())
        .map(str::to_owned)
        .collect()
}

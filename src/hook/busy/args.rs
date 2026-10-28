//! Reads the arguments of one command: which word is the command, options and operands, `-t DIR`, and the flags of `git clean`.

use super::lexer::{Cmd, W};

/// Words that only run the next one: skipped, with the `-x` options right after them, to find the real command.
const WRAP: &[&str] = &[
    "sudo", "doas", "command", "builtin", "exec", "env", "nohup", "time", "nice", "ionice",
    "stdbuf", "setsid", "if", "then", "else", "elif", "do", "while", "until", "!", "{", "}",
];

pub(super) fn base(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or(w)
}

pub(super) fn is_assign(w: &str) -> bool {
    w.split_once('=').is_some_and(|(k, _)| {
        k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !k.is_empty()
            && !k.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// The words from the command a list runs on: without the assignments and wrappers (`sudo`, `env`, ...) and their
/// options in front.
pub(super) fn head(mut ws: &[W]) -> &[W] {
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
pub(super) fn runs_script(c: &Cmd) -> bool {
    head(&c.words).first().is_some_and(|(v, _)| {
        v.contains('/')
            || matches!(
                v.as_str(),
                "." | "source" | "eval" | "sh" | "bash" | "zsh" | "dash" | "ksh"
            )
    })
}

/// Options (`-x`, `--long`, up to a `--`) and operands of a command's arguments.
pub(super) fn split(args: &[W]) -> (Vec<&str>, Vec<&W>) {
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
pub(super) fn target_dir(args: &[W]) -> (Option<W>, Vec<W>) {
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
pub(super) struct Clean<'a> {
    /// `-n`, `--dry-run`: nothing is removed.
    pub(super) dry: bool,
    /// `-x`: files the ignore rules name are removed too (not the ones `-e` names).
    pub(super) x: bool,
    /// `-X`: only files the ignore rules name are removed.
    pub(super) only_ignored: bool,
    /// `-f` twice: nested repositories are removed too.
    pub(super) nested: bool,
    /// The `-e` and `--exclude` patterns.
    pub(super) exclude: Vec<String>,
    /// The paths it was told to clean (none: the whole folder).
    pub(super) paths: Vec<&'a W>,
}

pub(super) fn clean_args(args: &[W]) -> Clean<'_> {
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

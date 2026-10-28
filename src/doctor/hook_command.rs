//! Reads the command line `init` wrote into a hook: its words, whether it is ours, its vault and engine, the probe's copy.

use crate::util::quote;
use std::path::{Path, PathBuf};

/// Words of a simple command line: quotes and backslashes, no expansion. None when a quote is left open.
pub fn words(s: &str) -> Option<Vec<String>> {
    let (mut out, mut cur, mut open) = (Vec::new(), String::new(), false);
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            '\'' => {
                open = true;
                loop {
                    match it.next()? {
                        '\'' => break,
                        ch => cur.push(ch),
                    }
                }
            }
            '"' => {
                open = true;
                loop {
                    match it.next()? {
                        '"' => break,
                        '\\' => match it.next()? {
                            n @ ('"' | '\\' | '$' | '`') => cur.push(n),
                            n => cur.extend(['\\', n]),
                        },
                        ch => cur.push(ch),
                    }
                }
            }
            '\\' => {
                open = true;
                cur.push(it.next()?);
            }
            c if c.is_whitespace() => {
                if open {
                    out.push(std::mem::take(&mut cur));
                    open = false;
                }
            }
            c => {
                open = true;
                cur.push(c);
            }
        }
    }
    if open {
        out.push(cur);
    }
    Some(out)
}

/// A handler that runs `.../torpor-muse hook <Event>`, which is what `init` writes.
pub fn is_ours(command: &str) -> bool {
    words(command).is_some_and(|w| {
        w.len() >= 3
            && w[w.len() - 2] == "hook"
            && Path::new(&w[w.len() - 3])
                .file_name()
                .is_some_and(|n| n == "torpor-muse")
    })
}

/// The `TORPOR_HOME=` the generated command carries (an empty value counts as none, like the engine does).
pub fn vault_of(command: &str) -> Option<PathBuf> {
    words(command)?
        .iter()
        .find_map(|x| x.strip_prefix("TORPOR_HOME="))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn tool_of(command: &str, name: &str) -> Option<PathBuf> {
    let pre = format!("{name}=");
    words(command)?
        .iter()
        .find_map(|x| x.strip_prefix(&pre))
        .map(PathBuf::from)
}

/// The generated command with its vault and its last word (the event) swapped: the probe must not touch the real vault.
pub fn probe_command(command: &str, vault: &Path, event: &str) -> Option<String> {
    let mut w = words(command)?;
    let v = format!("TORPOR_HOME={}", vault.display());
    if let Some(i) = w.iter().position(|x| x.starts_with("TORPOR_HOME=")) {
        w[i] = v;
    } else if w.first().map(String::as_str) == Some("env") {
        w.insert(1, v);
    } else {
        w.insert(0, v);
        w.insert(0, "env".to_string());
    }
    *w.last_mut()? = event.to_string();
    Some(w.iter().map(|x| quote(x)).collect::<Vec<_>>().join(" "))
}

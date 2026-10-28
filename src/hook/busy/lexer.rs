//! Reads a Bash command line into commands and words roughly as a shell does: quotes, separators, redirects, here-documents.

/// A word and whether it holds an unquoted glob character (`*`, `?`, `[`, `{`, `}`).
pub(super) type W = (String, bool);

pub(super) enum Tok {
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
pub(super) fn lex(s: &str, docs: bool) -> Vec<Tok> {
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
pub(super) struct Cmd {
    pub(super) words: Vec<W>,
    /// Targets of `>`, `>>`, `>|` and `&>`.
    pub(super) writes: Vec<W>,
}

pub(super) fn commands(s: &str, docs: bool) -> Vec<Cmd> {
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

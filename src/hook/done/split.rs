//! Reads a Bash command line into segments and the separators between them: only what the done check needs (quotes,
//! escapes, `&&` `;` newlines, comments, redirects). Anything else that changes an exit code makes the line `murky`.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Sep {
    And,
    Semi,
}

pub(super) struct Seg {
    /// The words of the segment, unquoted, without redirects.
    pub(super) words: Vec<String>,
    /// The segment as written.
    pub(super) text: String,
}

pub(super) struct Chain {
    pub(super) segs: Vec<Seg>,
    /// `seps[i]` joins `segs[i]` and `segs[i + 1]`.
    pub(super) seps: Vec<Sep>,
    /// The exit code of the line cannot be traced to one segment: a pipe, `||`, `&`, a group, a substitution, a shell
    /// keyword, a here-document, an unclosed quote, a line cut at `MAX` characters.
    pub(super) murky: bool,
}

const MAX: usize = 4000;
const KEYWORDS: [&str; 19] = [
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "select", "function", "!", "{", "}", "[[", "coproc",
];

#[derive(Default)]
struct Build {
    segs: Vec<Seg>,
    seps: Vec<Sep>,
    /// The separator in front of the segment that comes next.
    pending: Option<Sep>,
    murky: bool,
    words: Vec<String>,
    word: String,
    inword: bool,
    /// The word had a quote or an escape in it: it is not a redirect.
    plain: bool,
    /// The last word was a bare redirect operator (`>`, `2>`): the next word is its target.
    skip: bool,
    /// Where the segment being read starts.
    from: usize,
}

/// `>file`, `2>&1`, `&>f`, `<in` and the bare operators; `Some(true)` for a bare operator (a target word follows).
fn redirect(w: &str) -> Option<bool> {
    let rest = w.trim_start_matches(|c: char| c.is_ascii_digit());
    let rest = rest.strip_prefix('&').unwrap_or(rest);
    let target = rest.trim_start_matches(['>', '<']);
    (rest != target).then(|| target.is_empty() && !w.contains('&'))
}

impl Build {
    fn end_word(&mut self) {
        if !self.inword {
            return;
        }
        let w = std::mem::take(&mut self.word);
        self.inword = false;
        let plain = std::mem::take(&mut self.plain);
        if std::mem::take(&mut self.skip) {
            return;
        }
        match redirect(&w).filter(|_| !plain) {
            Some(bare) => self.skip = bare,
            None => self.words.push(w),
        }
    }

    /// A separator at `at` ends the segment.
    fn sep(&mut self, sep: Sep, at: usize, line: &[char]) {
        self.end_word();
        self.skip = false;
        if self.words.is_empty() {
            // a newline after `&&` goes on with the list; `a && && b` is not a line worth reading
            if self.pending.is_some() && sep != Sep::Semi {
                self.murky = true;
                self.pending = Some(sep);
            }
            return;
        }
        self.murky |= KEYWORDS.contains(&self.words[0].as_str());
        if !self.segs.is_empty() {
            self.seps.push(self.pending.take().unwrap_or(Sep::Semi));
        }
        self.pending = Some(sep);
        self.segs.push(Seg {
            words: std::mem::take(&mut self.words),
            text: line[self.from.min(at)..at].iter().collect(),
        });
    }
}

/// The text of a quoted string that starts at `c[i]` and the index after its closing quote; `None` if it is not closed.
fn quoted(c: &[char], i: usize) -> Option<(String, usize)> {
    let q = c[i];
    let (mut j, mut got) = (i + 1, String::new());
    while j < c.len() && c[j] != q {
        if q == '"' && c[j] == '\\' && j + 1 < c.len() {
            j += 1;
            if !matches!(c[j], '"' | '\\' | '$' | '`' | '\n') {
                got.push('\\');
            }
            if c[j] != '\n' {
                got.push(c[j]);
            }
        } else {
            got.push(c[j]);
        }
        j += 1;
    }
    (j < c.len()).then_some((got, j + 1))
}

/// Never fails: a line it cannot read (an unclosed quote) is one murky segment of its words.
pub(super) fn split(line: &str) -> Chain {
    let c: Vec<char> = line.chars().take(MAX + 1).collect();
    let mut b = Build {
        murky: c.len() > MAX,
        ..Build::default()
    };
    let c = &c[..c.len().min(MAX)];
    let mut i = 0;
    while i < c.len() {
        let next = c.get(i + 1).copied();
        let prev = i.checked_sub(1).map(|p| c[p]);
        // a separator ends the segment: (kind, the characters it takes, whether it makes the line murky)
        let sep = match c[i] {
            '\'' | '"' => {
                let Some((text, after)) = quoted(c, i) else {
                    return unreadable(line);
                };
                b.word.push_str(&text);
                (b.inword, b.plain, i) = (true, true, after);
                continue;
            }
            '\\' => {
                if let Some(x) = next.filter(|&x| x != '\n') {
                    b.word.push(x);
                    (b.inword, b.plain) = (true, true);
                }
                i += if next.is_some() { 2 } else { 1 };
                continue;
            }
            ' ' | '\t' | '\r' => {
                b.end_word();
                i += 1;
                continue;
            }
            '#' if !b.inword => {
                while i < c.len() && c[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            '\n' | ';' => (Sep::Semi, 1, false),
            '&' if next == Some('&') => (Sep::And, 2, false),
            // `2>&1`, `>&2`, `&>file`: part of a redirect word
            '&' if matches!(prev, Some('>' | '<')) || next == Some('>') => {
                b.word.push('&');
                b.inword = true;
                i += 1;
                continue;
            }
            '|' => (
                Sep::Semi,
                if matches!(next, Some('|' | '&')) {
                    2
                } else {
                    1
                },
                true,
            ),
            '&' | '(' | ')' | '`' => (Sep::Semi, 1, true),
            '<' if next == Some('<') => {
                b.murky = true;
                break;
            }
            ch => {
                // `npm test>out.log`: the redirect starts a word of its own, unless it is `2>` and the like
                let mid = matches!(ch, '>' | '<')
                    && b.inword
                    && !b.word.chars().all(|x| x.is_ascii_digit())
                    && !matches!(prev, Some('>' | '<' | '&'));
                if mid {
                    b.end_word();
                }
                b.word.push(ch);
                b.inword = true;
                i += 1;
                continue;
            }
        };
        b.murky |= sep.2;
        b.sep(sep.0, i, c);
        i += sep.1;
        b.from = i;
    }
    b.sep(Sep::Semi, c.len(), c);
    Chain {
        segs: b.segs,
        seps: b.seps,
        murky: b.murky,
    }
}

fn unreadable(line: &str) -> Chain {
    Chain {
        segs: vec![Seg {
            words: line.split_whitespace().map(str::to_owned).collect(),
            text: line.to_owned(),
        }],
        seps: Vec::new(),
        murky: true,
    }
}

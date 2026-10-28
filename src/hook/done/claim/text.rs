//! A message cut into sentences of lowercase words. Code blocks, quoted lines and quoted words are gone, a code span is
//! named by what it runs (`testcmd`, `buildcmd`, else `codespan`), a question is dropped.

use crate::hook::done::commands::listed;

pub(super) struct Sentence {
    pub(super) words: Vec<String>,
    /// The sentence as the reader saw it (code spans named): for the tests and the replay, never sent anywhere.
    #[cfg(test)]
    pub(super) shown: String,
}

pub(super) fn sentences(said: &str) -> Vec<Sentence> {
    let mut out = Vec::new();
    let mut fenced = false;
    for line in said.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced || t.starts_with('>') {
            continue;
        }
        for (text, question) in split(&unquoted(&named(bare(t)))) {
            let words = words(&text);
            if !question && !words.is_empty() {
                out.push(Sentence {
                    words,
                    #[cfg(test)]
                    shown: text.trim().to_owned(),
                });
            }
        }
    }
    out
}

/// The line without its list or heading marker; an open task (`[ ]`) is a wish, not a report.
fn bare(line: &str) -> &str {
    let t = line.trim_start_matches(['#', '-', '*', '+', '•', ' ', '\t']);
    if t.starts_with("[ ]") {
        return "";
    }
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    match t[digits..].strip_prefix(['.', ')']) {
        Some(rest) if digits > 0 && rest.starts_with(' ') => rest,
        _ => t,
    }
}

/// Emphasis marks gone, every code span named.
fn named(line: &str) -> String {
    let line = line.replace('*', "").replace("__", "");
    let (mut out, mut rest) = (String::new(), line.as_str());
    while let Some(i) = rest.find('`') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('`') {
            Some(j) => {
                out.push_str(match listed(&after[..j]) {
                    (true, _) => " testcmd ",
                    (_, true) => " buildcmd ",
                    _ => " codespan ",
                });
                rest = &after[j + 1..];
            }
            None => rest = after, // a lone backtick: the rest stays text
        }
    }
    out.push_str(rest);
    out
}

/// Without what is between quotation marks; a quote that is never closed takes the rest of the sentence with it.
fn unquoted(s: &str) -> String {
    let (mut out, mut close) = (String::new(), None);
    for c in s.chars() {
        match (close, c) {
            (None, '"') => close = Some('"'),
            (None, '“') => close = Some('”'),
            (Some(q), c) if c == q => {
                close = None;
                out.push(' ');
            }
            (Some(_), _) => {}
            (None, c) => out.push(c),
        }
    }
    out
}

/// Sentences of one line, each with whether it is a question.
fn split(line: &str) -> Vec<(String, bool)> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        cur.push(c);
        if matches!(c, '.' | '!' | '?') && chars.peek().is_none_or(|n| n.is_whitespace()) {
            out.push((std::mem::take(&mut cur), c == '?'));
        }
    }
    if !cur.trim().is_empty() {
        out.push((cur, false));
    }
    out
}

const INNER: &str = "'-/+_.%#@";

/// Lowercase words. `:` is a word, a clause break (`;` `—` `|` a lone dash) is `;`, a check mark is `✅`, a cross `❌`.
fn words(s: &str) -> Vec<String> {
    let (mut out, mut cur) = (Vec::new(), String::new());
    for c in s.chars() {
        if c.is_alphanumeric() || INNER.contains(c) || c == '’' {
            cur.push(if c == '’' { '\'' } else { c });
            continue;
        }
        flush(&mut cur, &mut out);
        match c {
            ':' => out.push(":".into()),
            ';' | '—' | '–' | '|' => out.push(";".into()),
            '✅' | '✔' | '✓' => out.push("✅".into()),
            '❌' | '✗' | '✘' | '⚠' => out.push("❌".into()),
            _ => {}
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn flush(cur: &mut String, out: &mut Vec<String>) {
    let raw = std::mem::take(cur);
    let word = raw.trim_matches(|c| INNER.contains(c)).to_lowercase();
    if !word.is_empty() {
        out.push(word);
    } else if !raw.is_empty() && raw.chars().all(|c| c == '-') {
        out.push(";".into());
    }
}

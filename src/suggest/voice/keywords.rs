//! The project's own words that go along with the clip, so the transcriber spells them right.

use super::KEYWORDS_MAX;
use crate::suggest::repo;
use std::collections::BTreeMap;
use std::path::Path;

/// The code files whose names and URL paths give the keywords.
const CODE: [&str; 18] = [
    "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "rb", "php", "go", "java", "rs", "cs", "vue",
    "html", "ejs", "hbs", "pug",
];

/// Lowercase ASCII words of 3 to 15 letters; camelCase and every other character are word breaks.
/// shortcut: ASCII letters only (the plan is English), so a word with an accent is cut at the accent. Upgrade with the languages.
pub(super) fn words(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut prev_lower) = (Vec::new(), String::new(), false);
    for c in s.chars() {
        if c.is_ascii_alphabetic() {
            if c.is_ascii_uppercase() && prev_lower {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = c.is_ascii_lowercase();
            cur.push(c.to_ascii_lowercase());
        } else {
            prev_lower = false;
            out.push(std::mem::take(&mut cur));
        }
    }
    out.push(cur);
    out.retain(|w| (3..=15).contains(&w.len()));
    out
}

/// The string literals that start with a slash: a quote, `/`, and on to the same quote (a template literal stops
/// at its first `${`, any literal at the end of the line). These are the URL paths of the code.
pub(super) fn url_paths(text: &str) -> Vec<&str> {
    let (b, mut out, mut i) = (text.as_bytes(), Vec::new(), 0);
    while i + 1 < b.len() {
        if matches!(b[i], b'\'' | b'"' | b'`') && b[i + 1] == b'/' {
            let rest = &text[i + 1..];
            let end = rest
                .bytes()
                .zip(0..)
                .find(|&(c, j)| {
                    c == b[i]
                        || c == b'\n'
                        || (b[i] == b'`' && rest.as_bytes()[j..].starts_with(b"${"))
                })
                .map_or(rest.len(), |(_, j)| j);
            out.push(repo::cut(&rest[..end], 120));
            i += 1 + end;
        }
        i += 1;
    }
    out
}

/// Up to 24 words of the project's own vocabulary, for the transcription to favour: the words in the names of its
/// code files and in the URL paths written in its code. The most repeated first, then alphabetical, so the same
/// project always gives the same list. Everything read is what the writer reads too (listed, redacted, capped).
/// shortcut: no stop-word list (`server` and `index` count like any word), no care for plurals, and test titles,
/// saved checks and READMEs are not read. Upgrade when a measurement shows it matters.
pub(super) fn keywords(root: &Path) -> Vec<String> {
    let synthetic = repo::is_synthetic(root);
    let mut count: BTreeMap<String, u32> = BTreeMap::new();
    for rel in repo::allowed_files(root) {
        if !CODE.contains(&rel.rsplit('.').next().unwrap_or("")) {
            continue;
        }
        let stem = rel
            .rsplit('/')
            .next()
            .unwrap_or(&rel)
            .split('.')
            .next()
            .unwrap_or("");
        let text = repo::read_capped(root, &rel, synthetic).unwrap_or_default();
        let text = text.replace(repo::MASK, " ");
        for w in words(stem)
            .into_iter()
            .chain(url_paths(&text).into_iter().flat_map(words))
        {
            *count.entry(w).or_default() += 1;
        }
    }
    let mut ranked: Vec<(String, u32)> = count.into_iter().collect();
    ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n)); // stable: equal counts stay alphabetical
    ranked
        .into_iter()
        .take(KEYWORDS_MAX)
        .map(|(w, _)| w)
        .collect()
}

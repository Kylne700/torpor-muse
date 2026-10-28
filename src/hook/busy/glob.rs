//! Shell patterns on one word: `{a,b}` brace expansion and `*`, `?`, `[a-z]` matching.

/// Words one `{a,b}` word may be expanded into; past it a group is read as `*`.
const BRACES: usize = 256;

/// The words a shell makes of one with `{a,b}` groups: `app.db{,-wal}` is `app.db` and `app.db-wal`. A group with no comma
/// (`{x}`), one that is never closed, and every group past `BRACES` words are read as `*` (more than they match, which is
/// the safe side).
pub(super) fn expand(w: &str) -> Vec<String> {
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
pub(super) fn glob_match(pat: &str, name: &str) -> bool {
    !(name.starts_with('.') && !pat.starts_with('.')) && fnmatch(pat, name)
}

/// `glob_match` without the dot rule: what `find -name` does (its `*` also matches a leading dot).
pub(super) fn fnmatch(pat: &str, name: &str) -> bool {
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

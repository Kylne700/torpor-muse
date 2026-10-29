//! Windows: the parts of a long file, past the start that always goes, that the developer's words are about.
//!
//! A line hits when it holds a word of the developer's words (letters only, split at case changes, stemmed, no
//! filler words). A word counts for more the fewer lines of the project hold it. Each hit grows into the block of code
//! around it, and the best blocks go into the room the starts left under the total cap. The text arrives redacted and
//! line-capped, and nothing here adds to it, so a window can hold nothing the start could not.

use std::collections::BTreeSet;

/// Most bytes of windows for one file.
pub(super) const WINDOW_CAP: usize = 4_000;

/// The marker where text was left out.
pub(super) const CUT: &str = "[cut]";

/// A block grown from one hit: at most this many lines and bytes.
const BLOCK_LINES: usize = 40;
const BLOCK_BYTES: usize = 1_800;

/// Windows with at most this many lines between them are joined.
const GAP: usize = 2;

/// A paragraph above a block (its constants, a small helper) goes with it when it is at most this many lines.
const PARAGRAPH: usize = 6;

/// Hit lines kept per file, best first.
const HITS: usize = 24;

/// Words of a prompt that say nothing about the code.
const STOP: &[&str] = &[
    "the", "and", "for", "but", "not", "you", "are", "was", "has", "had", "its", "our", "out",
    "any", "all", "can", "did", "how", "too", "now", "add", "let", "may", "one", "two", "who",
    "why", "yet", "get", "got", "set", "use", "this", "that", "with", "from", "have", "will",
    "just", "make", "sure", "also", "next", "then", "when", "what", "need", "want", "like", "show",
    "shows", "please", "thanks", "thank", "work", "works", "working", "worked", "still", "again",
    "redacted",
];

/// The stems of the words in `s`: letters only, split at case changes (`checkLogin` is two), three letters or more,
/// no filler word.
pub(super) fn stems(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphabetic() {
            words.push(std::mem::take(&mut cur));
            continue;
        }
        let prev = i.checked_sub(1).map(|j| chars[j]);
        let next = chars.get(i + 1).copied();
        let starts_word = c.is_uppercase()
            && prev.is_some_and(|p| {
                p.is_lowercase() || (p.is_uppercase() && next.is_some_and(char::is_lowercase))
            });
        if starts_word {
            words.push(std::mem::take(&mut cur));
        }
        cur.push(c);
    }
    words.push(cur);
    words
        .iter()
        .map(|w| w.to_lowercase())
        .filter(|w| w.chars().count() >= 3 && !STOP.contains(&w.as_str()))
        .map(|w| stem(&w))
        .collect()
}

/// shortcut: English suffixes only (plural, ing, ed, ly, a final e), the same for the prompt and the code so that they
/// meet; upgrade when a language's words are seen not to meet.
fn stem(w: &str) -> String {
    let mut s = w.to_string();
    if s.ends_with("ies") && s.len() > 4 {
        s.truncate(s.len() - 3);
        s.push('y');
    } else if s.ends_with("sses")
        || ["xes", "zes", "ches", "shes"]
            .iter()
            .any(|e| s.ends_with(e))
    {
        s.truncate(s.len() - 2);
    } else if s.ends_with('s') && !["ss", "us", "is"].iter().any(|e| s.ends_with(e)) && s.len() > 3
    {
        s.pop();
    }
    let mut verb = false;
    for suffix in ["ing", "ed", "ly"] {
        if s.ends_with(suffix) && s.len() >= suffix.len() + 3 {
            s.truncate(s.len() - suffix.len());
            verb = suffix != "ly";
            break;
        }
    }
    let mut last = s.chars().rev();
    if let (true, Some(a), Some(b)) = (verb, last.next(), last.next())
        && a == b
        && !"aeiouslz".contains(a)
    {
        s.pop();
    }
    if s.ends_with('e') && s.len() >= 4 {
        s.pop();
    }
    s
}

/// The words a prompt is looked up by.
pub(super) fn keys(words: &str) -> BTreeSet<String> {
    stems(words).into_iter().collect()
}

fn indent(l: &str) -> usize {
    l.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

fn blank(l: &str) -> bool {
    l.trim().is_empty()
}

/// A comment or a decorator (`@app.route(...)`): it belongs to the code under it.
fn comment(l: &str) -> bool {
    let t = l.trim_start();
    ["//", "#", "/*", "*", "--", "<!--", "@"]
        .iter()
        .any(|p| t.starts_with(p))
}

/// A line that only closes a block: `}`, `});`, `)`, `]`, `end`, `</div>`.
fn closer(l: &str) -> bool {
    let t = l.trim_start();
    t.starts_with(['}', ')', ']']) || t == "end" || t.starts_with("end ") || t.starts_with("</")
}

/// A line that goes on with the block above it at the same indent: `} else {`, `elif x:`, `rescue`.
fn goes_on(l: &str) -> bool {
    let t = l.trim_start().trim_start_matches('}').trim_start();
    [
        "else", "elif", "elsif", "catch", "except", "finally", "rescue", "ensure",
    ]
    .iter()
    .any(|w| {
        t.strip_prefix(w)
            .is_some_and(|r| !r.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
    })
}

/// A run of lines of one file, both ends included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Block {
    pub from: usize,
    pub to: usize,
}

/// The last line of the block that opens at line `head`: the lines indented deeper, and the line that closes it.
fn extent(lines: &[String], head: usize) -> usize {
    let ind = indent(&lines[head]);
    let mut stop = head + 1;
    while stop < lines.len()
        && (blank(&lines[stop])
            || indent(&lines[stop]) > ind
            || (indent(&lines[stop]) == ind && goes_on(&lines[stop])))
    {
        stop += 1;
    }
    let mut to = stop - 1;
    if stop < lines.len() && indent(&lines[stop]) == ind && closer(&lines[stop]) {
        to = stop;
    }
    while to > head && blank(&lines[to]) {
        to -= 1;
    }
    to
}

fn size(lines: &[String], a: usize, b: usize) -> usize {
    lines[a..=b].iter().map(|l| l.len() + 1).sum()
}

/// The block of code around line `hit`, never above line `first`: the function or route it sits in (the largest one
/// that is not over the block caps), the comment lines above it, and, when it starts at the left margin, the small
/// paragraph above that (its constants or helper).
fn grow(lines: &[String], hit: usize, first: usize) -> Block {
    let ind = |i: usize| indent(&lines[i]);
    let fits = |a: usize| {
        let b = extent(lines, a);
        b - a < BLOCK_LINES && size(lines, a, b) <= BLOCK_BYTES
    };
    let parent = |i: usize| {
        (first..i)
            .rev()
            .find(|&j| !blank(&lines[j]) && ind(j) < ind(i))
    };
    let mut hit = hit;
    if comment(&lines[hit]) {
        // a comment stands for the code under it
        hit = (hit + 1..lines.len().min(hit + 4))
            .find(|&j| !comment(&lines[j]) && !blank(&lines[j]))
            .unwrap_or(hit);
    }
    let opens = (hit + 1..lines.len())
        .find(|&j| !blank(&lines[j]))
        .is_some_and(|j| ind(j) > ind(hit));
    let mut head = if opens {
        hit
    } else {
        parent(hit).unwrap_or(hit)
    };
    while let Some(up) = parent(head).filter(|&up| fits(up)) {
        head = up;
    }
    let to = extent(lines, head);
    let mut from = head;
    while from > first && head - from < 3 && comment(&lines[from - 1]) {
        from -= 1;
    }
    if ind(head) == 0 && from >= first + 2 && blank(&lines[from - 1]) && !blank(&lines[from - 2]) {
        let mut p = from - 2;
        while p > first && !blank(&lines[p - 1]) {
            p -= 1;
        }
        if from - 1 - p <= PARAGRAPH && ind(p) == 0 {
            from = p;
        }
    }
    let mut to = to.max(hit);
    while to - from >= BLOCK_LINES || size(lines, from, to) > BLOCK_BYTES {
        // keep the hit: drop the end that is farther from it
        if hit - from > to - hit {
            from += 1;
        } else {
            to -= 1;
        }
    }
    Block { from, to }
}

/// Windows that touch or come within `GAP` lines of each other become one.
fn join(mut v: Vec<Block>) -> Vec<Block> {
    v.sort_by_key(|b| (b.from, b.to));
    let mut out: Vec<Block> = Vec::new();
    for b in v {
        match out.last_mut() {
            Some(last) if b.from <= last.to + 1 + GAP => last.to = last.to.max(b.to),
            _ => out.push(b),
        }
    }
    out
}

/// The windows as they go after the start of a file: each on a new line, the marker after it where more follows.
pub(super) fn text(lines: &[String], windows: &[Block]) -> String {
    let mut out = String::new();
    for w in windows {
        out.push('\n');
        out.push_str(&lines[w.from..=w.to].join("\n"));
        if w.to + 1 < lines.len() {
            out.push('\n');
            out.push_str(CUT);
        }
    }
    out
}

/// A file that may get windows. `lines`: all its lines, redacted. `first`: the first line a window may begin at, or
/// `None` for a file that gets none (it only counts in how common a word is).
pub(super) struct File<'a> {
    pub lines: &'a [String],
    pub first: Option<usize>,
}

/// The windows of each file, best blocks first, `budget` bytes in all. Same files and words, same windows.
pub(super) fn windows(files: &[File], words: &str, budget: usize) -> Vec<Vec<Block>> {
    let mut chosen: Vec<Vec<Block>> = vec![Vec::new(); files.len()];
    let keys: Vec<String> = keys(words).into_iter().collect();
    if keys.is_empty() || budget == 0 {
        return chosen;
    }
    // per file and line: the words of the prompt it holds
    let held: Vec<Vec<Vec<usize>>> = files
        .iter()
        .map(|f| {
            f.lines
                .iter()
                .map(|l| {
                    let have: BTreeSet<String> = stems(l).into_iter().collect();
                    (0..keys.len())
                        .filter(|&k| have.contains(&keys[k]))
                        .collect()
                })
                .collect()
        })
        .collect();
    let mut common = vec![0u64; keys.len()];
    for k in held.iter().flatten().flatten() {
        common[*k] += 1;
    }
    let weight: Vec<u64> = common.iter().map(|&c| 1_000_000 / c.max(1)).collect();
    let mut all: Vec<(u64, usize, Block)> = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let Some(first) = f.first else { continue };
        let mut hits: Vec<(u64, usize)> = (first..f.lines.len())
            .map(|l| (held[i][l].iter().map(|&k| weight[k]).sum(), l))
            .filter(|&(s, _)| s > 0)
            .collect();
        hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        hits.truncate(HITS);
        let mut blocks: Vec<Block> = hits.iter().map(|&(_, l)| grow(f.lines, l, first)).collect();
        blocks.sort_by_key(|b| (b.from, b.to));
        blocks.dedup();
        for b in blocks {
            let present: BTreeSet<usize> =
                held[i][b.from..=b.to].iter().flatten().copied().collect();
            all.push((present.iter().map(|&k| weight[k]).sum(), i, b));
        }
    }
    all.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.from.cmp(&b.2.from))
    });
    let mut spent = 0;
    for (_, i, b) in all {
        let lines = files[i].lines;
        let mut trial = chosen[i].clone();
        trial.push(b);
        let trial = join(trial);
        let (old, new) = (text(lines, &chosen[i]).len(), text(lines, &trial).len());
        if new <= WINDOW_CAP && spent + new - old <= budget {
            spent = spent + new - old;
            chosen[i] = trial;
        }
    }
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(String::from).collect()
    }

    fn filler(i: usize) -> String {
        format!(
            "app.get('/api/thing{i}', (req, res) => {{\n  const row = db.find({i});\n  res.json(row);\n}});\n"
        )
    }

    /// `before` filler routes, the route under test, `after` filler routes.
    fn file_with(route: &str, before: usize, after: usize) -> Vec<String> {
        let mut t = String::new();
        for i in 0..before {
            t += &filler(i);
            t += "\n";
        }
        t += route;
        for i in 0..after {
            t += "\n";
            t += &filler(100 + i);
        }
        lines(&t)
    }

    const ZEBRA: &str = "// Zebra census: how many zebras the park holds.\napp.get('/api/zebras', (req, res) => {\n  const n = db.count('zebra');\n  res.json({ zebras: n });\n});\n";

    fn one(ls: &[String], first: usize) -> File<'_> {
        File {
            lines: ls,
            first: Some(first),
        }
    }

    #[test]
    fn words_become_stems_that_meet_the_code() {
        assert_eq!(stems("checkLogin"), ["check", "login"]);
        assert_eq!(stems("The HTTPServer, please!"), ["http", "server"]);
        assert_eq!(stems("orders ordered ordering order"), ["order"; 4]);
        assert_eq!(stems("queries query"), ["query"; 2]);
        assert!(stems("to be or an it").is_empty());
        assert_eq!(keys("Zebra zebras ZEBRA_COUNT").len(), 2);
    }

    #[test]
    fn a_hit_far_into_a_file_gets_the_whole_route_around_it() {
        let ls = file_with(ZEBRA, 60, 5);
        let at = ls.iter().position(|l| l.contains("/api/zebras")).unwrap();
        let w = windows(&[one(&ls, 30)], "the zebra route", 3_000);
        assert_eq!(w[0].len(), 1, "{:?}", w[0]);
        let b = w[0][0];
        assert!(b.from < at && b.to == at + 3, "{b:?} around {at}");
        assert!(b.from + 1 + PARAGRAPH >= at, "only a small paragraph above");
        let shown = text(&ls, &w[0]);
        assert!(
            shown.contains("res.json({ zebras: n });\n});\n[cut]"),
            "{shown}"
        );
    }

    #[test]
    fn a_window_never_begins_before_the_first_line_it_may() {
        let mut ls = file_with(ZEBRA, 40, 40);
        ls[10] = "const zebra = 1;".into();
        let w = windows(&[one(&ls, 50)], "zebra", 3_000);
        assert!(
            !w[0].is_empty() && w[0].iter().all(|b| b.from >= 50),
            "{:?}",
            w[0]
        );
        let none = windows(
            &[File {
                lines: &ls,
                first: None,
            }],
            "zebra",
            3_000,
        );
        assert!(none[0].is_empty());
    }

    #[test]
    fn the_budget_and_the_file_cap_hold() {
        let mut t = String::new();
        for i in 0..80 {
            t +=
                &format!("app.get('/api/zebra{i}', (req, res) => {{\n  res.json(zebra);\n}});\n\n");
        }
        let ls = lines(&t);
        for budget in [0, 100, 700, 1_500, 100_000] {
            let w = windows(&[one(&ls, 0), one(&ls, 0)], "zebra", budget);
            let used: usize = w.iter().map(|f| text(&ls, f).len()).sum();
            assert!(used <= budget, "{used} over {budget}");
            assert!(w.iter().all(|f| text(&ls, f).len() <= WINDOW_CAP));
            assert!(
                !(700..=1_500).contains(&budget) || used * 2 > budget,
                "room is used: {used} of {budget}"
            );
        }
    }

    #[test]
    fn the_same_input_gives_the_same_windows() {
        let ls = file_with(ZEBRA, 50, 50);
        let a = windows(&[one(&ls, 20), one(&ls, 90)], "zebra thing census", 2_000);
        let b = windows(&[one(&ls, 20), one(&ls, 90)], "zebra thing census", 2_000);
        assert_eq!(a, b);
    }

    #[test]
    fn a_word_few_lines_hold_beats_a_word_many_hold() {
        let mut t = String::new();
        for i in 0..30 {
            t +=
                &format!("app.get('/api/order{i}', (req, res) => {{\n  res.json(order);\n}});\n\n");
        }
        t += "app.get('/api/zebra', (req, res) => {\n  res.json(zebra);\n});\n";
        let ls = lines(&t);
        let w = windows(&[one(&ls, 0)], "order zebra", 140);
        assert_eq!(w[0].len(), 1, "{:?}", w[0]);
        assert!(text(&ls, &w[0]).contains("zebra"), "{}", text(&ls, &w[0]));
    }

    #[test]
    fn nothing_to_look_for_means_no_windows() {
        let ls = file_with(ZEBRA, 10, 10);
        assert!(windows(&[one(&ls, 0)], "please add the work to it", 3_000)[0].is_empty());
        assert!(windows(&[one(&ls, 0)], "qqq xyzzy", 3_000)[0].is_empty());
    }

    #[test]
    fn the_marker_is_where_more_follows_and_nowhere_else() {
        let ls = lines("a\nb\nc\nd\ne");
        let mid = [Block { from: 1, to: 2 }];
        assert_eq!(text(&ls, &mid), "\nb\nc\n[cut]");
        let end = [Block { from: 3, to: 4 }];
        assert_eq!(text(&ls, &end), "\nd\ne");
    }

    #[test]
    fn else_branches_and_decorators_stay_with_their_block() {
        assert!(goes_on("  } else {") && goes_on("else:") && goes_on("  elif x:"));
        assert!(!goes_on("elsewhere()") && !goes_on("  return else_value;"));
        let ls = lines("if (a) {\n  x();\n} else {\n  y();\n}\nz();");
        assert_eq!(extent(&ls, 0), 4);
        let py =
            lines("@app.route('/zebra')\ndef zebra():\n    return 1\n\ndef other():\n    return 2");
        let w = windows(&[one(&py, 0)], "zebra", 1_000);
        assert_eq!(w[0], [Block { from: 0, to: 2 }]);
    }
}

//! The excerpt that goes to the model holds the code a claim is about, also when that code sits far into its file.
//! Offline: no model. The mock Meta API logs the request and the test reads the `repo_files` block in it.
//! `tests/fixtures/late/claims.json` names, for every claim, the lines of its app that implement it (1-based,
//! inclusive). A claim counts as seen when all of those lines are in the excerpt, in one piece.

mod writer_common;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use writer_common::*;

/// The README promises "about 14 KB" of the project to the model.
const TOTAL_CAP: usize = 14_000;
/// The first part of a file that was always sent.
const FILE_CAP: usize = 6_000;
/// Words the redaction masks by name: the fixtures must not hold them, or a masked line would not match.
const MASKED_WORDS: [&str; 12] = [
    "secret",
    "token",
    "password",
    "passwd",
    "pwd",
    "apikey",
    "api_key",
    "credential",
    "signing",
    "salt",
    "bearer",
    "authorization",
];

struct Claim {
    id: String,
    late: bool,
    app: String,
    file: String,
    from: usize,
    to: usize,
    words: String,
}

struct Seen {
    id: String,
    late: bool,
    bytes: usize,
    found: bool,
    /// How many of the claim's non-blank lines are not in the excerpt (for the failure message).
    absent: usize,
}

fn fixtures() -> PathBuf {
    repo_root().join("tests/fixtures/late")
}

fn claims() -> Vec<Claim> {
    let text = fs::read_to_string(fixtures().join("claims.json")).unwrap();
    let all: Value = serde_json::from_str(&text).unwrap();
    all.as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let s = |k: &str| c[k].as_str().unwrap().to_string();
            Claim {
                id: s("id"),
                late: s("kind") == "late",
                app: s("app"),
                file: s("file"),
                from: usize::try_from(c["lines"][0].as_u64().unwrap()).unwrap(),
                to: usize::try_from(c["lines"][1].as_u64().unwrap()).unwrap(),
                words: s("words"),
            }
        })
        .collect()
}

/// Every file of a fixture app as (path inside the app, text).
fn app_files(dir: &Path, rel: &str, out: &mut Vec<(String, String)>) {
    let mut entries: Vec<_> = fs::read_dir(dir).unwrap().flatten().collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for e in entries {
        let name = e.file_name().into_string().unwrap();
        let path = if rel.is_empty() {
            name
        } else {
            format!("{rel}/{name}")
        };
        if e.path().is_dir() {
            app_files(&e.path(), &path, out);
        } else {
            out.push((path, fs::read_to_string(e.path()).unwrap()));
        }
    }
}

fn source(c: &Claim) -> String {
    fs::read_to_string(fixtures().join(&c.app).join(&c.file)).unwrap()
}

/// The lines that implement the claim, as they are in the file.
fn implementing(c: &Claim) -> String {
    let text = source(c);
    let lines: Vec<&str> = text
        .lines()
        .skip(c.from - 1)
        .take(c.to - c.from + 1)
        .collect();
    lines.join("\n")
}

/// The `repo_files` block of a model input: what the model is shown of the project.
fn repo_files(input: &str) -> &str {
    let head = input
        .find("<<<DATA repo_files nonce=")
        .expect("a repo_files block");
    let body = &input[head..];
    let start = body.find(">>>\n").expect("the block opens") + 4;
    let end = body.find("\n<<<END nonce=").expect("the block closes");
    &body[start..end]
}

/// One real `torpor-muse suggest` run of the claim's words in a copy of its app; what the model was shown of it.
fn run(c: &Claim) -> Seen {
    let sb = Sandbox::new(&format!("late-{}", c.id.to_lowercase()));
    fs::remove_file(sb.ws.join("server.js")).unwrap();
    let mut files = Vec::new();
    app_files(&fixtures().join(&c.app), "", &mut files);
    for (rel, text) in &files {
        sb.write(rel, text);
    }
    sb.git(&["add", "-A"]);
    sb.git(&["commit", "-qm", "app"]);
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), &c.words, &[]);
    assert!(
        !mock.writer().is_empty(),
        "{}: the model was not asked: out={:?} err={:?}",
        c.id,
        r.out,
        r.err
    );
    let input = mock.input(0);
    let shown = repo_files(&input);
    let there: HashSet<&str> = shown.lines().collect();
    let code = implementing(c);
    Seen {
        id: c.id.clone(),
        late: c.late,
        bytes: shown.len(),
        found: shown.contains(&code),
        absent: code
            .lines()
            .filter(|l| !l.trim().is_empty() && !there.contains(l))
            .count(),
    }
}

/// All claims, run once per test binary (three at a time).
fn seen() -> &'static Vec<Seen> {
    static SEEN: OnceLock<Vec<Seen>> = OnceLock::new();
    SEEN.get_or_init(|| {
        let all = claims();
        let mut out = Vec::new();
        std::thread::scope(|s| {
            let runs: Vec<_> = all
                .chunks(3)
                .map(|part| s.spawn(move || part.iter().map(run).collect::<Vec<_>>()))
                .collect();
            for r in runs {
                out.extend(r.join().unwrap());
            }
        });
        out
    })
}

fn report(late: bool) -> (usize, usize, Vec<String>) {
    let of_kind: Vec<&Seen> = seen().iter().filter(|s| s.late == late).collect();
    let found = of_kind.iter().filter(|s| s.found).count();
    let missed = of_kind
        .iter()
        .filter(|s| !s.found)
        .map(|s| format!("{} ({} lines absent)", s.id, s.absent))
        .collect();
    let longest = seen().iter().map(|s| s.bytes).max().unwrap_or(0);
    println!(
        "{} claims: {found} of {} seen, longest excerpt {longest} bytes",
        if late { "late" } else { "early" },
        of_kind.len()
    );
    (found, of_kind.len(), missed)
}

#[test]
fn the_table_says_where_the_code_of_each_claim_is() {
    let all = claims();
    assert!(
        all.iter().filter(|c| c.late).count() >= 12,
        "12 late claims"
    );
    assert!(
        all.iter().filter(|c| !c.late).count() >= 6,
        "6 early claims"
    );
    for c in &all {
        let ctx = format!("{} ({}/{})", c.id, c.app, c.file);
        let text = source(c);
        assert!(text.is_ascii(), "{ctx}: ascii only");
        assert!(
            !text.contains("<<<") && !text.contains(">>>"),
            "{ctx}: the prompt rewrites these"
        );
        let low = text.to_lowercase();
        for w in MASKED_WORDS {
            assert!(!low.contains(w), "{ctx}: `{w}` would be masked");
        }
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines.iter().all(|l| l.len() < 300), "{ctx}: line cap");
        assert!(
            1 <= c.from && c.from <= c.to && c.to <= lines.len(),
            "{ctx}"
        );
        assert!(!implementing(c).trim().is_empty(), "{ctx}");
        let start: usize = lines[..c.from - 1].iter().map(|l| l.len() + 1).sum();
        let end: usize = lines[..c.to].iter().map(|l| l.len() + 1).sum::<usize>() - 1;
        if c.late {
            assert!(start >= FILE_CAP, "{ctx}: starts at byte {start}");
            assert!(
                (7_000..=60_000).contains(&text.len()),
                "{ctx}: {} bytes",
                text.len()
            );
        } else {
            assert!(end < FILE_CAP, "{ctx}: ends at byte {end}");
        }
    }
}

#[test]
fn a_claim_about_the_start_of_a_file_still_gets_its_code() {
    let (found, of, missed) = report(false);
    assert_eq!(
        found, of,
        "early claims whose code is not in the excerpt: {missed:?}"
    );
}

#[test]
fn no_excerpt_is_over_the_total_cap() {
    let long: Vec<String> = seen()
        .iter()
        .filter(|s| s.bytes > TOTAL_CAP)
        .map(|s| format!("{} {}", s.id, s.bytes))
        .collect();
    assert!(long.is_empty(), "over {TOTAL_CAP} bytes: {long:?}");
}

#[test]
fn a_claim_about_code_far_into_a_file_gets_its_code() {
    let (found, of, missed) = report(true);
    assert_eq!(of, 12);
    assert!(
        found >= 11,
        "late claims seen {found} of {of}; missed {missed:?}"
    );
}

//! The excerpt of a project that goes to the model: which files may be read and how much of each is sent.

use super::redact::redact;
use super::window;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

pub(super) const TOTAL_CAP: usize = 14_000;

pub(super) const FILE_CAP: usize = 6_000;

pub(super) const LINE_CAP: usize = 300;

const BIG_FILE: u64 = 64_000;

const LIST_CAP: usize = 120;

/// Code only: the app's data (.json, .txt, .csv, .sql, notes in .md) never goes to the model (dogfood1 M2).
const EXTS: [&str; 18] = [
    "js", "mjs", "cjs", "ts", "tsx", "jsx", "html", "ejs", "hbs", "pug", "py", "rb", "php", "go",
    "java", "rs", "cs", "vue",
];

const SKIP_DIRS: [&str; 11] = [
    "node_modules",
    "dist",
    "build",
    "target",
    "vendor",
    "coverage",
    "__pycache__",
    "venv",
    "out",
    "tmp",
    "logs",
];

fn git_list(root: &Path, extra: &[&str]) -> Option<BTreeSet<String>> {
    let out = Command::new("git")
        .args([
            "-c",
            "core.fsmonitor=false",
            "ls-files",
            "-z",
            "--exclude-standard",
        ])
        .args(extra)
        .current_dir(root)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| {
        out.stdout
            .split(|&b| b == 0)
            .filter_map(|p| std::str::from_utf8(p).ok())
            .filter(|p| !p.is_empty())
            .map(String::from)
            .collect()
    })
}

/// The files git would show (tracked and not ignored). Without git: the top level only, minus the plain file
/// names in the root `.gitignore`.
fn listing(root: &Path) -> Vec<String> {
    if let Some(mut all) = git_list(root, &["--cached", "--others"]) {
        if let Some(ignored) = git_list(root, &["--cached", "--ignored"]) {
            all.retain(|p| !ignored.contains(p));
        }
        return all.into_iter().collect();
    }
    let ignored: BTreeSet<String> = fs::read_to_string(root.join(".gitignore"))
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains(['/', '*', '?', '[', '!', '#']))
        .map(String::from)
        .collect();
    let mut v: Vec<String> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| !ignored.contains(n))
        .collect();
    v.sort();
    v
}

/// A code file (`EXTS`) or one of the manifests the writer needs to start the app: `package.json` (the scripts) and
/// `tsconfig*.json`. No other file, so no data, notes or dumps; never in a dot folder or a skipped one.
/// shortcut: data kept in a code file (a `data.js` that exports records) still goes; upgrade when a project is seen
/// to keep its records that way.
fn allowed(p: &str) -> bool {
    let parts: Vec<&str> = p.split('/').collect();
    let name = parts.last().copied().unwrap_or("").to_lowercase();
    let ext = name.rsplit_once('.').map_or("", |(_, e)| e);
    let manifest = name == "package.json" || (name.starts_with("tsconfig") && ext == "json");
    parts
        .iter()
        .all(|c| !c.starts_with('.') && !SKIP_DIRS.contains(c))
        && (EXTS.contains(&ext) || manifest)
        && !name.contains("secret")
        && !name.contains("credential")
        && !name.ends_with(".min.js")
        && !name.ends_with(".d.ts")
}

fn rank(p: &str) -> u8 {
    let low = p.to_lowercase();
    let base = low.rsplit('/').next().unwrap_or(&low);
    if base == "package.json" {
        0
    } else if ["server.", "app.", "index.", "main."]
        .iter()
        .any(|n| base.starts_with(n))
    {
        2
    } else if low.contains("test") || low.contains("spec") {
        6
    } else if !low.contains('/')
        || ["route", "api", "controller", "handler"]
            .iter()
            .any(|d| low.contains(d))
    {
        3
    } else {
        4
    }
}

/// Cuts at `max` bytes on a character boundary.
pub fn cut(s: &str, max: usize) -> &str {
    let mut n = max.min(s.len());
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    &s[..n]
}

/// The lines of a file the writer may read, redacted and each cut to `LINE_CAP`. The whole text is redacted first and
/// cut afterwards, so that a cut never leaves half a key behind.
fn read_lines(root: &Path, rel: &str, synthetic: bool) -> Option<Vec<String>> {
    let path = root.join(rel);
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > BIG_FILE || !fs::canonicalize(&path).ok()?.starts_with(root)
    {
        return None;
    }
    let text = String::from_utf8(fs::read(&path).ok()?).ok()?;
    let red = redact(&text, synthetic);
    Some(
        red.lines()
            .map(|l| l.chars().take(LINE_CAP).collect())
            .collect(),
    )
}

/// The start of a file that always goes: its first `FILE_CAP` bytes, `[cut]` after them where more follows. With
/// more, also the first line a window may begin at (the line the cut falls in is shown whole there).
fn start_of(lines: &[String]) -> (String, Option<usize>) {
    let body = lines.join("\n");
    let shown = cut(&body, FILE_CAP);
    if shown.len() == body.len() {
        return (body, None);
    }
    let whole = body.as_bytes()[shown.len()] == b'\n';
    let first = shown.matches('\n').count() + usize::from(whole);
    (format!("{shown}\n[cut]"), Some(first))
}

/// The start of a file for the voice keywords: `FILE_CAP` bytes, redacted.
pub(in crate::suggest) fn read_capped(root: &Path, rel: &str, synthetic: bool) -> Option<String> {
    Some(start_of(&read_lines(root, rel, synthetic)?).0)
}

/// The files the writer may read: tracked and not ignored, code or a manifest (`allowed`), no secrets. Sorted.
pub fn allowed_files(root: &Path) -> Vec<String> {
    listing(root).into_iter().filter(|p| allowed(p)).collect()
}

/// A file in the excerpt: its heading and start as they go, its lines, and where a window may begin.
struct Read {
    head: String,
    lines: Vec<String>,
    first: Option<usize>,
}

/// The excerpt: a file list, then the start of the most useful files, then, in the room that is left under
/// `TOTAL_CAP`, the parts of the longer files that `words` (the developer's) are about; all redacted. Stable for an
/// unchanged project and the same words. The starts are what they would be without words, so words only add.
pub fn excerpt(root: &Path, synthetic: bool, words: &str) -> String {
    let mut files = allowed_files(root);
    files.sort_by(|a, b| (rank(a), a).cmp(&(rank(b), b)));
    let mut out = String::from("FILE LIST\n");
    let mut names: Vec<&String> = files.iter().take(LIST_CAP).collect();
    names.sort();
    for n in names {
        out.push_str(n);
        out.push('\n');
    }
    let mut used = out.len();
    let mut read: Vec<Read> = Vec::new();
    for rel in &files {
        let Some(lines) = read_lines(root, rel, synthetic) else {
            continue;
        };
        let (start, first) = start_of(&lines);
        let room = TOTAL_CAP.saturating_sub(used + rel.len() + 16);
        if room < 400 {
            break;
        }
        let shown = cut(&start, room);
        let head = format!("\nFILE {rel}\n{shown}");
        used += head.len() + 1;
        // a start cut by the room is the end of what this file gets
        let first = first.filter(|_| shown.len() == start.len());
        read.push(Read { head, lines, first });
    }
    let all: Vec<window::File> = read
        .iter()
        .map(|r| window::File {
            lines: &r.lines,
            first: r.first,
        })
        .collect();
    let windows = window::windows(&all, words, TOTAL_CAP.saturating_sub(used));
    for (r, w) in read.iter().zip(&windows) {
        out.push_str(&r.head);
        out.push_str(&window::text(&r.lines, w));
        out.push('\n');
    }
    out
}

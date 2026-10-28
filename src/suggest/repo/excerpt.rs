//! The excerpt of a project that goes to the model: which files may be read and how much of each is sent.

use super::redact::redact;
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

pub(in crate::suggest) fn read_capped(root: &Path, rel: &str, synthetic: bool) -> Option<String> {
    let path = root.join(rel);
    let meta = fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > BIG_FILE || !fs::canonicalize(&path).ok()?.starts_with(root)
    {
        return None;
    }
    let text = String::from_utf8(fs::read(&path).ok()?).ok()?;
    // Redact the whole text first and cut afterwards, so that a cut never leaves half a key behind.
    let red = redact(&text, synthetic);
    let lines: Vec<String> = red
        .lines()
        .map(|l| l.chars().take(LINE_CAP).collect())
        .collect();
    let body = lines.join("\n");
    let mut shown = cut(&body, FILE_CAP).to_string();
    if shown.len() < body.len() {
        shown.push_str("\n[cut]");
    }
    Some(shown)
}

/// The files the writer may read: tracked and not ignored, code or a manifest (`allowed`), no secrets. Sorted.
pub fn allowed_files(root: &Path) -> Vec<String> {
    listing(root).into_iter().filter(|p| allowed(p)).collect()
}

/// The excerpt: a file list, then the start of the most useful files, all redacted. Stable for an unchanged project.
pub fn excerpt(root: &Path, synthetic: bool) -> String {
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
    for rel in &files {
        let Some(body) = read_capped(root, rel, synthetic) else {
            continue;
        };
        let room = TOTAL_CAP.saturating_sub(used + rel.len() + 16);
        if room < 400 {
            break;
        }
        let part = format!("\nFILE {rel}\n{}\n", cut(&body, room));
        used += part.len();
        out.push_str(&part);
    }
    out
}

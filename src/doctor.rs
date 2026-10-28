//! `torpor-muse doctor`: is the gate really live, and what broke without a sound. Owner: S3a-shim.
//!
//! It only reads, runs one harmless probe and the engine's read-only `torpor gate`: it never deletes or kills anything
//! that is not its own probe process.
//! Every line starts with `ok`, `info`, `warn` or `FIX`; a problem line is followed by a `do:` line that says how.
//! Exit 0 = nothing to fix (warnings may remain), 1 = at least one FIX.
#![allow(dead_code)] // tests/shim_doctor.rs reaches the parts

use crate::liveness::{self, PROBE_SID};
use crate::{install, transcript};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The only Muse Code version these hooks were tested with (research/60 AD13).
const MUSE_TESTED: &str = "1.4.13";
/// Without these three the gate cannot work.
const REQUIRED: [&str; 3] = ["SessionStart", "UserPromptSubmit", "Stop"];
/// How long the engine lets itself run per event, in ms (Torpor 2.3.1 hook.rs `own_deadline_ms`). A Muse timeout
/// shorter than this (plus 1 s for our own start-up) can kill the hook before the engine has answered.
const ENGINE_MS: [(&str, u64); 10] = [
    ("SessionStart", 3000),
    ("UserPromptSubmit", 5000),
    ("PreToolUse", 2000),
    ("PostToolUse", 2000),
    ("PostToolUseFailure", 2000),
    ("Stop", 30000),
    ("StopFailure", 5000),
    ("SubagentStop", 1000),
    ("PreCompact", 1000),
    ("SessionEnd", 1200),
];
const LOGS_SCANNED: usize = 6;
const ORPHAN_AGE_S: u64 = 120;
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// Muse keeps at most this much of a hook's stdout or stderr (it fails the hook beyond it); so does the probe.
const PIPE_CAP: usize = 16 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Ok,
    Info,
    Warn,
    Fix,
}

#[derive(Default, Debug)]
pub struct Report(pub Vec<(Level, String)>);

impl Report {
    fn add(&mut self, l: Level, what: impl Into<String>) {
        self.0.push((l, what.into()));
    }
    /// A problem and the way out, as two lines.
    fn problem(&mut self, l: Level, what: impl AsRef<str>, how: impl AsRef<str>) {
        let (w, h) = (what.as_ref(), how.as_ref());
        self.0.push((l, format!("{w}\n      do: {h}")));
    }
    pub fn fixes(&self) -> usize {
        self.0.iter().filter(|(l, _)| *l == Level::Fix).count()
    }
    pub fn has(&self, l: Level, needle: &str) -> bool {
        self.0.iter().any(|(k, s)| *k == l && s.contains(needle))
    }
    pub fn text(&self) -> String {
        let tag = |l: &Level| match l {
            Level::Ok => "ok  ",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Fix => "FIX ",
        };
        self.0
            .iter()
            .map(|(l, s)| format!("{} {s}\n", tag(l)))
            .collect()
    }
}

/// Everything doctor takes from the outside, so a test can set it without touching the environment.
pub struct Ctx {
    /// Muse's USER-scope settings file.
    pub settings: PathBuf,
    pub home: PathBuf,
    /// Muse's session logs as this shell would find them (it honours XDG_DATA_HOME), and as the hooks find them
    /// (their environment is cleared, so always `$HOME/.local/share`).
    pub logs: PathBuf,
    pub hook_logs: PathBuf,
    pub xdg_data: Option<PathBuf>,
    /// TORPOR_HOME of this shell.
    pub env_vault: Option<PathBuf>,
    pub shell: Option<String>,
    pub temps: Vec<PathBuf>,
    pub cwd: PathBuf,
    pub now_ms: u64,
    /// How long the probe may run before doctor kills the process group it started.
    pub probe_timeout: Duration,
}

impl Ctx {
    pub fn from_env(settings: PathBuf) -> Ctx {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        let home = PathBuf::from(var("HOME").unwrap_or_default());
        let hook_logs = transcript::Paths::from_env().sessions;
        let xdg_data = var("XDG_DATA_HOME").map(PathBuf::from);
        let logs = xdg_data
            .as_ref()
            .map_or_else(|| hook_logs.clone(), |x| x.join("muse/sessions"));
        let mut temps = vec![std::env::temp_dir(), PathBuf::from("/tmp")];
        temps.dedup();
        Ctx {
            settings,
            home,
            logs,
            hook_logs,
            xdg_data,
            env_vault: var("TORPOR_HOME").map(PathBuf::from),
            shell: var("SHELL").map(|s| s.to_string_lossy().into_owned()),
            temps,
            cwd: std::env::current_dir().unwrap_or_default(),
            now_ms: liveness::now_ms(),
            probe_timeout: PROBE_TIMEOUT,
        }
    }
}

pub fn run() -> u8 {
    finish(&examine(&Ctx::from_env(install::settings_path())))
}

/// Prints the report; the exit code is 1 when anything needs a fix.
pub fn finish(r: &Report) -> u8 {
    print!("{}", r.text());
    let warns = r.0.iter().filter(|(l, _)| *l == Level::Warn).count();
    match (r.fixes(), warns) {
        (0, 0) => println!("doctor: nothing to fix"),
        (0, w) => println!("doctor: nothing to fix, {w} warning(s) to read"),
        (n, _) => println!("doctor: {n} to fix"),
    }
    u8::from(r.fixes() > 0)
}

pub fn examine(c: &Ctx) -> Report {
    let mut r = Report::default();
    let (hooks, parsed) = match read_hooks(&c.settings) {
        Ok(h) => (h, true),
        Err((missing, what)) => {
            let how = if missing {
                "torpor-muse init"
            } else {
                "repair the JSON (init keeps a .torpor-muse.bak copy of the file it changed), then run torpor-muse init"
            };
            r.problem(Level::Fix, what, how);
            (Vec::new(), false)
        }
    };
    let ours: Vec<&Hook> = hooks.iter().filter(|h| is_ours(&h.command)).collect();
    if parsed {
        check_hooks(c, &ours, &mut r);
    }
    let vault = check_vault(c, &ours, &mut r);
    check_logs(c, &ours, &mut r);
    check_gate(c, &vault, &ours, &mut r);
    check_promise_gate(c, &vault, &ours, &mut r);
    check_errors(&vault, c.now_ms, &mut r);
    check_problems(&vault, c.now_ms, &mut r);
    check_probe(c, &ours, &mut r);
    if !ours.is_empty() {
        check_writer(&mut r);
    }
    check_xdg(c, &mut r);
    check_orphans(c, &vault, &mut r);
    r
}

// ---------------------------------------------------------------- settings

pub struct Hook {
    pub event: String,
    pub command: String,
    pub timeout: Option<u64>,
    /// A `timeout` that is not a whole number of seconds: Muse refuses or drops it.
    pub timeout_bad: bool,
    pub is_async: bool,
}

/// Err((file is missing, message)).
fn read_hooks(path: &Path) -> Result<Vec<Hook>, (bool, String)> {
    let text = fs::read_to_string(path).map_err(|e| {
        let missing = e.kind() == std::io::ErrorKind::NotFound;
        let what = if missing {
            format!(
                "no Muse settings file at {}: no torpor-muse hook is installed",
                path.display()
            )
        } else {
            format!("cannot read {}: {e}", path.display())
        };
        (missing, what)
    })?;
    let v: Value = serde_json::from_str(&text).map_err(|e| {
        (
            false,
            format!(
                "{} is not valid JSON ({e}): Muse ignores its hooks",
                path.display()
            ),
        )
    })?;
    let mut out = Vec::new();
    for (event, groups) in v
        .get("hooks")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        for g in groups.as_array().into_iter().flatten() {
            for h in g
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(command) = h.get("command").and_then(Value::as_str) else {
                    continue;
                };
                let t = h.get("timeout");
                out.push(Hook {
                    event: event.clone(),
                    command: command.to_string(),
                    timeout: t.and_then(Value::as_u64),
                    timeout_bad: t.is_some_and(|t| !t.is_u64()),
                    is_async: h.get("async").and_then(Value::as_bool) == Some(true),
                });
            }
        }
    }
    Ok(out)
}

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

pub fn quote(s: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "_-./=:@%+,".contains(c);
    if !s.is_empty() && s.chars().all(plain) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
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

fn tool_of(command: &str, name: &str) -> Option<PathBuf> {
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

fn check_hooks(c: &Ctx, ours: &[&Hook], r: &mut Report) {
    if ours.is_empty() {
        // when `init` would refuse the file, the reason comes first: "run init" would only meet the same refusal
        match crate::install::refusal(&c.settings) {
            Some(why) => r.problem(
                Level::Fix,
                format!("no torpor-muse hook can be installed: {why}"),
                "fix what is named above (init does not edit a symbolic link, and it leaves a file Muse Code would reject as it is), then run torpor-muse init",
            ),
            None => r.problem(
                Level::Fix,
                format!(
                    "no torpor-muse hook in {}: the gate is not installed",
                    c.settings.display()
                ),
                "torpor-muse init",
            ),
        }
        return;
    }
    let events: BTreeSet<&str> = ours.iter().map(|h| h.event.as_str()).collect();
    for need in REQUIRED {
        if !events.contains(need) {
            r.problem(
                Level::Fix,
                format!(
                    "no torpor-muse hook for {need} in {}: the gate needs it",
                    c.settings.display()
                ),
                "torpor-muse init",
            );
        }
    }
    let mut seconds: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for h in ours {
        let t = h.timeout.map_or("default".to_string(), |t| format!("{t}s"));
        seconds.entry(&h.event).or_default().push(t);
        if h.is_async {
            r.problem(
                Level::Fix,
                format!("the {} hook is async: Muse does not wait for it, so nothing it says reaches the model", h.event),
                "torpor-muse init (it writes the hooks as synchronous)",
            );
        }
        if h.timeout_bad {
            r.problem(
                Level::Fix,
                format!("the {} hook has a timeout that is not a whole number of seconds: Muse refuses or drops it", h.event),
                "torpor-muse init",
            );
        }
        let need = ENGINE_MS
            .iter()
            .find(|(e, _)| *e == h.event)
            .map(|(_, ms)| ms + 1000);
        if let (Some(t), Some(need)) = (h.timeout, need)
            && t * 1000 < need
        {
            r.problem(
                Level::Warn,
                format!("the {} timeout ({t}s) is shorter than the engine's own limit plus start-up ({}s): Muse can kill the hook before the engine answers", h.event, need.div_ceil(1000)),
                "torpor-muse init",
            );
        }
    }
    let list: Vec<String> = seconds
        .iter()
        .map(|(e, t)| format!("{e} {}", t.join("/")))
        .collect();
    r.add(Level::Info, format!("hook timeouts: {}", list.join(", ")));
    for h in ours {
        if let Some(bin) = tool_of(&h.command, "TORPOR_BIN")
            && !bin.is_file()
        {
            r.problem(
                Level::Fix,
                format!("the engine {} (TORPOR_BIN in the {} hook) does not exist: every hook then runs without a check", bin.display(), h.event),
                "install Torpor again, then run torpor-muse init",
            );
            break;
        }
    }
}

/// Prints the vault the hooks really write to; returns it.
fn check_vault(c: &Ctx, ours: &[&Hook], r: &mut Report) -> PathBuf {
    let default = c.home.join(".torpor");
    let baked: BTreeSet<Option<PathBuf>> = ours.iter().map(|h| vault_of(&h.command)).collect();
    let effective = match baked.iter().flatten().next() {
        Some(v) => v.clone(),
        None => default.clone(),
    };
    if baked.len() > 1 {
        let all: Vec<String> = baked
            .iter()
            .map(|v| {
                v.as_ref()
                    .map_or("(none)".into(), |p| p.display().to_string())
            })
            .collect();
        r.problem(
            Level::Fix,
            format!("the hook commands use different vaults: {}", all.join(", ")),
            "torpor-muse init (it rewrites every hook with the same vault)",
        );
    } else if !ours.is_empty() && baked.contains(&None) {
        r.problem(
            Level::Warn,
            format!(
                "the hook commands carry no TORPOR_HOME, so they use {}",
                default.display()
            ),
            "torpor-muse init",
        );
    } else if !ours.is_empty() {
        r.add(
            Level::Ok,
            format!(
                "vault: {} (TORPOR_HOME in the generated hook commands)",
                effective.display()
            ),
        );
    }
    let mine = c.env_vault.clone().unwrap_or(default);
    if !ours.is_empty() && mine != effective {
        r.problem(
            Level::Warn,
            format!("this shell uses the vault {}, Muse's hooks use {}: `torpor status` here shows another history", mine.display(), effective.display()),
            format!("export TORPOR_HOME={}", quote(&effective.to_string_lossy())),
        );
    }
    if !ours.is_empty() && !effective.is_dir() {
        r.add(
            Level::Info,
            format!(
                "the vault {} does not exist yet: no hook has run",
                effective.display()
            ),
        );
    }
    effective
}

// ---------------------------------------------------------------- Muse's own session logs

#[derive(Deserialize)]
struct LogRec {
    /// Microseconds since the epoch.
    recorded_at: Option<u64>,
    payload: Option<LogPayload>,
}

#[derive(Deserialize)]
struct LogPayload {
    /// The turn (Muse calls it the run) a hook run belongs to.
    run_id: Option<String>,
    intent_id: Option<String>,
    semantic_kind: Option<Sem>,
    event: Option<LogEvent>,
    record: Option<Meta>,
}

#[derive(Deserialize)]
struct Sem {
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Meta {
    build: Option<Build>,
}

#[derive(Deserialize)]
struct Build {
    semver: Option<String>,
}

#[derive(Deserialize)]
struct LogEvent {
    event: Option<String>,
    hook_key: Option<String>,
    status: Option<String>,
    error: Option<String>,
    exit_code: Option<i64>,
    stdout: Option<String>,
}

struct HookRun {
    event: String,
    /// When Muse recorded the run (microseconds since the epoch).
    at: u64,
    status: String,
    error: String,
    stdout: String,
    log: PathBuf,
}

#[derive(Default)]
struct Scan {
    version: Option<String>,
    /// Newest run of every hook of the settings file, by hook_key.
    last: BTreeMap<String, HookRun>,
    /// (session folder, prompt) pairs that reached a Stop hook but never a UserPromptSubmit hook.
    no_ups: Vec<(String, String)>,
}

/// Reads one log once. `key_prefix`: the settings file path Muse puts in front of every hook_key.
fn scan_log(path: &Path, key_prefix: &str, scan: &mut Scan) {
    let Ok(f) = File::open(path) else { return };
    let mut rd = BufReader::new(f);
    let (mut line, mut n) = (Vec::new(), 0u32);
    let (mut intents, mut ups, mut stops) = (Vec::new(), HashSet::new(), HashSet::new());
    while rd.read_until(b'\n', &mut line).is_ok_and(|k| k > 0) {
        n += 1;
        let text = std::str::from_utf8(&line).unwrap_or("");
        let kind = if text.contains(r#""kind":"hook_run_terminal""#) {
            1
        } else if text.contains(r#""payload_type":"runtime.user_intent.accepted""#) {
            2
        } else if n <= 8 && text.contains(r#""payload_type":"runtime.session.metadata""#) {
            3
        } else {
            0
        };
        if kind != 0
            && let Ok(rec) = serde_json::from_slice::<LogRec>(&line)
            && let Some(p) = rec.payload
        {
            match (kind, p.event, p.intent_id) {
                (1, Some(ev), _)
                    if ev
                        .hook_key
                        .as_deref()
                        .is_some_and(|k| k.starts_with(key_prefix)) =>
                {
                    let (key, event) = (
                        ev.hook_key.unwrap_or_default(),
                        ev.event.unwrap_or_default(),
                    );
                    let run = p.run_id.unwrap_or_default();
                    match event.as_str() {
                        "UserPromptSubmit" => ups.insert(run),
                        "Stop" => stops.insert(run),
                        _ => false,
                    };
                    scan.last.insert(
                        key,
                        HookRun {
                            event,
                            at: rec.recorded_at.unwrap_or(0),
                            status: ev.status.unwrap_or_default(),
                            error: ev.error.unwrap_or_default(),
                            stdout: ev.stdout.unwrap_or_default(),
                            log: path.to_path_buf(),
                        },
                    );
                }
                // A live prompt has a version-7 id; a fork replays its parent's prompts with other ids.
                (2, _, Some(id))
                    if p.semantic_kind.and_then(|k| k.kind).as_deref() == Some("chat")
                        && id.as_bytes().get(14) == Some(&b'7') =>
                {
                    intents.push(id)
                }
                (3, _, _) => {
                    scan.version = scan
                        .version
                        .take()
                        .or(p.record.and_then(|r| r.build).and_then(|b| b.semver))
                }
                _ => {}
            }
        }
        line.clear();
    }
    let folder = path
        .parent()
        .and_then(Path::file_name)
        .map(|s| s.to_string_lossy().into_owned());
    for id in intents {
        if stops.contains(&id) && !ups.contains(&id) {
            scan.no_ups.push((folder.clone().unwrap_or_default(), id));
        }
    }
}

fn check_logs(c: &Ctx, ours: &[&Hook], r: &mut Report) {
    let mut logs = transcript::recent_logs(&c.logs, LOGS_SCANNED);
    if logs.is_empty() && c.logs != c.hook_logs {
        logs = transcript::recent_logs(&c.hook_logs, LOGS_SCANNED);
    }
    if logs.is_empty() {
        r.add(Level::Info, format!("no Muse session log under {}: Muse version and rejected outputs cannot be checked yet", c.logs.display()));
        return;
    }
    let prefix = c.settings.to_string_lossy().into_owned();
    // Oldest first, so that the newest run of a hook wins.
    let (mut scan, mut version) = (Scan::default(), None);
    for p in logs.iter().rev() {
        let before = scan.version.take();
        scan_log(p, &prefix, &mut scan);
        version = scan.version.take().or(before).or(version);
    }
    match version.as_deref() {
        Some(v) if v == MUSE_TESTED => r.add(Level::Ok, format!("Muse Code {v} (the version these hooks were tested with)")),
        Some(v) => r.problem(
            Level::Warn,
            format!("Muse Code {v}: the hooks and the log translation were tested only with {MUSE_TESTED}"),
            format!("if something below fails, install Muse Code {MUSE_TESTED}"),
        ),
        None => r.add(Level::Info, "Muse Code version not found in its newest session logs"),
    }
    // The key of a hook ends in a hash of its definition, so a repaired hook is a new key and the old key's failure is
    // the newest record it will ever have. That failure is history once the settings changed after it and the same event
    // has run fine since (a hook of the same event that never changed keeps its own newer record, so it is still found).
    let changed = fs::metadata(&c.settings)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(u64::MAX, |d| d.as_micros() as u64);
    for (key, run) in &scan.last {
        if run.status != "failed" && run.status != "timed_out" {
            continue;
        }
        if run.at < changed
            && scan.last.values().any(|o| {
                o.event == run.event
                    && o.at > changed
                    && matches!(o.status.as_str(), "completed" | "blocked")
            })
        {
            continue;
        }
        let saw = if run.stdout.is_empty() {
            String::new()
        } else {
            format!(" saw: {}", clip(&run.stdout, 100))
        };
        let sid = run
            .log
            .parent()
            .and_then(Path::file_name)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let err = if run.error.is_empty() {
            "no reason given".to_string()
        } else {
            run.error.clone()
        };
        r.problem(
            Level::Fix,
            format!(
                "Muse rejected a hook in {}: {} {} ({err}){saw}\n      seen only in Muse's session log (record hook_run_terminal, status {}): {}",
                c.settings.display(),
                run.event,
                run.status,
                run.status,
                run.log.display()
            ),
            if run.status == "timed_out" {
                "raise the timeout of that hook (torpor-muse init) or find out why it is slow; Muse kills it and goes on without its answer".to_string()
            } else {
                format!("run that hook's command from the settings file by hand with a payload on stdin and read its stdout: it must be one JSON object with camelCase keys only (an unknown key rejects the whole output); the record is in the log of session {sid}, hook {key}")
            },
        );
    }
    let ups_installed = ours.iter().any(|h| h.event == "UserPromptSubmit");
    if ups_installed {
        for (sid, id) in &scan.no_ups {
            r.problem(
                Level::Fix,
                format!("Muse took the prompt {} (session {}) and ran its Stop hook, but the UserPromptSubmit hook never ran: Torpor did not see that prompt", short(id), short(sid)),
                "check the UserPromptSubmit entry in the settings (torpor-muse init rewrites it) and the hook_run records in that session's log",
            );
        }
    }
}

// ---------------------------------------------------------------- heartbeats

fn check_gate(c: &Ctx, vault: &Path, ours: &[&Hook], r: &mut Report) {
    if ours.is_empty() {
        return;
    }
    let (main, obs) = liveness::last_by_event(vault, 20);
    if main.is_empty() {
        r.add(
            Level::Info,
            format!(
                "no heartbeat yet in {}: no hook has run since the install",
                vault.display()
            ),
        );
    } else {
        let list: Vec<String> = main
            .iter()
            .map(|(e, (t, _, n))| format!("{e} {} ago ({n})", ago(c.now_ms, *t)))
            .collect();
        r.add(
            Level::Info,
            format!(
                "last heartbeat per event, main sessions: {}",
                list.join(", ")
            ),
        );
    }
    if !obs.is_empty() {
        let list: Vec<String> = obs
            .iter()
            .map(|(e, (t, _, n))| format!("{e} {n}x (last {} ago)", ago(c.now_ms, *t)))
            .collect();
        r.add(
            Level::Info,
            format!(
                "observer (child) sessions, counted apart: {}",
                list.join(", ")
            ),
        );
    }
    let silent: Vec<&str> = ours
        .iter()
        .map(|h| h.event.as_str())
        .filter(|e| !main.contains_key(*e))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !silent.is_empty() && !main.is_empty() {
        r.add(
            Level::Info,
            format!("registered but no heartbeat yet: {}", silent.join(", ")),
        );
    }
    let found = liveness::sessions(vault)
        .into_iter()
        .filter(|s| s != PROBE_SID)
        .find_map(|sid| {
            let b = liveness::read_beats(vault, &sid);
            liveness::is_main(&b).then_some((sid, b))
        });
    let how =
        "send one prompt in Muse, wait until it has answered, then run torpor-muse doctor again";
    let Some((sid, beats)) = found else {
        r.problem(
            Level::Fix,
            "gate not proven live: no heartbeat from a main session",
            how,
        );
        return;
    };
    let t = liveness::turns(&beats);
    let last_stop = beats.iter().rev().find(|b| b.e == "Stop").map(|b| b.t);
    let written = fs::metadata(&c.settings)
        .and_then(|m| m.modified())
        .ok()
        .map(ms_of);
    if !liveness::gate_live(&beats) {
        r.problem(
            Level::Fix,
            format!("gate not proven live: session {} has {} prompt(s) and no finished turn (no Stop heartbeat from a previous turn)", short(&sid), t.opened),
            format!("{how}; if it stays so, the Stop hook does not run: look at the timeouts and any rejected outputs above"),
        );
    } else if let (Some(stop), Some(w)) = (last_stop, written)
        && stop < w
    {
        r.problem(
            Level::Fix,
            format!("gate not proven live since the settings changed: the last Stop heartbeat is {} old, the settings were written {} ago", ago(c.now_ms, stop), ago(c.now_ms, w)),
            how,
        );
    } else if let Some(stop) = last_stop
        && engine_down_since(vault, stop)
    {
        // the heartbeat is written before the engine is called, so a Stop that failed open still left one
        r.problem(
            Level::Fix,
            format!("gate not live: a hook call at or after the last Stop ({} ago, session {}) went through without the engine, see the hook problem below", ago(c.now_ms, stop), short(&sid)),
            format!("fix the hook problem below; after that {how}"),
        );
    } else {
        r.add(
            Level::Ok,
            format!(
                "gate live: Stop ran {} ago in session {} ({} finished turn(s))",
                ago(c.now_ms, last_stop.unwrap_or(0)),
                short(&sid),
                t.closed
            ),
        );
    }
    if t.no_stop > 0 {
        r.problem(
            Level::Warn,
            format!("{} turn(s) of session {} ended without a Stop (interrupted, step limit, or the Stop hook did not run)", t.no_stop, short(&sid)),
            "if it was not an interrupt: read shim-errors.log in the vault's muse folder and the hook_run records in Muse's log",
        );
    }
    if t.extra_stops > 0 {
        r.add(
            Level::Info,
            format!(
                "{} extra Stop(s) in session {}: Muse runs Stop again after a blocking Stop and when it double-checks its answer, normal",
                t.extra_stops,
                short(&sid)
            ),
        );
    }
    let armed: HashSet<&str> = ours.iter().map(|h| h.event.as_str()).collect();
    for u in liveness::unpaired(&beats, |e| armed.contains(e)) {
        r.problem(
            Level::Warn,
            format!("events without their pair in session {}: {u}", short(&sid)),
            "a hook of that pair failed or timed out (Muse's rejected outputs, if any, are listed above), or the turn was stopped while a tool ran",
        );
    }
    if let Some(st) = transcript::read_state(vault, &sid) {
        if st.lines > 200 && st.users == 0 {
            r.problem(
                Level::Fix,
                format!("{} lines of Muse's log for session {} hold no prompt: its log format may have changed", st.lines, short(&sid)),
                format!("check the Muse Code version (tested only with {MUSE_TESTED}); until the translation is updated, origin and compaction checks are blind"),
            );
        }
        if st.bad > 0 {
            r.problem(Level::Warn, format!("{} line(s) of Muse's log for session {} had an unexpected shape and were skipped", st.bad, short(&sid)), "check the Muse Code version");
        }
    }
}

fn check_errors(vault: &Path, now: u64, r: &mut Report) {
    let f = liveness::shim_dir(vault).join("shim-errors.log");
    let Ok(text) = fs::read_to_string(&f) else {
        return;
    };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let Some(last) = lines.last() else { return };
    let (t, msg) = last.split_once(' ').unwrap_or(("0", last));
    r.problem(
        Level::Warn,
        format!(
            "{} error(s) logged by the hooks, the last {} ago: {}",
            lines.len(),
            ago(now, t.parse().unwrap_or(0)),
            clip(msg, 160)
        ),
        format!("read {}", f.display()),
    );
}

/// How far back a line of problems.log still counts.
const PROBLEMS_RECENT_S: u64 = 24 * 3600;
/// Core keeps the file near 32 KiB, newest last; doctor never reads more than this from its end.
pub const PROBLEMS_READ: u64 = 128 * 1024;

/// The last `max` bytes of a file, from the start of a whole line.
fn tail_of(path: &Path, max: u64) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(max))).ok()?;
    let mut b = Vec::new();
    f.read_to_end(&mut b).ok()?;
    let text = String::from_utf8_lossy(&b).into_owned();
    if len > max {
        return Some(
            text.split_once('\n')
                .map_or(String::new(), |x| x.1.to_string()),
        );
    }
    Some(text)
}

/// What one line of problems.log is worth. FIX: the engine did not run (missing, wrong version, killed at its
/// deadline, exited with an error) or the hook's input was unusable: that call went through without a check. info:
/// routine notes. warn: the rest (an output Muse would reject was cut, Muse's payload changed shape, an unknown message).
fn problem_level(msg: &str) -> Level {
    const DOWN: [&str; 9] = [
        "torpor not found",
        "torpor engine ",
        "torpor hook ",
        "cannot run ",
        "engine.lock has no version",
        "cannot read stdin",
        "stdin is not a JSON object",
        "payload has no session_id",
        "unknown event name",
    ];
    const ROUTINE: [&str; 3] = [
        "Stop block dropped",
        "is not in our schema",
        "gate default not set", // the next session start tries again; the line about the gate says what it is now
    ];
    if DOWN.iter().any(|d| msg.starts_with(d)) || msg.contains(" timed out after ") {
        Level::Fix
    } else if ROUTINE.iter().any(|q| msg.contains(q)) {
        Level::Info
    } else {
        Level::Warn
    }
}

/// Did a hook call at or after `since_ms` (to the second) go through without the engine? Same lines as the fixes below.
fn engine_down_since(vault: &Path, since_ms: u64) -> bool {
    let f = liveness::shim_dir(vault).join("problems.log");
    tail_of(&f, PROBLEMS_READ).is_some_and(|text| {
        text.lines().any(|l| {
            let mut w = l.splitn(4, ' ');
            let t = w.next().and_then(|t| t.parse::<u64>().ok());
            matches!((t, w.nth(2)), (Some(t), Some(msg))
                if t >= since_ms / 1000 && problem_level(msg) == Level::Fix)
        })
    })
}

/// (unix seconds, event, message) of one problems.log line.
type Line<'a> = (u64, &'a str, &'a str);

/// `<vault>/muse/problems.log` (written by the hook core): `<unix seconds> <Event> <session, 8 chars> <message>`.
fn check_problems(vault: &Path, now_ms: u64, r: &mut Report) {
    let f = liveness::shim_dir(vault).join("problems.log");
    let Some(text) = tail_of(&f, PROBLEMS_READ) else {
        return;
    };
    let now_s = now_ms / 1000;
    let (mut fix, mut warn): (Vec<Line>, Vec<Line>) = (vec![], vec![]);
    let (mut routine, mut old) = (0, 0);
    for line in text.lines() {
        let mut w = line.splitn(4, ' ');
        let (Some(t), Some(event), Some(_session), Some(msg)) = (
            w.next().and_then(|t| t.parse::<u64>().ok()),
            w.next(),
            w.next(),
            w.next(),
        ) else {
            continue;
        };
        if now_s.saturating_sub(t) > PROBLEMS_RECENT_S {
            old += 1;
            continue;
        }
        match problem_level(msg) {
            Level::Fix => fix.push((t, event, msg)),
            Level::Info => routine += 1,
            _ => warn.push((t, event, msg)),
        }
    }
    let newest = |v: &[Line]| {
        let &(t, event, msg) = v.iter().max_by_key(|x| x.0).unwrap_or(&(0, "", ""));
        format!(
            "the last {} ago on {event}: {}",
            ago(now_ms, t * 1000),
            clip(msg, 200)
        )
    };
    if !fix.is_empty() {
        r.problem(
            Level::Fix,
            format!(
                "{} hook call(s) went through without a check in the last 24 h (the engine did not run, or the hook's input was unusable), {}",
                fix.len(),
                newest(&fix)
            ),
            format!(
                "fix what that line says (an engine that is missing or of another version: install the pinned one, then torpor-muse init); all lines: {0}; when fixed, empty it so this check starts clean: : > {1}",
                f.display(),
                quote(&f.to_string_lossy())
            ),
        );
    }
    if !warn.is_empty() {
        r.problem(
            Level::Warn,
            format!(
                "{} hook note(s) in the last 24 h, {}",
                warn.len(),
                newest(&warn)
            ),
            format!(
                "read {}: an output Muse would reject was cut, or Muse's payload changed shape (tested only with Muse Code {MUSE_TESTED})",
                f.display()
            ),
        );
    }
    if fix.is_empty() && warn.is_empty() {
        r.add(
            Level::Ok,
            format!(
                "no problem logged by the hooks in the last 24 h ({routine} routine and {old} older line(s) in {})",
                f.display()
            ),
        );
    }
}

// ---------------------------------------------------------------- the probe

struct Ran {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

fn drain(mut r: impl Read + Send + 'static) -> (Arc<Mutex<Vec<u8>>>, std::thread::JoinHandle<()>) {
    let buf = Arc::new(Mutex::new(Vec::new()));
    let b = buf.clone();
    let h = std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = r.read(&mut chunk)
            && n > 0
        {
            if let Ok(mut v) = b.lock() {
                let room = PIPE_CAP.saturating_sub(v.len());
                v.extend_from_slice(&chunk[..n.min(room)]);
            }
        }
    });
    (buf, h)
}

/// Runs `shell -c command` the way Muse runs a hook: cleared environment, payload on stdin, own process group.
fn run_shell(
    shell: &str,
    command: &str,
    env: &[(String, String)],
    cwd: &Path,
    stdin: &[u8],
    timeout: Duration,
) -> Result<Ran, String> {
    let mut cmd = Command::new(shell);
    cmd.arg("-c")
        .arg(command)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child =
        crate::liveness::spawn_retry(&mut cmd).map_err(|e| format!("cannot start {shell}: {e}"))?;
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(stdin); // small; a hook that does not read it is fine
    }
    let (out, h1) = drain(child.stdout.take().ok_or("no stdout")?);
    let (err, h2) = drain(child.stderr.take().ok_or("no stderr")?);
    let t0 = Instant::now();
    let (code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(s)) => break (s.code(), false),
            Ok(None) if t0.elapsed() > timeout => {
                // The only thing doctor ever kills: the process group of the probe it started itself.
                let _ = Command::new("kill")
                    .args(["-s", "KILL", "--", &format!("-{}", child.id())])
                    .status();
                let _ = child.kill();
                let _ = child.wait();
                break (None, true);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("wait: {e}")),
        }
    };
    // A background process that kept the pipe open must not hold doctor: take what arrived within a moment.
    let t1 = Instant::now();
    while !(h1.is_finished() && h2.is_finished()) && t1.elapsed() < Duration::from_millis(500) {
        std::thread::sleep(Duration::from_millis(5));
    }
    let take = |b: &Arc<Mutex<Vec<u8>>>| b.lock().map(|v| v.clone()).unwrap_or_default();
    Ok(Ran {
        code,
        stdout: take(&out),
        stderr: take(&err),
        timed_out,
    })
}

/// What a person must know while the engine's gate is off, as it is since F4b: `doctor` shows it, and so does the line the
/// check-writer prints after it saves a check (src/suggest/attach.rs).
pub const GATE_OFF_TELLS: &str = "A broken check is told to the AI before it says Done; it does not stop the AI. To let checks stop it, run `torpor gate on` in your own terminal.";

/// What a broken check does in the folder doctor is run in (F4b): the engine's own read-only `torpor gate`, run with the
/// engine and the vault the hooks use. An `info`: it is a setting of the user's, not a fault. Left out when the hooks do
/// not say which engine they run (doctor does not guess from its own PATH), when that engine is missing (`check_hooks`
/// says so) and when the vault does not exist yet (nothing is saved, and doctor creates nothing).
fn check_promise_gate(c: &Ctx, vault: &Path, ours: &[&Hook], r: &mut Report) {
    const TELLS: &str = GATE_OFF_TELLS;
    let Some(bin) = ours.iter().find_map(|h| tool_of(&h.command, "TORPOR_BIN")) else {
        return;
    };
    if !bin.is_file() || !vault.is_dir() {
        return;
    }
    let env: Vec<(String, String)> = vec![
        (
            "PATH".into(),
            std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
        ),
        ("HOME".into(), c.home.to_string_lossy().into_owned()),
        ("TORPOR_HOME".into(), vault.to_string_lossy().into_owned()),
    ];
    let cmd = format!("{} gate", quote(&bin.to_string_lossy()));
    let (ok, first) = match run_shell("/bin/sh", &cmd, &env, &c.cwd, b"", Duration::from_secs(5)) {
        Err(e) => (false, e),
        Ok(x) if x.timed_out => (false, "it did not finish in 5 s".to_string()),
        Ok(x) => {
            let text = [x.stdout, x.stderr]
                .map(|b| String::from_utf8_lossy(&b).into_owned())
                .join("\n");
            let line = text.lines().map(str::trim).find(|l| !l.is_empty());
            (x.code == Some(0), line.unwrap_or("").to_string())
        }
    };
    let now = match first.as_str() {
        s if ok && s.starts_with("The gate is on") => "on. A broken check can stop the AI. To let checks only tell it, run `torpor gate off` in your own terminal.".to_string(),
        s if ok && s.starts_with("The gate is off") => format!("off. {TELLS}"),
        s if ok && s.starts_with("The gate is advisory") => format!("off (advisory). {TELLS}"),
        s => format!("not read: {}", clip(s, 120)),
    };
    r.add(Level::Info, format!("torpor gate (this folder): {now}"));
}

fn check_probe(c: &Ctx, ours: &[&Hook], r: &mut Report) {
    let probe_vault = c
        .temps
        .first()
        .cloned()
        .unwrap_or_default()
        .join("torpor-muse-doctor-probe");
    let shell = c.shell.clone().unwrap_or_else(|| "/bin/sh".into());
    let mut env: Vec<(String, String)> = [
        "PATH", "USER", "LOGNAME", "TMPDIR", "TEMP", "TMP", "LANG", "LC_ALL", "TERM",
    ]
    .iter()
    .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
    .collect();
    env.push(("HOME".into(), c.home.to_string_lossy().into_owned()));
    env.push(("SHELL".into(), shell.clone()));
    env.push(("MUSE_SESSION_ID".into(), PROBE_SID.into()));
    env.push(("MUSE_THREAD_ID".into(), PROBE_SID.into()));
    if !env.iter().any(|(k, _)| k == "PATH") {
        env.push(("PATH".into(), "/usr/local/bin:/usr/bin:/bin".into()));
    }
    // SubagentStart: a registered event that only records a heartbeat (in the probe vault), so the engine is not called.
    let payload = json!({
        "hook_event_name": "SubagentStart", "session_id": PROBE_SID, "cwd": c.cwd,
        "transcript_path": null, "model": "probe", "model_provider": "probe", "permission_mode": "default",
    })
    .to_string();
    let cmds: BTreeSet<String> = ours
        .iter()
        .filter_map(|h| probe_command(&h.command, &probe_vault, "SubagentStart"))
        .collect();
    for cmd in cmds {
        let ran = match run_shell(
            &shell,
            &cmd,
            &env,
            &c.cwd,
            payload.as_bytes(),
            c.probe_timeout,
        ) {
            Ok(x) => x,
            Err(e) => {
                r.problem(
                    Level::Fix,
                    format!("probe: {e}"),
                    "set SHELL to a working shell",
                );
                continue;
            }
        };
        let (out, err) = (
            String::from_utf8_lossy(&ran.stdout),
            String::from_utf8_lossy(&ran.stderr),
        );
        let object = serde_json::from_slice::<Value>(&ran.stdout).is_ok_and(|v| v.is_object());
        if ran.timed_out {
            r.problem(Level::Fix, format!("probe: `{shell} -c` did not finish in {}s (the hook command, or the shell's start-up files, hang)", c.probe_timeout.as_secs()), "run the same command by hand and see where it stops");
        } else if ran.code != Some(0) {
            r.problem(Level::Fix, format!("probe: the hook command exits with {:?}: {}", ran.code, clip(&err, 200)), "run `torpor-muse hook SubagentStart < /dev/null` by hand; if the shell cannot find torpor-muse, run torpor-muse init again");
        } else if !out.trim().is_empty() && !object {
            r.problem(
                Level::Fix,
                format!("probe: `{shell} -c` printed text that is not one JSON object: {}\n      Muse reads all of stdout as the hook's answer: on UserPromptSubmit and SessionStart it hands that text to the model as context, on Stop it ignores the answer", clip(&out, 100)),
                "make the shell's non-interactive start-up files print nothing (zsh reads ~/.zshenv for every `-c`; bash reads BASH_ENV): guard prints with [ -t 1 ] or move them to the interactive file",
            );
        } else {
            r.add(
                Level::Ok,
                format!(
                    "probe: `{} -c` runs the hook command with a cleared environment and prints {}",
                    shell,
                    if out.trim().is_empty() {
                        "nothing"
                    } else {
                        "one JSON object"
                    }
                ),
            );
        }
    }
}

// ---------------------------------------------------------------- where the logs are

/// The check-writer that starts after a prompt reads its key from a file the config names; without one it stays off
/// and shows nothing. A warning: the gate itself works without it.
fn check_writer(r: &mut Report) {
    if let Some(why) = crate::suggest::writer_off(false) {
        r.problem(
            Level::Warn,
            format!("check-writer off: {why}"),
            "put your Meta API key alone on one line in a file only you can read (chmod 600), then add `key_file = ~/that/file` to ~/.config/torpor-muse/config (chmod 600); a META_API_KEY in your shell does not reach Muse Code's hooks",
        );
    }
}

fn check_xdg(c: &Ctx, r: &mut Report) {
    let Some(x) = &c.xdg_data else { return };
    let seen = x.join("muse/sessions");
    if seen == c.hook_logs {
        return;
    }
    let link = c
        .hook_logs
        .parent()
        .map_or_else(PathBuf::new, |p| p.to_path_buf());
    let (l, tail, how) = if c.hook_logs.is_dir() {
        (
            Level::Warn,
            "both places exist, the hooks use only the second",
            "start Muse without XDG_DATA_HOME so that its sessions go where the hooks look"
                .to_string(),
        )
    } else {
        (
            Level::Fix,
            "the hooks cannot find any session log",
            format!(
                "ln -s {} {}   (or start Muse without XDG_DATA_HOME)",
                quote(&x.join("muse").to_string_lossy()),
                quote(&link.to_string_lossy())
            ),
        )
    };
    r.problem(
        l,
        format!("XDG_DATA_HOME={} is set here: Muse keeps sessions in {} but hooks run with a cleared environment and look in {} ({tail})", x.display(), seen.display(), c.hook_logs.display()),
        how,
    );
}

// ---------------------------------------------------------------- orphans of killed checks

struct Proc {
    pid: u32,
    pgid: u32,
    args: String,
}

fn processes() -> Vec<Proc> {
    let Ok(out) = Command::new("ps")
        .args(["-A", "-ww", "-o", "pid=,pgid=,command="])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (pid, rest) = l.trim_start().split_once(' ')?;
            let (pgid, args) = rest.trim_start().split_once(' ')?;
            Some(Proc {
                pid: pid.parse().ok()?,
                pgid: pgid.parse().ok()?,
                args: args.trim_start().to_string(),
            })
        })
        .collect()
}

fn ms_of(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A killed check can leave its app, its runner, its browser and a `tp-...` folder behind (README:213). The engine's
/// scratch folders are `<vault>/<workspace>/scratch/<run>` (with `*.pgid` files naming the process groups) and the
/// browser's is `tp-*` in the temp folder.
fn check_orphans(c: &Ctx, vault: &Path, r: &mut Report) {
    let mut dirs: Vec<PathBuf> = Vec::new();
    for w in fs::read_dir(vault).into_iter().flatten().flatten() {
        let name = w.file_name().to_string_lossy().into_owned();
        if (name.len() == 16 || name.len() == 20) && name.bytes().all(|b| b.is_ascii_hexdigit()) {
            dirs.extend(
                fs::read_dir(w.path().join("scratch"))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|e| e.path()),
            );
        }
    }
    let me = fs::metadata(&c.home).map(|m| m.uid()).ok();
    for t in &c.temps {
        for e in fs::read_dir(t).into_iter().flatten().flatten() {
            let owned = me.is_none_or(|u| e.metadata().is_ok_and(|m| m.uid() == u));
            if e.file_name().to_string_lossy().starts_with("tp-") && owned {
                dirs.push(e.path());
            }
        }
    }
    dirs.retain(|d| {
        d.is_dir()
            && fs::metadata(d)
                .and_then(|m| m.modified())
                .is_ok_and(|m| c.now_ms.saturating_sub(ms_of(m)) > ORPHAN_AGE_S * 1000)
    });
    dirs.sort();
    dirs.dedup();
    if dirs.is_empty() {
        return;
    }
    let procs = processes();
    let own_pgid = procs
        .iter()
        .find(|p| p.pid == std::process::id())
        .map(|p| p.pgid);
    let have_proc = Path::new("/proc/self").exists();
    for d in dirs {
        let ds = d.to_string_lossy().into_owned();
        let holders: Vec<&Proc> = procs
            .iter()
            .filter(|p| p.pid != std::process::id())
            .filter(|p| {
                p.args.contains(&ds)
                    || (have_proc
                        && fs::read_link(format!("/proc/{}/cwd", p.pid))
                            .is_ok_and(|w| w.starts_with(&d)))
            })
            .collect();
        let listed: BTreeSet<u32> = fs::read_dir(&d)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".pgid"))
            .filter_map(|e| fs::read_to_string(e.path()).ok()?.trim().parse().ok())
            .collect();
        let mut cmds: Vec<String> = Vec::new();
        // A group is killed only when a process that works in this folder belongs to it (a recycled number is not ours).
        let groups: BTreeSet<u32> = holders
            .iter()
            .map(|p| p.pgid)
            .filter(|g| listed.contains(g) && *g > 1 && Some(*g) != own_pgid)
            .collect();
        cmds.extend(groups.iter().map(|g| format!("kill -9 -- -{g}")));
        let loose: Vec<String> = holders
            .iter()
            .filter(|p| !groups.contains(&p.pgid) && Some(p.pgid) != own_pgid)
            .map(|p| p.pid.to_string())
            .collect();
        if !loose.is_empty() {
            cmds.push(format!("kill -9 {}", loose.join(" ")));
        }
        cmds.push(format!("rm -rf {}", quote(&ds)));
        let age = fs::metadata(&d)
            .and_then(|m| m.modified())
            .map_or("?".into(), |m| ago(c.now_ms, ms_of(m)));
        let (l, what) = if holders.is_empty() {
            (
                Level::Warn,
                format!(
                    "leftover folder of a killed check: {ds} ({age} old, no process works in it)"
                ),
            )
        } else {
            let list: Vec<String> = holders
                .iter()
                .map(|p| format!("{} {}", p.pid, clip(&p.args, 60)))
                .collect();
            (
                Level::Fix,
                format!(
                    "a killed check left processes running for {ds} ({age} old): {}",
                    list.join("; ")
                ),
            )
        };
        r.problem(l, what, cmds.join("; "));
    }
}

// ---------------------------------------------------------------- small helpers

fn ago(now_ms: u64, t_ms: u64) -> String {
    let s = now_ms.saturating_sub(t_ms) / 1000;
    match s {
        0..=119 => format!("{s}s"),
        120..=7199 => format!("{}m", s / 60),
        7200..=172_799 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86_400),
    }
}

/// One line, no control characters, at most `n` characters.
fn clip(s: &str, n: usize) -> String {
    let one: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let one = one.trim();
    if one.chars().count() > n {
        format!("{}...", one.chars().take(n).collect::<String>())
    } else {
        one.to_string()
    }
}

fn short(id: &str) -> String {
    id.chars().take(13).collect()
}

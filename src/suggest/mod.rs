//! `suggest "<what works>"`: Muse Spark (Responses API, text.format json_schema) writes a check from the user's words;
//! the engine proves it or keeps it advisory. `trigger` starts a detached run from UserPromptSubmit. Owner: S3b-writer.
//!
//! Exit codes of `run`: 0 a promise was saved (proven or advisory), 3 nothing saved (the reason is printed),
//! 1 the run could not work (no key, no engine, no state folder, another run holds the lock), 2 wrong use.
//! Every run that got past the lock leaves `<state>/ws/<project>/result.json` (written whole, mode 0600).
mod attach;
mod client;
mod prompt;
mod repo;
mod schema;
mod state;

use client::{Bad, Key};
use schema::Start;
use serde_json::{Value, json};
use state::env;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const DEFAULT_MODEL: &str = "muse-spark-1.3";
/// Low reasoning effort still spends about 500 to 1000 tokens before the answer, so the budget is large.
const MAX_OUT: u32 = 6000;
const CACHE_KEY: &str = concat!("torpor-muse-writer-", env!("CARGO_PKG_VERSION"));
const HARD_TIMEOUT: Duration = Duration::from_secs(720);
const HTTP_TIMEOUT: Duration = Duration::from_secs(60);
const WATCHDOG_GRACE: Duration = Duration::from_secs(3);
const WORDS_MAX: usize = 1500;
const RETRY_WAIT_MAX_S: u64 = 20;
const RETRY_WAIT_DEFAULT_S: u64 = 2;
const MIN_TIME_FOR_RETRY: Duration = Duration::from_secs(90);

/// Set by whoever writes the final result first (the run, or the watchdog); the other then stands aside.
static DONE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Proven,
    Advisory,
    Rejected,
    Skipped,
    Error,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Proven => "proven",
            Kind::Advisory => "advisory",
            Kind::Rejected => "rejected",
            Kind::Skipped => "skipped",
            Kind::Error => "error",
        }
    }
}

struct Outcome {
    kind: Kind,
    saved: bool,
    promise: Option<u32>,
    message: String,
}

impl Outcome {
    fn new(kind: Kind, message: impl Into<String>) -> Self {
        Self {
            kind,
            saved: false,
            promise: None,
            message: message.into(),
        }
    }
}

/// What a run knows about itself, for the result file and for keeping the key out of every text it shows.
struct Ctx {
    started: Instant,
    deadline: Instant,
    result: Option<PathBuf>,
    session: Option<String>,
    key: Option<Key>,
    model: String,
    calls: u32,
    usd: f64,
}

impl Ctx {
    fn time_left(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Text from the model, the server or the engine, made safe to print and to store.
    fn safe(&self, s: &str, max: usize) -> String {
        let t = client::clean(s, max);
        self.key.as_ref().map_or_else(|| t.clone(), |k| k.scrub(&t))
    }
}

struct Plan {
    root: PathBuf,
    ledger_dir: PathBuf,
    model: String,
    key: Key,
    base: client::Base,
    cap: f64,
    starts: Vec<Start>,
    excerpt: String,
    words: String,
    http: Duration,
}

fn usage() -> u8 {
    eprintln!("usage: torpor-muse suggest \"<what works, in your words>\"");
    2
}

fn bail(msg: &str) -> u8 {
    eprintln!("torpor-muse suggest: {msg}");
    1
}

/// A long prompt keeps its start and its end.
fn cap_words(s: &str) -> String {
    let s = s.trim();
    let n = s.chars().count();
    if n <= WORDS_MAX {
        return s.to_string();
    }
    let head: String = s.chars().take(WORDS_MAX * 2 / 3).collect();
    let tail: String = s.chars().skip(n - WORDS_MAX / 3).collect();
    format!("{head} [...] {tail}")
}

fn seconds(name: &str) -> Option<Duration> {
    env(name)?
        .parse::<u64>()
        .ok()
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
}

/// How long to wait before the one retry, or `None` when the wait would pass the budget.
fn backoff(bad: &Bad) -> Option<Duration> {
    match bad {
        Bad::RateLimited { wait, .. } => {
            let w = wait.unwrap_or(RETRY_WAIT_DEFAULT_S);
            (w <= RETRY_WAIT_MAX_S).then(|| Duration::from_secs(w))
        }
        _ => Some(Duration::ZERO),
    }
}

fn model_name() -> Result<String, String> {
    let m = env("TORPOR_MUSE_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into());
    let plain = m.len() <= 64
        && m.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if plain {
        Ok(m)
    } else {
        Err("TORPOR_MUSE_MODEL is not a model id".into())
    }
}

fn request(model: &str, input: &str, starts: &[Start]) -> Value {
    json!({
        "model": model,
        "instructions": prompt::INSTRUCTIONS,
        "input": input,
        "reasoning": {"effort": "low"},
        "max_output_tokens": MAX_OUT,
        "prompt_cache_key": CACHE_KEY,
        "text": {"format": schema::response_format(starts)},
    })
}

pub fn run(words: &str) -> u8 {
    let started = Instant::now();
    let hook = env("TORPOR_MUSE_FROM_HOOK");
    let words = match (words.trim(), &hook) {
        ("-", Some(_)) => env("TORPOR_MUSE_WORDS").unwrap_or_default(),
        ("-", None) => String::new(),
        (w, _) => w.to_string(),
    };
    if words.trim().is_empty() {
        return usage();
    }
    let base = match client::api_base() {
        Ok(b) => b,
        Err(e) => return bail(&e),
    };
    // The short test limits count only against a loopback address; a real run cannot be given them by accident.
    let (http, hard) = if base.loopback {
        (
            seconds("TORPOR_MUSE_HTTP_TIMEOUT_S").unwrap_or(HTTP_TIMEOUT),
            seconds("TORPOR_MUSE_HARD_TIMEOUT_S").unwrap_or(HARD_TIMEOUT),
        )
    } else {
        (HTTP_TIMEOUT, HARD_TIMEOUT)
    };
    let root = match repo::resolve_root(None) {
        Ok(r) => r,
        Err(e) => return bail(&e),
    };
    let ws = match state::ws_dir(&root) {
        Ok(d) => d,
        Err(e) => return bail(&e),
    };
    let _lock = match state::try_lock(&ws) {
        Ok(Some(l)) => l,
        Ok(None) => return bail("another run is already working on this folder"),
        Err(e) => return bail(&e),
    };
    let mut c = Ctx {
        started,
        deadline: started + hard,
        result: Some(ws.join("result.json")),
        session: hook.clone(),
        key: None,
        model: DEFAULT_MODEL.into(),
        calls: 0,
        usd: 0.0,
    };
    watchdog(&c);
    let outcome = match plan(&mut c, &root, &words, hook.is_some(), base, http) {
        Ok(p) => work(&mut c, &p),
        Err(o) => o,
    };
    finish(&c, outcome)
}

/// Everything that must be true before money is spent.
fn plan(
    c: &mut Ctx,
    root: &Path,
    words: &str,
    hook: bool,
    base: client::Base,
    http: Duration,
) -> Result<Plan, Outcome> {
    let fail = |m: String| Outcome::new(Kind::Error, m);
    let synthetic = repo::is_synthetic(root);
    let model = model_name().map_err(fail)?;
    c.model.clone_from(&model);
    if model.ends_with("-contributor") && !synthetic {
        return Err(fail(format!(
            "the model `{model}` may train on what it is sent, so it is refused unless the folder holds a `.synthetic` marker"
        )));
    }
    let cfg = client::load_config().map_err(fail)?;
    let cap = client::day_cap(&cfg).map_err(fail)?;
    let starts = repo::start_choices(root);
    if starts.is_empty() {
        return Err(Outcome::new(
            Kind::Advisory,
            "cannot make a check: package.json has no start, dev, serve or server script to start the app with",
        ));
    }
    attach::present(root).map_err(fail)?;
    if hook {
        let turn = attach::hook_turn(root).map_err(|why| Outcome::new(Kind::Skipped, why))?;
        if attach::promised_at(root, turn.n) {
            return Err(Outcome::new(
                Kind::Skipped,
                "a check from your words at this turn is already saved",
            ));
        }
    }
    let key = client::load_key(&cfg).map_err(fail)?;
    c.key = Some(key.clone());
    Ok(Plan {
        root: root.to_path_buf(),
        ledger_dir: state::state_dir().map_err(fail)?,
        model,
        key,
        base,
        cap,
        starts,
        excerpt: repo::excerpt(root, synthetic),
        words: repo::redact(&cap_words(words), synthetic),
        http,
    })
}

/// One call to the model per try, at most two tries, and the engine's verdict on the check.
fn work(c: &mut Ctx, p: &Plan) -> Outcome {
    let mut retry: Option<(String, String)> = None;
    for attempt in 0..2u32 {
        let last = attempt == 1;
        let nonce = prompt::nonce();
        let rejected = retry.as_ref().map(|(a, m)| prompt::Rejected {
            answer: a,
            message: m,
        });
        let input = prompt::input(&nonce, &p.starts, &p.excerpt, &p.words, rejected.as_ref());
        let body = request(&p.model, &input, &p.starts).to_string();
        let reading = match ask(c, p, &body, attempt) {
            Ok(r) => r,
            Err(o) => return o,
        };
        let text = match reading.outcome {
            Ok(t) => t,
            Err(bad) => {
                let wait = backoff(&bad)
                    .filter(|_| bad.retryable() && !last && c.time_left() > MIN_TIME_FOR_RETRY);
                let Some(wait) = wait else {
                    let kind = if bad.rejected() {
                        Kind::Rejected
                    } else {
                        Kind::Advisory
                    };
                    return Outcome::new(kind, bad.reason());
                };
                std::thread::sleep(wait);
                retry = None;
                continue;
            }
        };
        let answer = match schema::parse(&text) {
            Ok(a) => a,
            Err(e) if last => return Outcome::new(Kind::Rejected, e),
            Err(e) => {
                retry = Some((c.safe(&text, 3000), e));
                continue;
            }
        };
        match answer.verdict {
            schema::Verdict::NotAClaim => {
                let why = c.safe(&answer.reason, 300);
                return Outcome::new(
                    Kind::Advisory,
                    format!("nothing to check: not a statement that something works ({why})"),
                );
            }
            schema::Verdict::CannotCheck => {
                let why = c.safe(&answer.reason, 300);
                return Outcome::new(
                    Kind::Advisory,
                    format!("cannot check this with HTTP requests ({why})"),
                );
            }
            schema::Verdict::Check => {}
        }
        // what the steps cannot prove, in the model's words (for a `check` the reason is only that)
        let gap = c.safe(&answer.reason, 200);
        let check = match schema::to_check(&answer, &p.starts) {
            Ok(v) => v,
            Err(e) if last => return Outcome::new(Kind::Rejected, e),
            Err(e) => {
                retry = Some((c.safe(&text, 3000), e));
                continue;
            }
        };
        let said = match attach::add(
            &p.root,
            &check,
            c.time_left().saturating_sub(Duration::from_secs(5)),
        ) {
            Ok(a) => a,
            Err(e) => return Outcome::new(Kind::Error, e),
        };
        let turned_down = match said {
            attach::Added::Proven { id, line } => return saved(Kind::Proven, id, &line, &gap),
            attach::Added::Advisory { id, line } => return saved(Kind::Advisory, id, &line, &gap),
            attach::Added::Lint(m) => m,
            attach::Added::NotSaved { why, retry: true } => why,
            attach::Added::NotSaved { why, retry: false } => {
                return Outcome::new(
                    Kind::Advisory,
                    format!("Torpor did not save the check: {why}"),
                );
            }
            attach::Added::Failed(m) => return Outcome::new(Kind::Advisory, m),
        };
        if last || c.time_left() < MIN_TIME_FOR_RETRY {
            return Outcome::new(
                Kind::Advisory,
                format!("the engine turned the check down: {turned_down}"),
            );
        }
        retry = Some((c.safe(&text, 3000), turned_down));
    }
    Outcome::new(Kind::Advisory, "no check was made")
}

fn saved(kind: Kind, promise: Option<u32>, line: &str, gap: &str) -> Outcome {
    let message = if gap.is_empty() {
        line.to_string()
    } else {
        format!("{line} Not covered by this check: {gap}")
    };
    Outcome {
        kind,
        saved: true,
        promise,
        message,
    }
}

/// One response from the model, put into the ledger whatever it was, under the ledger's lock from the cap check
/// to the row (so two runs cannot both pass the cap).
fn ask(c: &mut Ctx, p: &Plan, body: &str, attempt: u32) -> Result<client::Reading, Outcome> {
    let err = |m: String| Outcome::new(Kind::Error, m);
    let mut ledger = crate::ledger::Ledger::open(&p.ledger_dir).map_err(err)?;
    let worst = ledger
        .preflight(&p.model, body.len(), MAX_OUT, p.cap)
        .map_err(|m| Outcome::new(Kind::Advisory, m))?;
    c.calls += 1;
    let timeout = p.http.min(c.time_left());
    let sent = Instant::now();
    let reading = match client::post(&p.base, &p.key, body, timeout) {
        Ok(reply) => client::interpret(&reply, worst),
        Err(n) => client::from_net(n, worst),
    };
    // the call's own time goes into the note, so the ledger shows how long the model took
    let secs = sent.elapsed().as_secs_f64();
    let note = match &reading.outcome {
        Ok(_) => format!("attempt {}, {secs:.1}s", attempt + 1),
        Err(bad) => format!(
            "attempt {}, {secs:.1}s: {}",
            attempt + 1,
            c.safe(&bad.reason(), 120)
        ),
    };
    c.usd += ledger
        .record(&p.model, &reading.status, reading.charge, &note)
        .map_err(err)?;
    Ok(reading)
}

/// Ends the run if it takes too long: stops the engine call, writes the result, exits.
fn watchdog(c: &Ctx) {
    let (deadline, result, session) = (c.deadline, c.result.clone(), c.session.clone());
    std::thread::spawn(move || {
        // A call in flight times out at the deadline itself and is written to the ledger; the watchdog is for what
        // does not (a stuck engine, a stuck write), so it waits a little longer.
        std::thread::sleep(deadline.saturating_duration_since(Instant::now()) + WATCHDOG_GRACE);
        if DONE.swap(true, Ordering::SeqCst) {
            return;
        }
        attach::kill_running();
        let message = "the run took too long and was stopped";
        if let Some(p) = result {
            let row = json!({"v": 1, "at": crate::ledger::iso(crate::ledger::now_secs()), "outcome": Kind::Advisory.word(),
                "saved": false, "promise": null, "message": message, "session": session});
            let _ = state::write_atomic(&p, row.to_string().as_bytes());
        }
        println!("{}: {message}", Kind::Advisory.word());
        std::process::exit(3);
    });
}

fn finish(c: &Ctx, o: Outcome) -> u8 {
    if DONE.swap(true, Ordering::SeqCst) {
        // the watchdog is ending the process
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    let message = c.safe(&o.message, 1500);
    if let Some(p) = &c.result {
        let row = json!({
            "v": 1, "at": crate::ledger::iso(crate::ledger::now_secs()), "outcome": o.kind.word(), "saved": o.saved,
            "promise": o.promise, "message": message, "session": c.session, "model": c.model, "calls": c.calls,
            "usd": (c.usd * 1e6).round() / 1e6, "ms": u64::try_from(c.started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
        if let Err(e) = state::write_atomic(p, row.to_string().as_bytes()) {
            eprintln!("torpor-muse suggest: {e}");
        }
    }
    if o.kind == Kind::Error {
        eprintln!("torpor-muse suggest: {message}");
    } else {
        println!("{}: {message}", o.kind.word());
    }
    match (o.saved, o.kind) {
        (true, _) => 0,
        (false, Kind::Error) => 1,
        (false, _) => 3,
    }
}

/// hook.rs calls this on UserPromptSubmit (main session) after the engine call; returns at once (detached run).
#[allow(dead_code)] // hook.rs calls this once S3a-core is merged
pub fn trigger(session_id: &str, cwd: &str, prompt: &str) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = trigger_with(&exe, session_id, cwd, prompt);
    }
}

/// Why the trigger did not start a run (it never says more than that, and never fails the hook).
#[derive(Debug, PartialEq)]
struct Skip(&'static str);

/// Cheap, local checks only (no network, no engine, no model): a prompt with something in it, a usable folder, a key
/// file that only the user can read, no run under way here. Then a detached `suggest -` in its own process group
/// with the prompt in its environment (not on its command line, where every user could read it) and without the key.
/// The judgment whether the prompt says "this works" is the model's, in the run.
fn trigger_with(exe: &Path, session_id: &str, cwd: &str, prompt: &str) -> Result<Child, Skip> {
    if prompt.chars().filter(|c| !c.is_whitespace()).count() < 3 {
        return Err(Skip("the prompt is too short"));
    }
    let root = repo::resolve_root(Some(cwd)).map_err(|_| Skip("no usable project folder"))?;
    let cfg = client::load_config().map_err(|_| Skip("the config file is not usable"))?;
    if !client::key_source_ok(&cfg) {
        return Err(Skip("no key file in the config"));
    }
    let ws = state::ws_dir(&root).map_err(|_| Skip("no state folder"))?;
    match state::try_lock(&ws) {
        Ok(Some(_free)) => {}
        _ => return Err(Skip("a run is already under way here")),
    }
    Command::new(exe)
        .args(["suggest", "-"])
        .current_dir(&root)
        .env_remove("META_API_KEY")
        .env_remove("CLAUDECODE")
        .env("TORPOR_MUSE_FROM_HOOK", session_id)
        .env("TORPOR_MUSE_WORDS", cap_words(prompt))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|_| Skip("cannot start the run"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_long_prompt_keeps_its_start_and_its_end() {
        assert_eq!(cap_words("  login works  "), "login works");
        let long = format!("{}{}{}", "a".repeat(1200), "m".repeat(900), "z".repeat(600));
        let c = cap_words(&long);
        assert!(c.chars().count() <= WORDS_MAX + 8);
        assert!(c.starts_with("aaa") && c.ends_with("zzz") && c.contains("[...]"));
        let accented = "é".repeat(3000);
        assert!(
            cap_words(&accented).chars().count() <= WORDS_MAX + 8,
            "cuts on characters, not bytes"
        );
    }

    #[test]
    fn the_rate_limit_wait_follows_retry_after_within_the_budget() {
        let limited = |w| Bad::RateLimited {
            wait: w,
            msg: String::new(),
        };
        assert_eq!(backoff(&limited(Some(2))), Some(Duration::from_secs(2)));
        assert_eq!(
            backoff(&limited(None)),
            Some(Duration::from_secs(RETRY_WAIT_DEFAULT_S))
        );
        assert_eq!(
            backoff(&limited(Some(RETRY_WAIT_MAX_S))),
            Some(Duration::from_secs(RETRY_WAIT_MAX_S))
        );
        assert_eq!(backoff(&limited(Some(RETRY_WAIT_MAX_S + 1))), None);
        assert_eq!(backoff(&Bad::Timeout), Some(Duration::ZERO));
    }

    #[test]
    fn the_request_asks_for_low_effort_a_cache_key_and_a_strict_schema() {
        let starts = vec![Start {
            key: "start".into(),
            cmd: "npm start --silent".into(),
        }];
        let r = request("muse-spark-1.3", "data", &starts);
        assert_eq!(r["reasoning"]["effort"], "low");
        assert_eq!(r["text"]["format"]["type"], "json_schema");
        assert_eq!(r["text"]["format"]["strict"], true);
        assert!(
            r["prompt_cache_key"]
                .as_str()
                .unwrap()
                .starts_with("torpor-muse-writer-")
        );
        assert_eq!(r["max_output_tokens"], MAX_OUT);
        assert!(r.get("tools").is_none() && r.get("start_cmd").is_none());
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("tm-trigger-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_trigger_returns_fast_and_starts_a_detached_run_without_the_key_or_the_prompt_on_the_command_line()
     {
        let d = tmp("spawn");
        let (home, proj, state) = (d.join("home"), d.join("proj"), d.join("state"));
        for x in [&home, &proj] {
            fs::create_dir_all(x).unwrap();
        }
        let keyfile = d.join("key");
        fs::write(&keyfile, "LLM_notarealkey000\n").unwrap();
        fs::set_permissions(&keyfile, fs::Permissions::from_mode(0o600)).unwrap();
        fs::create_dir_all(home.join(".config/torpor-muse")).unwrap();
        let cfg = home.join(".config/torpor-muse/config");
        fs::write(&cfg, format!("key_file = {}\n", keyfile.display())).unwrap();
        fs::set_permissions(&cfg, fs::Permissions::from_mode(0o600)).unwrap();
        let fake = d.join("fake-muse");
        fs::write(
            &fake,
            "#!/bin/sh\nsleep 0.3\necho \"$@\" > seen-args\nenv > seen-env\npwd > seen-pwd\n",
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();

        let _g = state::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: every test that touches the environment holds ENV_LOCK.
        unsafe {
            std::env::set_var("HOME", &home);
            std::env::set_var("TORPOR_MUSE_STATE", &state);
            std::env::set_var("META_API_KEY", "LLM_envkeyshouldnotpass");
            std::env::remove_var("CLAUDE_PROJECT_DIR");
        }
        let prompt = "the login works now, thanks";
        let cwd = proj.to_string_lossy().into_owned();
        let mut times = Vec::new();
        let mut children = Vec::new();
        for i in 0..5 {
            let t = Instant::now();
            let r = trigger_with(&fake, "sess-1", &cwd, prompt);
            times.push(t.elapsed());
            children.push(r.unwrap_or_else(|s| panic!("trigger {i} did not start a run: {s:?}")));
        }
        times.sort();
        assert!(
            times[2] < Duration::from_millis(50),
            "median {:?}",
            times[2]
        );
        for mut ch in children {
            ch.wait().unwrap();
        }
        let seen = |n: &str| fs::read_to_string(proj.join(n)).unwrap_or_default();
        assert_eq!(
            seen("seen-args").trim(),
            "suggest -",
            "the prompt is not on the command line"
        );
        let env_seen = seen("seen-env");
        assert!(env_seen.contains("TORPOR_MUSE_FROM_HOOK=sess-1"));
        assert!(env_seen.contains(&format!("TORPOR_MUSE_WORDS={prompt}")));
        assert!(!env_seen.contains("META_API_KEY"), "the child gets no key");
        assert!(seen("seen-pwd").contains("proj"));

        // too short, no key file, a run under way: no spawn
        assert_eq!(
            trigger_with(&fake, "s", &cwd, "ok").unwrap_err(),
            Skip("the prompt is too short")
        );
        let ws = state::ws_dir(&repo::resolve_root(Some(&cwd)).unwrap()).unwrap();
        let held = state::try_lock(&ws).unwrap().unwrap();
        assert_eq!(
            trigger_with(&fake, "s", &cwd, prompt).unwrap_err(),
            Skip("a run is already under way here")
        );
        drop(held);
        fs::set_permissions(&keyfile, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            trigger_with(&fake, "s", &cwd, prompt).unwrap_err(),
            Skip("no key file in the config")
        );
        unsafe {
            std::env::remove_var("META_API_KEY");
            std::env::remove_var("TORPOR_MUSE_STATE");
        }
        let _ = fs::remove_dir_all(&d);
    }
}

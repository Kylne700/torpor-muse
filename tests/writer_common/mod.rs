//! Shared by tests/writer_*.rs: the mock Meta API as a child process, a synthetic workspace, a scripted fake
//! engine, and a runner for `torpor-muse suggest` and `say` with a clean environment. Owner: S3b-writer.
#![allow(dead_code)]

use serde_json::{Value, json};
use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

/// Not a real key: the tests check that it never shows up anywhere.
pub const FAKE_KEY: &str = "LLM_fake-key-for-the-tests-0123456789abcdef";

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn path_var() -> String {
    std::env::var("PATH").unwrap_or_default()
}

/// The real engine, for the tests that need it: `TORPOR_BIN`, else `.engine/torpor` (where the gate puts it).
pub fn real_engine() -> Option<PathBuf> {
    let p = std::env::var_os("TORPOR_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join(".engine/torpor"));
    let ok = Command::new(&p)
        .arg("version")
        .env_remove("CLAUDECODE")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        eprintln!(
            "SKIPPED: no working Torpor engine at {} (set TORPOR_BIN)",
            p.display()
        );
    }
    ok.then_some(p)
}

/// A valid answer for the fixture app, in the shape of the writer schema.
pub fn good_answer() -> Value {
    json!({
        "verdict": "check", "reason": "", "name": "login works", "why": "login works for alice",
        "start_script": "start", "ready_path": "/", "node_env": "",
        "steps": [
            {"method": "POST", "path": "/api/login",
             "body_json": "{\"user\":\"alice\",\"password\":\"wonderland-demo-pass\"}",
             "expect_status": 200, "json_has": ["user"], "json_types": [], "body_contains": "",
             "why": "login works for alice with the right password"},
            {"method": "GET", "path": "/api/me", "body_json": "", "expect_status": 200,
             "json_has": ["user"], "json_types": [], "body_contains": "alice",
             "why": "login works: the account page shows alice"}
        ]
    })
}

/// Days since 1970-01-01 to a civil date (the proleptic Gregorian calendar).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Today's date in UTC as `YYYY-MM-DD`, the way the ledger writes it.
pub fn utc_day() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (y, m, d) = civil(i64::try_from(secs / 86_400).unwrap());
    format!("{y:04}-{m:02}-{d:02}")
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            fs::copy(e.path(), dest).unwrap();
        }
    }
}

/// The mock Meta API (mock/server.mjs) as a child process. Dropping it kills exactly that child.
pub struct Mock {
    child: Child,
    _stdout: BufReader<ChildStdout>,
    pub port: u16,
    pub log: PathBuf,
    pub text: PathBuf,
}

impl Mock {
    /// `fixtures`: the mock's `--fixture` list. `text`: what the model "writes" (the `ok` fixture's output text).
    pub fn start(dir: &Path, fixtures: &str, delay_ms: u32, text: &str) -> Mock {
        Mock::start_with(dir, fixtures, delay_ms, text, &[])
    }

    /// `start` with more arguments for the mock, e.g. `["--asr", "429,ok", "--transcript", "hello"]`.
    pub fn start_with(
        dir: &Path,
        fixtures: &str,
        delay_ms: u32,
        text: &str,
        extra: &[&str],
    ) -> Mock {
        let log = dir.join("mock.log");
        let text_file = dir.join("mock-text.txt");
        fs::write(&text_file, text).unwrap();
        let mut child = Command::new("node")
            .arg(repo_root().join("mock/server.mjs"))
            .args(["--port", "0", "--fixture", fixtures, "--delay-ms"])
            .arg(delay_ms.to_string())
            .arg("--output-text-file")
            .arg(&text_file)
            .arg("--log")
            .arg(&log)
            .args(extra)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("node is needed to run the mock API");
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        out.read_line(&mut line).unwrap();
        let port = line
            .trim()
            .strip_prefix("listening ")
            .unwrap_or_else(|| panic!("the mock did not start: {line:?}"))
            .parse()
            .unwrap();
        Mock {
            child,
            _stdout: out,
            port,
            log,
            text: text_file,
        }
    }

    pub fn ok(dir: &Path, answer: &Value) -> Mock {
        Mock::start(dir, "ok", 0, &answer.to_string())
    }

    pub fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Each logged request; `body` is parsed (the mock writes it as a JSON string), `Null` when it is not JSON.
    pub fn requests(&self) -> Vec<Value> {
        fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| {
                let mut v: Value = serde_json::from_str(l).unwrap();
                let body = v["body"].as_str().unwrap_or("");
                v["body"] = serde_json::from_str(body).unwrap_or(Value::Null);
                v
            })
            .collect()
    }

    /// The logged voice requests (`body` is `{request, audio: {bytes, type, filename}, parts}`, never the audio).
    pub fn asr(&self) -> Vec<Value> {
        let mut v = self.requests();
        v.retain(|r| r["path"] == "/v1/asr/transcribe");
        v
    }

    /// The logged writer requests (`/v1/responses`).
    pub fn writer(&self) -> Vec<Value> {
        let mut v = self.requests();
        v.retain(|r| r["path"] == "/v1/responses");
        v
    }

    /// All the text the model was given in request `i`: the fixed rules and the data.
    pub fn prompt(&self, i: usize) -> String {
        let r = &self.requests()[i];
        format!(
            "{}\n{}",
            r["body"]["instructions"].as_str().unwrap_or(""),
            r["body"]["input"].as_str().unwrap_or("")
        )
    }

    /// The model's input text of request `i` only (the data).
    pub fn input(&self, i: usize) -> String {
        self.requests()[i]["body"]["input"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const FAKE_ENGINE: &str = r#"#!/bin/sh
# A scripted stand-in for the Torpor engine: answers from files next to it and records what `promise add` got.
D="$(dirname "$0")"
case "$1 $2" in
  "version "*)
    [ -f "$D/version.sleep" ] && sleep "$(cat "$D/version.sleep")"
    echo "torpor 0.0.0-fake" ;;
  "status --json") cat "$D/status.json" 2>/dev/null || echo '{"saving":true,"off":false,"last_turn":{"n":2}}' ;;
  "promise list") cat "$D/list.txt" 2>/dev/null ;;
  "gate "*) cat "$D/gate.txt" 2>/dev/null || echo "The gate is on." ;;
  "promise add")
    cat >> "$D/adds.log"; echo >> "$D/adds.log"
    [ -f "$D/add.sleep" ] && sleep "$(cat "$D/add.sleep")"
    cat "$D/add.out" 2>/dev/null
    cat "$D/add.err" 1>&2 2>/dev/null
    exit "$(cat "$D/add.code" 2>/dev/null || echo 0)" ;;
  *) echo "fake engine: unknown command $*" 1>&2; exit 64 ;;
esac
"#;

/// One run of `torpor-muse suggest` or `say`.
pub struct Ran {
    pub code: i32,
    pub out: String,
    pub err: String,
    pub secs: f64,
}

/// A temp folder with a synthetic workspace (the fixture app in a git repo), a state dir, a home and a fake engine.
pub struct Sandbox {
    pub dir: PathBuf,
    pub ws: PathBuf,
    /// The real engine, when a test uses it instead of the scripted one.
    pub engine: Option<PathBuf>,
    /// A state dir shared with other sandboxes (the live runs keep one ledger across all their runs).
    pub state_override: Option<PathBuf>,
}

impl Sandbox {
    pub fn new(tag: &str) -> Sandbox {
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "tm-w-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        for d in ["state", "home", "th", "eng"] {
            fs::create_dir_all(dir.join(d)).unwrap();
        }
        let ws = dir.join("ws");
        copy_dir(&repo_root().join("tests/fixtures/writer/app"), &ws);
        let fake = dir.join("eng/torpor");
        fs::write(&fake, FAKE_ENGINE).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let sb = Sandbox {
            dir,
            ws,
            engine: None,
            state_override: None,
        };
        sb.git(&["init", "-q"]);
        sb.git(&["add", "-A"]);
        sb.git(&["commit", "-qm", "init"]);
        sb
    }

    pub fn state(&self) -> PathBuf {
        self.state_override
            .clone()
            .unwrap_or_else(|| self.dir.join("state"))
    }
    pub fn home(&self) -> PathBuf {
        self.dir.join("home")
    }
    pub fn th(&self) -> PathBuf {
        self.dir.join("th")
    }
    pub fn fake(&self) -> PathBuf {
        self.dir.join("eng/torpor")
    }
    /// Scripts the fake engine's answer to `promise add`.
    pub fn engine_says(&self, code: i32, out: &str, err: &str) {
        let e = self.dir.join("eng");
        fs::write(e.join("add.code"), code.to_string()).unwrap();
        fs::write(e.join("add.out"), out).unwrap();
        fs::write(e.join("add.err"), err).unwrap();
    }
    pub fn engine_file(&self, name: &str, text: &str) {
        fs::write(self.dir.join("eng").join(name), text).unwrap();
    }

    pub fn write(&self, rel: &str, text: &str) {
        let p = self.ws.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    pub fn git(&self, args: &[&str]) {
        let o = Command::new("git")
            .args(args)
            .current_dir(&self.ws)
            .env_clear()
            .env("PATH", path_var())
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t")
            .output()
            .expect("git is needed");
        assert!(
            o.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }

    /// `torpor-muse suggest <words>` in the workspace with a clean environment (fake key, the sandbox's state,
    /// home and fake engine). `set`: `("NAME", "value")` adds a variable, `("-NAME", "")` removes one.
    pub fn suggest(&self, base: Option<&str>, words: &str, set: &[(&str, &str)]) -> Ran {
        self.cli(&["suggest", words], base, set)
    }

    /// `torpor-muse say <args>` in the workspace, the same clean environment as `suggest`.
    pub fn say(&self, base: Option<&str>, args: &[&str], set: &[(&str, &str)]) -> Ran {
        let all: Vec<&str> = ["say"].into_iter().chain(args.iter().copied()).collect();
        self.cli(&all, base, set)
    }

    /// A WAV of silence, 24 kHz mono 16-bit, `ms` long, written next to the workspace; returns its path.
    pub fn clip(&self, name: &str, ms: u32) -> PathBuf {
        let data = (ms * 48) as usize;
        let mut w = b"RIFF".to_vec();
        w.extend_from_slice(&(36 + data as u32).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 1] {
            w.extend_from_slice(&v.to_le_bytes());
        }
        w.extend_from_slice(&24_000u32.to_le_bytes());
        w.extend_from_slice(&48_000u32.to_le_bytes());
        for v in [2u16, 16] {
            w.extend_from_slice(&v.to_le_bytes());
        }
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data as u32).to_le_bytes());
        w.resize(w.len() + data, 0);
        let p = self.dir.join(name);
        fs::write(&p, w).unwrap();
        p
    }

    fn cli(&self, args: &[&str], base: Option<&str>, set: &[(&str, &str)]) -> Ran {
        let mut c = Command::new(env!("CARGO_BIN_EXE_torpor-muse"));
        c.args(args)
            .current_dir(&self.ws)
            .env_clear()
            .env("PATH", path_var())
            .env("HOME", self.home())
            .env("TORPOR_HOME", self.th())
            .env("TORPOR_MUSE_STATE", self.state())
            .env(
                "TORPOR_BIN",
                self.engine.clone().unwrap_or_else(|| self.fake()),
            )
            .env("META_API_KEY", FAKE_KEY);
        if let Some(b) = base {
            c.env("TORPOR_MUSE_API_BASE", b);
        }
        if let Ok(t) = std::env::var("TMPDIR") {
            c.env("TMPDIR", t);
        }
        for (k, v) in set {
            match k.strip_prefix('-') {
                Some(k) => {
                    c.env_remove(k);
                }
                None => {
                    c.env(k, v);
                }
            }
        }
        let t = Instant::now();
        let o = c.stdin(Stdio::null()).output().expect("run torpor-muse");
        Ran {
            code: o.status.code().unwrap_or(-1),
            out: String::from_utf8_lossy(&o.stdout).into_owned(),
            err: String::from_utf8_lossy(&o.stderr).into_owned(),
            secs: t.elapsed().as_secs_f64(),
        }
    }

    /// The newest result file under the state dir (the run that just ended; `Null` when there is none).
    pub fn result(&self) -> Value {
        let newest = fs::read_dir(self.state().join("ws"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|d| {
                let p = d.path().join("result.json");
                Some((fs::metadata(&p).ok()?.modified().ok()?, p))
            })
            .max();
        let text = newest.map_or(String::new(), |(_, p)| {
            fs::read_to_string(p).unwrap_or_default()
        });
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }

    pub fn ledger(&self) -> Vec<Value> {
        fs::read_to_string(self.state().join("ledger.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The checks the fake engine was handed by `promise add`.
    pub fn adds(&self) -> Vec<Value> {
        fs::read_to_string(self.dir.join("eng/adds.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The text of every file the run left behind (state, ledger, results, home, engine store), for leak checks.
    pub fn private_text(&self) -> String {
        fn walk(p: &Path, out: &mut String) {
            let Ok(rd) = fs::read_dir(p) else { return };
            for e in rd.flatten() {
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    walk(&e.path(), out);
                } else if let Ok(s) = fs::read_to_string(e.path()) {
                    out.push_str(&s);
                }
            }
        }
        let mut s = String::new();
        for d in ["state", "home", "th"] {
            walk(&self.dir.join(d), &mut s);
        }
        s
    }
}

impl Sandbox {
    /// Runs the real engine in the workspace with the sandbox's store; returns (exit code, stdout, stderr).
    pub fn torpor(&self, args: &[&str], stdin: Option<&str>) -> (i32, String, String) {
        use std::io::Write;
        let mut c = Command::new(self.engine.as_ref().expect("a real engine"))
            .args(args)
            .current_dir(&self.ws)
            .env_clear()
            .env("PATH", path_var())
            .env("HOME", self.home())
            .env("TORPOR_HOME", self.th())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run the engine");
        let mut pipe = c.stdin.take().unwrap();
        if let Some(t) = stdin {
            let _ = pipe.write_all(t.as_bytes());
        }
        drop(pipe);
        let o = c.wait_with_output().unwrap();
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    }

    pub fn hook(&self, event: &str, extra: Value) {
        let mut payload = json!({"session_id": "s1", "cwd": self.ws, "hook_event_name": event});
        payload
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let (code, _, err) = self.torpor(&["hook", event], Some(&payload.to_string()));
        assert_eq!(code, 0, "hook {event}: {err}");
    }

    /// The workspace without its login handler: what the app was before the login work.
    pub fn without_login(app: &str) -> String {
        let (a, b) = (app.find("  // login:start\n"), app.find("  // login:end\n"));
        let (a, b) = (a.expect("login:start marker"), b.expect("login:end marker"));
        format!("{}{}", &app[..a], &app[b + "  // login:end\n".len()..])
    }

    /// Turn 1 for the real engine: it starts on the app without the login and ends with it (what the hooks of a real
    /// session saw before the developer's praise).
    pub fn engine_first_turn(&self) {
        let file = self.ws.join("server.js");
        let after = fs::read_to_string(&file).unwrap();
        fs::write(&file, Self::without_login(&after)).unwrap();
        self.hook("SessionStart", json!({"source": "startup"}));
        self.hook(
            "UserPromptSubmit",
            json!({"prompt": "add a login to the notes app"}),
        );
        fs::write(&file, after).unwrap();
        self.hook("Stop", json!({"stop_hook_active": false}));
    }

    /// Gives the real engine a history to qualify checks against: turn 1, then the praise starts turn 2.
    pub fn engine_history(&self, praise: &str) {
        self.engine_first_turn();
        self.hook("UserPromptSubmit", json!({"prompt": praise}));
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!("kept for inspection: {}", self.dir.display());
        } else {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

pub const PROVEN: &str = "Promise #1 \"login works\" saved. Proof: passed 5 of 5 on the version from the start of turn 2 and failed 2 of 2 on the version from the start of turn 1. In shadow for now: it blocks nothing yet.";

pub const WORDS: &str = "login works now";

pub fn say_proven(sb: &Sandbox) {
    sb.engine_says(0, &format!("Checking...\n{PROVEN}\n"), "");
}

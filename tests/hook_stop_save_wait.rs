//! A Stop that comes while the check-writer is saving a new check (S3t), with the real engine.
//!
//! The scenario: promise #1 is saved (a page must answer). The user praises the app, the writer starts saving promise #2
//! with `torpor promise add`, which runs the new check on saved versions of the app one run after the other, each holding
//! the engine's run lock; the last of those runs is slow. In that turn the AI breaks the page promise #1 checks, and the
//! turn ends while the save is still going.
//!
//! The engine's Stop gate waits 3 s for the run lock and then runs no check ("unverified"): the break is not told in
//! that turn (the L4c proof had one such Stop). The first test is the product: the Stop hook waits for the save, then
//! the engine checks promise #1 and the hook blocks with the break. The second test is the control: the engine's own
//! Stop, called directly in the same scenario, gives up and checks nothing.
mod hook_common;
mod writer_common;

use hook_common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use writer_common::{FAKE_KEY, Mock, Sandbox, good_answer};

const PRAISE: &str = "login works now, alice can sign in and then see her account";
const PROOF_WORDS: &str = "the proof page must answer";
const PROOF_SPEC: &str = r#"{"v":1,"name":"proof page answers","why":"the proof page must answer","start":{"cmd":"python3 -m http.server {port} --bind 127.0.0.1","ready":"/","timeout_s":90},"env":{},"steps":[{"http":{"method":"GET","path":"/proof.html"},"expect":{"status":200,"body_contains":"proof"},"why":"the proof page answers"}],"unverifiable":false}"#;
/// How long the first start of the old version of the app takes (the save's slow run, which holds the engine's run lock
/// for this and about 0.6 s more). The engine's Stop polls that lock for 3 s, so the polling must fit in the slow run:
/// the hooks between its start and the Stop (about 0.1 s) leave a second to spare. Longer would only take the Stop hook,
/// which then still runs the engine's checks (about 3 s), closer to `CAP`.
const SLOW_MS: u32 = 3500;
/// `save_wait_cap` of the Stop hook's timeout (35 s): src/hook/events.rs.
const CAP: Duration = Duration::from_secs(12);

fn path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

fn at(t: u32, mut p: Value) -> Value {
    p["turn_id"] = json!(format!("00000000-0000-7000-8000-{t:012}"));
    p
}

struct InFlight {
    r: Rig,
    _mock: Mock,
    writer: JoinHandle<Output>,
}

impl InFlight {
    /// The writer's end: it must have saved the check it was saving.
    fn finish(self) -> Rig {
        let o = self.writer.join().unwrap();
        let (out, err) = (
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr),
        );
        assert!(
            o.status.success() && out.contains("Promise #2"),
            "{:?} {out} {err}",
            o.status
        );
        self.r
    }
}

/// Turn 2 begins with the praise and the writer starts saving a check for it; its run on the old version of the app is
/// slow. In that turn the AI breaks the page. Returns once the slow run has begun (the save is in flight) and the page
/// is broken; the Stop is for the caller.
fn saving_turn(name: &str) -> InFlight {
    let r = Rig::real(name);
    let app = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/writer/app/server.js"
    ))
    .unwrap();
    let mark = r.dir.join("slow.mark");
    let wait = format!(
        "const fs = require('fs');\nif (!fs.existsSync({m})) {{ fs.writeFileSync({m}, ''); Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, {SLOW_MS}); }}\n",
        m = serde_json::to_string(mark.to_str().unwrap()).unwrap()
    );
    // only the old version (turn 1's start) is slow: it is the last runs of the save, so the save ends soon after
    let old = Sandbox::without_login(&app).replacen(
        "'use strict';\n",
        &format!("'use strict';\n{wait}"),
        1,
    );
    assert!(
        old.contains("Atomics.wait"),
        "the slow start is in the old version"
    );
    fs::write(r.cwd.join("server.js"), old).unwrap();
    fs::write(r.cwd.join(".synthetic"), "synthetic\n").unwrap();
    started(&r);
    fs::write(r.cwd.join("server.js"), &app).unwrap();
    r.hook_ok("Stop", &r.fixture("Stop"));
    // promise #1: the user's own check of a page
    fs::write(r.cwd.join("proof.html"), "<p>proof</p>\n").unwrap();
    let (code, text) = engine_cli_stdin(&r, &["keep", PROOF_WORDS, "--check", "-"], PROOF_SPEC);
    assert_eq!(code, 0, "{text}");

    let mut answer = good_answer();
    answer["start_script"] = json!("node server.js"); // the fixture app has no package.json
    let mock = Mock::ok(&r.dir, &answer);
    r.hook_ok("UserPromptSubmit", &at(3, r.fixture("UserPromptSubmit")));
    let mut c = Command::new(EXE);
    c.args(["suggest", "-"])
        .current_dir(&r.cwd)
        .env_clear()
        .env("PATH", path())
        .env("HOME", r.dir.join("home"))
        .env("TORPOR_HOME", r.store())
        .env("TORPOR_BIN", &r.engine)
        .env("META_API_KEY", FAKE_KEY)
        .env("TORPOR_MUSE_API_BASE", mock.base())
        .env("TORPOR_MUSE_FROM_HOOK", SID)
        .env("TORPOR_MUSE_WORDS", PRAISE)
        .stdin(Stdio::null());
    let writer = std::thread::spawn(move || c.output().expect("run the check-writer"));
    let begun = Instant::now();
    while !mark.exists() {
        assert!(
            begun.elapsed() < Duration::from_secs(90),
            "the slow run never began"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let (mut pre, mut post) = (
        at(3, r.fixture("PreToolUse-bash")),
        at(3, r.fixture("PostToolUse-bash")),
    );
    for p in [&mut pre, &mut post] {
        p["tool_input"]["command"] = json!("echo nope > proof.html");
        p["tool_use_id"] = json!("call_3");
    }
    r.hook_ok("PreToolUse", &pre);
    fs::write(r.cwd.join("proof.html"), "<p>nope</p>\n").unwrap();
    r.hook_ok("PostToolUse", &post);
    InFlight {
        r,
        _mock: mock,
        writer,
    }
}

/// The outcome words on the engine's receipts, one row per Stop, oldest first ("unverified": the Stop gave up on its
/// checks; "fail": one failed). No engine command shows a single receipt, so the table is read.
fn receipts(r: &Rig) -> Vec<String> {
    let db: PathBuf = fs::read_dir(r.store())
        .unwrap()
        .map(|e| e.unwrap().path().join("torpor.db"))
        .find(|p| p.exists())
        .expect("the engine's database");
    let code = "import sqlite3,sys\nfor (o,) in sqlite3.connect(sys.argv[1]).execute('select outcome from receipts order by id'): print(o)";
    let o = Command::new("python3")
        .args(["-I", "-c", code])
        .arg(db)
        .output()
        .expect("python3 is needed");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// F-S3t: the Stop hook waits for the save, then the engine checks promise #1 and the hook tells the break in its block.
#[test]
fn a_stop_during_a_save_waits_for_it_and_then_tells_the_break() {
    let s = saving_turn("save-wait-product");
    let stop = at(3, s.r.fixture("Stop"));
    let run =
        s.r.hook_env("Stop", stop.to_string().as_bytes(), &[("PATH", &path())]);
    eprintln!("S3t: the Stop hook took {:?}", run.took);
    assert_eq!(run.code, 0, "{}", run.err);
    let out: Value = serde_json::from_str(&run.out).unwrap_or(Value::Null);
    let why =
        blocked(&out).unwrap_or_else(|| panic!("no block (took {:?}): {}", run.took, run.out));
    assert!(
        why.contains("Promise #1 \"proof page answers\" (")
            && why.contains(") failed after this turn."),
        "{why}"
    );
    // it waited for the save (the engine alone gives up after 3 s) and was done before the cap
    assert!(
        run.took > Duration::from_secs(3) && run.took < CAP,
        "{:?}",
        run.took
    );
    let r = s.finish();
    let last = receipts(&r).pop().unwrap_or_default();
    assert!(
        last.split(',').any(|o| o == "fail") && !last.contains("unverified"),
        "{last}"
    );
}

/// The control: the same scenario, the engine's Stop called directly (what the hook did before): it gives up on the run
/// lock after 3 s, checks nothing and says nothing of the break, which is real.
#[test]
fn the_engines_own_stop_gives_up_on_the_same_save_and_checks_nothing() {
    let s = saving_turn("save-wait-direct");
    let stop = at(3, s.r.fixture("Stop"));
    let t = Instant::now();
    let (code, text) = engine_cli_stdin(&s.r, &["hook", "Stop"], &stop.to_string());
    let took = t.elapsed();
    eprintln!("S3t: the engine's own Stop took {took:?}");
    assert_eq!(code, 0, "{text}");
    assert!(took >= Duration::from_millis(2500), "{took:?}");
    let out: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    assert!(
        blocked(&out).is_none() && !text.contains("failed after this turn"),
        "{text}"
    );
    let r = s.finish();
    let last = receipts(&r).pop().unwrap_or_default();
    assert!(
        last.contains("unverified") && !last.contains("fail"),
        "{last}"
    );
    let (_, checked) = engine_cli(&r, &["check", "1"]);
    assert!(
        checked.contains("FAIL"),
        "the page was broken, and the check would have said so: {checked}"
    );
}

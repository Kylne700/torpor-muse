//! A Stop that comes while the check-writer is saving a new check (S3t), with the real engine.
//!
//! The scenario: promise #1 is saved (a page must answer). The user praises the app, the writer starts saving promise #2
//! with `torpor promise add`, which proves the new check on saved versions of the app one run after the other, each run
//! holding the engine's run lock. The last of those runs is slow: it starts the old version of the app, which waits until
//! the test lets it go. In that turn the AI edits a file and the turn ends while the save is still going.
//!
//! The engine's Stop gate waits 3 s for the run lock and then runs no check ("unverified"): a break made in that turn
//! would not be told (the L4c proof had one such Stop). The first test is the product: the Stop hook waits for the save,
//! then the engine checks promise #1. The second test is the control: the engine's own Stop, called directly in the same
//! scenario, gives up and checks nothing.
//!
//! Speed: in both tests the test, not a timer in the app, lets the slow run go on `HOLD` after the Stop begins; the rest
//! of the save then takes longer on a slower runner, which only lengthens the time the lock is held (the control still
//! gives up, the product waits longer and has less room below the cap). The cap bounds the wait, not the checks the engine
//! runs after it. Those are two here (promise #1, and promise #2 which its own bad runs left failing): about 0.6 s each on
//! a Linux machine, about four times that on the macOS CI runner. So the wait is measured up to the moment the engine is
//! called (a stand-in for it notes the time, then runs the real one), and the whole hook only has to end inside Muse's
//! timeout. The edit keeps the page answering: a break would make the engine run five checks at this Stop (three of the
//! failed one and a control run, and promise #2), more than the engine's 20 s for them on a slow runner.
mod hook_common;
mod writer_common;

use hook_common::*;
use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use writer_common::{FAKE_KEY, Mock, Sandbox, good_answer};

const PRAISE: &str = "login works now, alice can sign in and then see her account";
const PROOF_WORDS: &str = "the proof page must answer";
const PROOF_SPEC: &str = r#"{"v":1,"name":"proof page answers","why":"the proof page must answer","start":{"cmd":"python3 -m http.server {port} --bind 127.0.0.1","ready":"/","timeout_s":90},"env":{},"steps":[{"http":{"method":"GET","path":"/proof.html"},"expect":{"status":200,"body_contains":"proof"},"why":"the proof page answers"}],"unverifiable":false}"#;
/// How long after the Stop begins the test lets the slow run go on: more than the engine's own 3 s poll of the run lock,
/// so the hook is shown to wait longer than the engine alone, and below `CAP` once the rest of the save is added (about
/// 1 s on a Linux machine, about four times that on the macOS CI runner).
const HOLD: Duration = Duration::from_millis(3500);
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
    /// Writing this file lets the slow run go on.
    release: PathBuf,
}

impl InFlight {
    /// Lets the slow run go on `HOLD` from now, from a thread: the caller runs the Stop meanwhile.
    fn release_after_hold(&self) -> JoinHandle<()> {
        let release = self.release.clone();
        std::thread::spawn(move || {
            std::thread::sleep(HOLD);
            fs::write(release, "").unwrap();
        })
    }

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

/// Turn 2 begins with the praise and the writer starts saving a check for it; its last run, on the old version of the
/// app, waits for `release`. In that turn the AI edits the page. Returns once the slow run has begun (the save is in
/// flight) and the page is edited; the Stop, and the release of the slow run, are for the caller.
fn saving_turn(name: &str) -> InFlight {
    let r = Rig::real(name);
    let app = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/writer/app/server.js"
    ))
    .unwrap();
    let (first, mark, release) = (
        r.dir.join("first.mark"),
        r.dir.join("slow.mark"),
        r.dir.join("release.mark"),
    );
    let quoted = |p: &PathBuf| serde_json::to_string(p.to_str().unwrap()).unwrap();
    // the save proves the check on the old version twice (turn 1's start): the first start is quick, the second waits
    // for `release` (60 s at most, so a test that fails leaves nothing waiting for long)
    let wait = format!(
        "const fs = require('fs');\nif (!fs.existsSync({f})) fs.writeFileSync({f}, '');\nelse if (!fs.existsSync({m})) {{ fs.writeFileSync({m}, ''); for (let i = 0; i < 1200 && !fs.existsSync({x}); i++) Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 50); }}\n",
        f = quoted(&first),
        m = quoted(&mark),
        x = quoted(&release)
    );
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
            begun.elapsed() < Duration::from_secs(180),
            "the slow run never began"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let (mut pre, mut post) = (
        at(3, r.fixture("PreToolUse-bash")),
        at(3, r.fixture("PostToolUse-bash")),
    );
    for p in [&mut pre, &mut post] {
        p["tool_input"]["command"] = json!("echo more > proof.html");
        p["tool_use_id"] = json!("call_3");
    }
    r.hook_ok("PreToolUse", &pre);
    fs::write(r.cwd.join("proof.html"), "<p>proof, and more</p>\n").unwrap();
    r.hook_ok("PostToolUse", &post);
    InFlight {
        r,
        _mock: mock,
        writer,
        release,
    }
}

/// Rows of one SQL query on the engine's database, the columns joined with "|". No engine command shows the receipts or
/// the runs of a check, so the tables are read.
fn rows(r: &Rig, sql: &str) -> Vec<String> {
    let db: PathBuf = fs::read_dir(r.store())
        .unwrap()
        .map(|e| e.unwrap().path().join("torpor.db"))
        .find(|p| p.exists())
        .expect("the engine's database");
    let code = "import sqlite3,sys\nfor row in sqlite3.connect(sys.argv[1]).execute(sys.argv[2]): print('|'.join(map(str, row)))";
    let o = Command::new("python3")
        .args(["-I", "-c", code])
        .arg(db)
        .arg(sql)
        .output()
        .expect("python3 is needed");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The outcome words of the engine's last receipt, the last Stop ("unverified": it gave up on its checks).
fn last_receipt(r: &Rig) -> String {
    rows(r, "select outcome from receipts order by id desc limit 1")
        .pop()
        .unwrap_or_default()
}

/// What the engine recorded, for the message of a failed assert.
fn story(r: &Rig) -> String {
    format!(
        "receipts (selected|ran|outcome) {:?}, runs (promise|kind|verdict|ms) {:?}",
        rows(r, "select selected,ran,outcome from receipts order by id"),
        rows(
            r,
            "select promise_id,kind,verdict,ms from promise_runs order by id"
        )
    )
}

/// An engine stand-in for the Stop hook's environment: it notes when the engine is called for the Stop (nanoseconds since
/// the epoch, python3 because BSD `date` has no `%N`), then runs the real engine. Returns the script and the note's file.
fn noting_the_engine_call(r: &Rig) -> (PathBuf, PathBuf) {
    let (shim, note) = (r.dir.join("engine-shim.sh"), r.dir.join("engine-call.ns"));
    let text = format!(
        "#!/bin/sh\n[ \"$2\" = Stop ] && python3 -I -c 'import time; print(time.time_ns())' > '{}'\nexec '{}' \"$@\"\n",
        note.display(),
        r.engine.display()
    );
    fs::write(&shim, text).unwrap();
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    (shim, note)
}

/// F-S3t: the Stop hook waits for the save, then the engine checks promise #1 (and does not give up on the run lock).
#[test]
fn a_stop_during_a_save_waits_for_it_and_then_runs_the_check() {
    let s = saving_turn("save-wait-product");
    let stop = at(3, s.r.fixture("Stop"));
    let (shim, note) = noting_the_engine_call(&s.r);
    let began = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let timer = s.release_after_hold();
    let run = s.r.hook_env(
        "Stop",
        stop.to_string().as_bytes(),
        &[("PATH", &path()), ("TORPOR_BIN", shim.to_str().unwrap())],
    );
    timer.join().unwrap();
    assert_eq!(run.code, 0, "{}", run.err);
    let called = fs::read_to_string(&note).expect("the engine was called for the Stop");
    let waited = Duration::from_nanos(called.trim().parse().unwrap()).saturating_sub(began);
    let r = s.finish();
    let story = story(&r);
    eprintln!(
        "S3t: the Stop hook took {:?}, called the engine after {waited:?}; {story}",
        run.took
    );
    // the engine checked promise #1 at that Stop (it gave up on the run lock before)
    let last = last_receipt(&r);
    assert!(
        !last.contains("unverified") && last.split(',').any(|o| o == "ok"),
        "the Stop gave up on its checks: {story}"
    );
    assert_eq!(
        rows(
            &r,
            "select count(*) from promise_runs where promise_id=1 and kind='gate'"
        ),
        ["1"],
        "promise #1 was checked once at the Stop: {story}"
    );
    // it waited for the save (the engine alone gives up after 3 s), until the save was done and not for the whole cap
    assert!(
        waited > Duration::from_secs(3) && waited < CAP,
        "the Stop hook called the engine after {waited:?}: {story}"
    );
    // and with the engine's checks it still ended inside Muse's timeout for the hook, or its answer would be lost
    assert!(
        run.took < Duration::from_secs(35),
        "the Stop hook took {:?}: {story}",
        run.took
    );
}

/// The control: the same scenario, the engine's Stop called directly (what the hook did before), the slow run let go on
/// the same `HOLD` after the Stop begins: it gives up on the run lock after 3 s, checks nothing and says nothing of the edit.
#[test]
fn the_engines_own_stop_gives_up_on_the_same_save_and_checks_nothing() {
    let s = saving_turn("save-wait-direct");
    let stop = at(3, s.r.fixture("Stop"));
    let t = Instant::now();
    let timer = s.release_after_hold();
    let (code, text) = engine_cli_stdin(&s.r, &["hook", "Stop"], &stop.to_string());
    let took = t.elapsed();
    timer.join().unwrap();
    eprintln!("S3t: the engine's own Stop took {took:?}");
    assert_eq!(code, 0, "{text}");
    assert!(took >= Duration::from_millis(2500), "{took:?}");
    let r = s.finish();
    let story = story(&r);
    assert!(
        last_receipt(&r).contains("unverified"),
        "the engine's own Stop gave up on the run lock: {story}"
    );
    assert_eq!(
        rows(
            &r,
            "select count(*) from promise_runs where kind in ('gate','control')"
        ),
        ["0"],
        "it ran no check: {story}"
    );
}

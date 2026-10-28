//! No false alarms from the hands-free parts: harmless commands pass, a Stop block comes only for real loss.

mod hook_common;

use hook_common::*;
use serde_json::{Value, json};
use std::fs;

// ---------------------------------------------------------------- F4c: no false alarms from the hands-free parts

/// The F4c corpus (pre-registered: no case is added, dropped or edited). The folder is a git repo with a busy `dev.db`
/// (its app runs, as in D07) and the files the commands name; the hook only looks at the commands, none is run. Rows:
/// id, command, the folder's .gitignore. The first list must go through, the second must be denied.
const IGNORED: &str = "dev.db";
const QUIET_ALLOW: [(&str, &str, &str); 14] = [
    ("A1", "find . -name .DS_Store -delete", IGNORED),
    ("A2", "find . -name '*.pyc' -delete", IGNORED),
    (
        "A3",
        "find . -type d -name __pycache__ -exec rm -rf {} +",
        IGNORED,
    ),
    ("A4", "git clean -fd", "dev.db*"),
    ("A5", "git clean -fdx -e 'dev.db*'", IGNORED),
    ("A6", "rm app.{log,txt}", IGNORED),
    ("A7", "cp -t backup/ dev.db", IGNORED),
    ("A8", "cat > reset.sh <<'EOF'\nrm dev.db\nEOF", IGNORED),
    ("A9", "mv src/old.ts src/new.ts", IGNORED),
    ("A10", "rm -rf node_modules", IGNORED),
    ("A11", "find src -name '*.orig' -delete", IGNORED),
    ("A12", "git rm --cached dev.db", IGNORED),
    ("A13", "cp dev.db dev.db.bak", IGNORED),
    ("A14", "rm -f dev.db.bak", IGNORED),
];
const QUIET_DENY: [(&str, &str, &str); 12] = [
    ("D1", "rm dev.db", IGNORED),
    ("D2", "rm -f ./dev.db*", IGNORED),
    ("D3", "find . -name '*.db' -delete", IGNORED),
    ("D4", "find . -delete", IGNORED),
    ("D5", "git clean -fdx", IGNORED),
    ("D6", "mv dev.db old.db", IGNORED),
    ("D7", "cp other.db dev.db", IGNORED),
    ("D8", ": > dev.db", IGNORED),
    ("D9", "sh -c 'rm dev.db'", IGNORED),
    ("D10", "cd data && rm ../dev.db", IGNORED),
    ("D11", "git clean -fd", IGNORED),
    ("D12", "git clean -fdx -e dev.db", IGNORED),
];

/// The corpus folder; its app holds `dev.db` open as long as the returned value lives.
fn quiet_folder(r: &Rig) -> BusyApp {
    assert!(
        r.command("git", &["init", "-q"])
            .status()
            .unwrap()
            .success()
    );
    for d in ["data", "src", "backup", "node_modules/pkg"] {
        fs::create_dir_all(r.cwd.join(d)).unwrap();
    }
    for f in [
        "src/old.ts",
        "other.db",
        "dev.db.bak",
        "app.log",
        "app.txt",
        "node_modules/pkg/index.js",
    ] {
        fs::write(r.cwd.join(f), "x\n").unwrap();
    }
    let app = BusyApp::start(&r.cwd.join("dev.db"));
    assert!(fs::metadata(r.cwd.join("dev.db-wal")).unwrap().len() > 0);
    app
}

/// The rows of `list` whose verdict is not the wanted one (`want_deny`), each as `id command`.
fn quiet_wrong(name: &str, list: &[(&str, &str, &str)], want_deny: bool) -> Vec<String> {
    let r = Rig::real(name);
    let _app = quiet_folder(&r);
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    let mut wrong = Vec::new();
    for (id, cmd, ignored) in list {
        fs::write(r.cwd.join(".gitignore"), format!("{ignored}\n")).unwrap();
        // A6 only means something when a busy `app.db` is there for `app.*` to hit (the expanded braces must not)
        let app = (*id == "A6").then(|| BusyApp::start(&r.cwd.join("app.db")));
        let mut p = r.fixture("PreToolUse-bash");
        p["tool_input"]["command"] = json!(cmd);
        p["tool_use_id"] = json!(format!("call_{id}"));
        let out = r.hook_ok("PreToolUse", &p).out;
        let denied = serde_json::from_str::<Value>(&out)
            .is_ok_and(|v| v["hookSpecificOutput"]["permissionDecision"] == "deny");
        if denied != want_deny {
            wrong.push(format!("{id} {cmd}"));
        }
        if app.is_some() {
            drop(app);
            for s in ["", "-wal", "-shm"] {
                let _ = fs::remove_file(r.cwd.join(format!("app.db{s}")));
            }
        }
    }
    wrong
}

/// F4c: commands that cannot hurt the busy database are not stopped (a false alarm teaches the model to ask for approval).
#[test]
fn corpus_commands_that_cannot_hurt_a_busy_database_are_not_denied() {
    let wrong = quiet_wrong("quiet-allow", &QUIET_ALLOW, false);
    assert!(wrong.is_empty(), "{} false denies: {wrong:#?}", wrong.len());
}

/// F4c: and every way of losing it is still denied, `git clean` included (it deletes the -wal and -shm files too).
#[test]
fn corpus_commands_that_would_hurt_a_busy_database_are_denied() {
    let wrong = quiet_wrong("quiet-deny", &QUIET_DENY, true);
    assert!(
        wrong.is_empty(),
        "{} missed denies: {wrong:#?}",
        wrong.len()
    );
}

/// F4c Stop cases. S1 S3 S4 S5 with the fake engine saying what the turn did; S2 (a rename) with the real engine.
#[test]
fn corpus_the_stop_block_comes_only_for_what_is_deleted_or_lost() {
    let mut wrong: Vec<String> = Vec::new();
    let r = Rig::new("quiet-stop");
    started(&r);
    // S1: a migration changed dev.db: the user's line stays, the model is not stopped for it
    fs::write(r.cwd.join("dev.db"), b"x").unwrap();
    engine_turn(&r, 5, 30, &[31], &[("dev.db", "shell", "changed")]);
    engine_stop(&r, &[SAVED]);
    let v = stop_of(&r, 2);
    if blocked(&v).is_some() || !said(&v).contains("turn 5 changed dev.db") {
        wrong.push(format!("S1 {v}"));
    }
    // S3: a deleted file is still told
    engine_turn(&r, 6, 40, &[], &[("notes.md", "shell", "deleted")]);
    let v = stop_of(&r, 3);
    if !blocked(&v).is_some_and(|w| w.contains("This turn deleted notes.md")) {
        wrong.push(format!("S3 {v}"));
    }
    // S5: Muse then runs the turn on and stops again (stop_hook_active): that Stop asks the engine nothing
    let calls = r.calls().len();
    let mut again = r.fixture("Stop-active");
    again["turn_id"] = json!("00000000-0000-7000-8000-000000000003");
    let v = serde_json::from_str::<Value>(&r.hook_ok("Stop", &again).out).unwrap_or(Value::Null);
    if r.calls().len() != calls || blocked(&v).is_some() {
        wrong.push(format!("S5 {} engine calls: {v}", r.calls().len() - calls));
    }
    // S4 (D07): the database is deleted after its app stopped, or while it ran: both stay blocked
    engine_turn(&r, 7, 50, &[51], &[("dev.db", "shell", "deleted")]);
    let v = stop_of(&r, 4);
    if !blocked(&v).is_some_and(|w| w.contains("deleted dev.db (database files)")) {
        wrong.push(format!("S4 deleted {v}"));
    }
    fs::remove_file(r.cwd.join("dev.db")).unwrap();
    engine_turn(&r, 8, 60, &[61], &[("dev.db", "shell", "not saved")]);
    let v = stop_of(&r, 5);
    if !blocked(&v).is_some_and(|w| w.contains("could not save dev.db")) {
        wrong.push(format!("S4 in use {v}"));
    }
    // S2: the real engine lists a rename as a deleted file and a new one
    let r = Rig::real("quiet-rename");
    fs::create_dir(r.cwd.join("src")).unwrap();
    fs::write(r.cwd.join("src/old.ts"), "export const a = 1;\n".repeat(12)).unwrap();
    started(&r);
    r.hook_ok("Stop", &r.fixture("Stop"));
    let v = real_turn(&r, 3, "mv src/old.ts src/new.ts", || {
        fs::rename(r.cwd.join("src/old.ts"), r.cwd.join("src/new.ts")).unwrap()
    });
    if blocked(&v).is_some_and(|w| w.contains("deleted")) {
        let (_, st) = engine_cli(&r, &["status", "--json"]);
        let n = serde_json::from_str::<Value>(&st)
            .map_or(0, |s| s["last_turn"]["n"].as_u64().unwrap_or(0));
        let (_, rows) = engine_cli(&r, &["show", &n.to_string(), "--json"]);
        let rows = serde_json::from_str::<Value>(&rows).map_or(rows, |s| s["files"].to_string());
        wrong.push(format!("S2 {v} rows {rows}"));
    }
    assert!(
        wrong.is_empty(),
        "{} wrong Stop cases: {wrong:#?}",
        wrong.len()
    );
}

/// AD3 (research/60): a promise the AI adds through its bash tool must not count as the user's words. The engine knows
/// who ran `torpor keep` only from the row PreToolUse(Bash) leaves (or CLAUDECODE, which Muse does not set), so the same
/// promise made without that row is the control: it has to reach the stronger tier, or the test proves nothing.
/// (The engine reads the turn's origin from the transcript; with no transcript the origin is unknown. Once the shim
/// supplies one that says "human", the engine counts an AI-run keep in that turn as the user's words: see the report.)
#[test]
fn real_engine_an_ai_run_keep_stays_advisory_and_the_same_keep_from_the_user_gets_shadow() {
    const WORDS: &str = "the proof page must answer";
    const SPEC: &str = r#"{"v":1,"name":"proof page answers","why":"the proof page must answer","start":{"cmd":"python3 -m http.server {port} --bind 127.0.0.1","ready":"/","timeout_s":90},"env":{},"steps":[{"http":{"method":"GET","path":"/proof.html"},"expect":{"status":200},"why":"the proof page answers"}],"unverifiable":false}"#;
    // turn 1 leaves a save point without the page; the page is then made by hand, outside any turn, so the check is provable
    let rig = |name: &str| {
        let r = Rig::real(name);
        r.hook_ok("SessionStart", &r.fixture("SessionStart"));
        r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
        r.hook_ok("Stop", &r.fixture("Stop"));
        fs::write(r.cwd.join("proof.html"), "<p>proof</p>\n").unwrap();
        r
    };
    let keep = |r: &Rig| {
        let (code, text) = engine_cli_stdin(r, &["keep", WORDS, "--check", "-"], SPEC);
        assert_eq!(code, 0, "{text}");
        assert!(
            text.contains("failed 2 of 2 on the start of turn 1"),
            "the check must be provable: {text}"
        );
    };

    let user = rig("real-ad3-user");
    keep(&user);
    let (_, list) = engine_cli(&user, &["promise", "list"]);
    assert!(
        list.contains("· shadow ·") && list.contains("your words"),
        "{list}"
    );

    let ai = rig("real-ad3-ai");
    let in_turn_2 = |mut p: Value| {
        p["turn_id"] = json!("00000000-0000-7000-8000-000000000012");
        p
    };
    ai.hook_ok(
        "UserPromptSubmit",
        &in_turn_2(ai.fixture("UserPromptSubmit")),
    );
    let mut pre = in_turn_2(ai.fixture("PreToolUse-bash"));
    pre["tool_input"]["command"] = json!(format!("torpor keep \"{WORDS}\" --check - < spec.json"));
    ai.hook_ok("PreToolUse", &pre);
    keep(&ai);
    let stop = ai.hook_ok("Stop", &in_turn_2(ai.fixture("Stop")));
    assert!(stop.out.contains("proposed by the AI"), "{}", stop.out);
    let (_, list) = engine_cli(&ai, &["promise", "list"]);
    assert!(
        list.contains("· advisory ·") && list.contains("the AI"),
        "{list}"
    );
}

#[test]
fn real_engine_every_reply_in_a_session_is_valid_muse_output() {
    let r = Rig::real("real-session");
    turn_with_a_created_and_deleted_file(&r, true); // asserts every reply on the way
    for (event, fixture) in [
        ("PostToolUseFailure", "PostToolUseFailure-bash"),
        ("PostToolUseFailure", "PostToolUseFailure-bash-sandbox"),
        ("PreCompact", "PreCompact"),
        ("SessionStart", "SessionStart-compact"),
        ("StopFailure", "StopFailure"),
        ("SessionEnd", "SessionEnd"),
    ] {
        let run = r.hook_ok(event, &r.fixture(fixture));
        assert!(
            run.err.is_empty() || !run.err.contains("panicked"),
            "{event}: {}",
            run.err
        );
    }
    // a command that deletes data outside this computer: Muse cannot ask, so it is denied with a reason
    let mut p = r.fixture("PreToolUse-bash");
    p["tool_input"]["command"] = json!("git push --force origin main");
    p["permission_mode"] = json!("default");
    let run = r.hook_ok("PreToolUse", &p);
    assert!(
        run.out.contains(r#""permissionDecision":"deny""#),
        "never an allow, never an ask: {}",
        run.out
    );
    assert!(!run.out.contains("Approve only"), "{}", run.out);
}

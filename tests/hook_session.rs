//! The hook with the real engine over a session: what the model is told, the user's own commands, the gate default.

mod hook_common;

use hook_common::*;
use serde_json::{Value, json};
use std::fs;

// ---------------------------------------------------------------- the real engine

#[test]
fn real_engine_a_file_made_and_deleted_in_one_turn_comes_back_only_with_the_pre_image() {
    // with the hook seeing the delete before it runs, `torpor back 1` brings the file back
    let r = Rig::real("real-restore");
    turn_with_a_created_and_deleted_file(&r, true);
    assert!(!r.cwd.join("created.txt").exists());
    let (code, text) = engine_cli(&r, &["back", "1"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(
        fs::read_to_string(r.cwd.join("created.txt")).unwrap(),
        "new file\n",
        "{text}"
    );
    assert!(
        text.contains("before rm created.txt"),
        "the save point is named after the command: {text}"
    );
    // negative control: the same turn without the PreToolUse of the delete has nothing to bring back
    let c = Rig::real("real-restore-control");
    turn_with_a_created_and_deleted_file(&c, false);
    let (_, text) = engine_cli(&c, &["back", "1"]);
    assert!(
        !c.cwd.join("created.txt").exists(),
        "no pre-image, no file: {text}"
    );
}

/// L3 N1 and N2: a turn that changed a database is kept for its folder (the last 5, newest first), also past
/// SessionEnd. The user gets a short line with the command first; the model gets every kept turn, each with its own
/// command, once per session: at the next prompt, or at the start of a session (startup, resume, compact) in that folder.
#[test]
fn database_turns_reach_the_model_after_an_exit_and_in_later_sessions() {
    let r = Rig::new("db-kept");
    let db_turn = |n: u32, p: u32, files: &[&str]| {
        r.set(
            "out.status",
            &json!({"saving": true, "last_turn": {"n": n}}).to_string(),
        );
        let files: Vec<Value> = files
            .iter()
            .map(|f| json!({"path": f, "class": "tracked", "change": "deleted"}))
            .collect();
        let show = json!({"turn": {"n": n, "start_point": p}, "files": files});
        r.set(&format!("out.show.{n}"), &show.to_string());
        note_parts(&stop_run(&r, n).out).0
    };
    let ups = || context_of(&r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit")));
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    assert_eq!(ups(), "");
    let said = db_turn(3, 9, &["notes.db"]);
    assert_eq!(
        said,
        "Torpor: turn 3 changed notes.db. Get it back: stop the app, then torpor back --id 9 --with-data"
    );
    // exit, then resume (the same session id): the start context has it, and the next prompt does not repeat it
    r.hook_ok("SessionEnd", &r.fixture("SessionEnd"));
    let ctx = context_of(&r.hook_ok("SessionStart", &r.fixture("SessionStart-resume")));
    for part in [
        "Turn 3 changed notes.db",
        "stop the app",
        "every other file",
        "even if the app made a fresh database since",
        "only when the user asks",
    ] {
        assert!(ctx.contains(part), "{part}: {ctx}");
    }
    assert!(
        ctx.ends_with(&format!(
            "The command: {} back --id 9 --with-data",
            r.engine.display()
        )),
        "{ctx}"
    );
    assert_eq!(ups(), "");
    // a second database turn: both are listed, the newer first, each with its own command
    let said = db_turn(5, 12, &["data/app.sqlite"]);
    assert_eq!(
        said,
        "Torpor: turn 5 changed data/app.s... Get it back: stop the app, then torpor back --id 12 --with-data",
        "100 characters: the name is cut, the command stays whole"
    );
    let ctx = ups();
    let (five, three) = (ctx.find("Turn 5 changed"), ctx.find("Turn 3 changed"));
    assert!(five.is_some() && three.is_some() && five < three, "{ctx}");
    assert!(
        ctx.contains("back --id 12 --with-data") && ctx.contains("back --id 9 --with-data"),
        "{ctx}"
    );
    assert_eq!(ups(), "", "once per session");
    // an exit with no new start: the next prompt of the session gets them again
    r.hook_ok("SessionEnd", &r.fixture("SessionEnd"));
    assert!(ups().contains("Turn 5 changed"));
    // compaction, and a new session in the same folder
    assert!(
        context_of(&r.hook_ok("SessionStart", &r.fixture("SessionStart-compact")))
            .contains("Turn 3 changed")
    );
    let mut other = r.fixture("SessionStart");
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000bb");
    let ctx = context_of(&r.hook_ok("SessionStart", &other));
    assert!(
        ctx.contains("Turn 5 changed") && ctx.contains("Turn 3 changed"),
        "{ctx}"
    );
    // long names are cut, never the command; only the last 5 turns stay
    let said = db_turn(
        7,
        20,
        &["data/a-database-file-with-a-very-long-name.sqlite3", "b.db"],
    );
    assert!(
        said.chars().count() <= 100
            && said.starts_with("Torpor: turn 7 changed data/a")
            && said.contains("...")
            && said.ends_with(". Get it back: stop the app, then torpor back --id 20 --with-data"),
        "{said}"
    );
    for n in [8, 10, 11] {
        db_turn(n, n + 20, &["notes.db"]);
    }
    let mut fresh = r.fixture("SessionStart");
    fresh["session_id"] = json!("00000000-0000-7000-8000-0000000000cc");
    let ctx = context_of(&r.hook_ok("SessionStart", &fresh));
    assert!(
        ctx.contains("Turn 11 changed")
            && ctx.contains("Turn 5 changed")
            && !ctx.contains("Turn 3 changed"),
        "{ctx}"
    );
}

/// A Stop after a turn that deleted `files` (database files) and began at save point `p`, with the fake engine's
/// `status --json` and `show N --json` answers set to match: the turn is then kept for its folder (L3 N1).
fn db_stop(r: &Rig, n: u32, p: u32, files: &[String]) -> String {
    r.set(
        "out.status",
        &json!({"saving": true, "last_turn": {"n": n}}).to_string(),
    );
    let rows: Vec<Value> = files
        .iter()
        .map(|f| json!({"path": f, "class": "tracked", "change": "deleted"}))
        .collect();
    let show = json!({"turn": {"n": n, "start_point": p}, "files": rows});
    r.set(&format!("out.show.{n}"), &show.to_string());
    note_parts(&stop_run(r, 100 + n).out).0 // not 1: the session's own id is a subagent's turn
}

const FIVE_SOURCES: [&str; 5] = [
    "SessionStart",
    "SessionStart-clear",
    "SessionStart-compact",
    "SessionStart-fork",
    "SessionStart-resume",
];

/// F4 item 1: every session start (startup, resume, clear, compact, fork) gives the model one short guide: the plain
/// words a user says, the Torpor command for each (a dry run first), and the commands that are the user's own.
#[test]
fn the_model_gets_the_guide_at_every_session_start_and_nowhere_else() {
    let r = Rig::new("guide");
    let engine = r.engine.display().to_string();
    for name in FIVE_SOURCES {
        let run = r.hook_ok("SessionStart", &r.fixture(name));
        let (said, ctx) = note_parts(&run.out);
        assert_eq!(said, "", "{name}: nothing for the user");
        assert!(
            ctx.starts_with("[Torpor guide: stays valid] "),
            "{name}: {ctx}"
        );
        assert_eq!(
            ctx.matches(&engine).count(),
            1,
            "the engine's path, once: {ctx}"
        );
        assert!(
            ctx.chars().count() - engine.len() <= 670,
            "about 730 characters with a path of 60: {}",
            ctx.chars().count()
        );
        for want in [
            "torpor log -n 5, torpor undo N --dry-run, torpor undo N.",
            "torpor back --turn K or --id P",
            "torpor find-break --promise ID (or --check -)",
            "the database note's command, app stopped first",
            "Never run off, on, gate, retain, forget, undo --force, promise ok/drop/block/show --spec, runner install",
        ] {
            assert!(ctx.contains(want), "{name}: {want:?} missing: {ctx}");
        }
    }
    // the engine's own startup line stays for the user, and its own context stays in front of the guide
    r.set(
        "out.SessionStart",
        r#"{"systemMessage":"Torpor is saving this folder at the start and end of every turn.","hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"engine words"}}"#,
    );
    let (said, ctx) = note_parts(&r.hook_ok("SessionStart", &r.fixture("SessionStart")).out);
    assert_eq!(
        said,
        "Torpor is saving this folder at the start and end of every turn."
    );
    assert!(
        ctx.starts_with("[Torpor note")
            && ctx.contains("engine words\n\n[Torpor guide: stays valid]"),
        "{ctx}"
    );
    // not at a prompt, not at a stop, not for a subagent's own session
    for event in ["UserPromptSubmit", "Stop", "PreCompact"] {
        let run = r.hook_ok(event, &r.fixture(event));
        assert!(!run.out.contains("Torpor guide"), "{event}: {}", run.out);
    }
    let mut child = r.fixture("SessionStart");
    child["turn_id"] = child["session_id"].clone();
    assert!(r.hook_ok("SessionStart", &child).out.is_empty());
}

/// F4 item 1, the worst case: the engine's context is as long as Muse takes, a systemMessage is near its limit, five
/// database turns have the longest file lists the note allows. Muse drops an answer of 12 KiB or more, so the engine's
/// text is cut, never the database note (the user's data) and not the guide.
#[test]
fn guide_and_database_note_fit_with_the_longest_engine_text_by_cutting_the_engines() {
    let r = Rig::new("guide-fit");
    let files: Vec<String> = (0..30)
        .map(|i| format!("data/a-very-long-database-file-name-{i}.sqlite3"))
        .collect();
    let turns = [(1, 2), (3, 4), (5, 6), (7, 8), (9, 10)];
    for (n, p) in turns {
        db_stop(&r, n, p, &files);
    }
    // quotes, backslashes, tabs, newlines and multibyte text: the encoded text is longer than the raw one
    let nasty = "\"quoted\" \\path\\ \t tab \u{e9}\u{20ac}\u{1f642} line\n";
    let engine = |chars: usize| {
        let context: String = nasty.chars().cycle().take(chars).collect();
        json!({"systemMessage": "\u{e9}".repeat(990), "hookSpecificOutput": {"hookEventName": "SessionStart", "additionalContext": context}})
            .to_string()
    };
    let mut chars = 3000;
    while engine(chars + 50).len() < 11_800 {
        chars += 50;
    }
    r.set("out.SessionStart", &engine(chars));
    for name in FIVE_SOURCES {
        let run = r.hook_ok("SessionStart", &r.fixture(name)); // under 12 KiB, closed keys
        let (said, ctx) = note_parts(&run.out);
        assert!(
            said.starts_with("\u{e9}\u{e9}\u{e9}") && said.chars().count() > 900,
            "{name}"
        );
        assert!(
            ctx.contains("\n[cut]\n\n[Torpor guide: stays valid] "),
            "{name}: the engine's text is cut"
        );
        for (_, p) in turns {
            assert!(
                ctx.contains(&format!("back --id {p} --with-data")),
                "{name}: {p}"
            );
        }
        assert!(
            ctx.ends_with("back --id 2 --with-data"),
            "{name}: the database note is last"
        );
        assert!(run.out.len() < 12 * 1024, "{name}: {} bytes", run.out.len());
    }
}

/// F4 item 1, the commands that are the user's own: under Muse the shell does not carry the marker the engine's command
/// line looks for, so what stops the model is the engine's PreToolUse answer, passed through. The commands the guide
/// does name pass.
#[test]
fn real_engine_refuses_the_users_own_commands_to_the_model_and_passes_the_guides_commands() {
    let r = Rig::real("only-you");
    fs::write(r.cwd.join("base.txt"), "keep\n").unwrap();
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    let engine = r.engine.display().to_string();
    let ask = |command: &str| -> Value {
        let mut p = r.fixture("PreToolUse-bash");
        p["tool_input"]["command"] = json!(command);
        let run = r.hook_ok("PreToolUse", &p);
        if run.out.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&run.out).unwrap()
        }
    };
    let users = [
        "torpor gate on",
        "torpor gate off",
        "torpor off",
        "torpor on",
        "torpor retain 90",
        "torpor forget",
        "torpor promise ok 1",
        "torpor promise drop 1",
        "torpor promise block 1",
        "torpor runner install",
        "torpor undo 1 --force",
        "cd /tmp && torpor gate on",
        &format!("{engine} gate on"),
    ];
    for command in users {
        let v = ask(command);
        let h = &v["hookSpecificOutput"];
        assert_eq!(h["permissionDecision"], "deny", "{command}: {v}");
        let why = h["permissionDecisionReason"].as_str().unwrap();
        assert!(
            why.contains("only the user can do this"),
            "{command}: {why}"
        );
    }
    let guides = [
        "torpor log -n 5",
        "torpor undo 1 --dry-run",
        "torpor undo 1",
        "torpor back --turn 1 --dry-run",
        "torpor back --id 2 --with-data --dry-run",
        "torpor find-break --promise 1",
        "torpor find-break --check -",
        "torpor check 1",
        "torpor promise dispute 1 \"the check is outdated\"",
        &format!("{engine} undo 1 --dry-run"),
    ];
    for command in guides {
        let v = ask(command);
        assert!(
            v["hookSpecificOutput"]["permissionDecision"].is_null(),
            "{command}: {v}"
        );
    }
}

/// L3 N7: Muse shows `systemMessage` on one line and drops its newlines, so the engine's lines ran together. They are
/// joined with " — " now; what the model reads keeps its newlines.
#[test]
fn the_engine_lines_stay_apart_on_the_one_line_muse_shows() {
    let r = Rig::new("one-line");
    r.set(
        "out.Stop",
        r#"{"systemMessage":"Torpor: save point 4. undo it: torpor undo 3\nTorpor: new promise #1 \"x\" (guard only), from your words at turn 2. Drop it: torpor promise drop 1"}"#,
    );
    let said = note_parts(&r.hook_ok("Stop", &r.fixture("Stop")).out).0;
    assert_eq!(
        said,
        "Torpor: save point 4. undo it: torpor undo 3 — Torpor: new promise #1 \"x\" (guard only), from your words at turn 2. Drop it: torpor promise drop 1"
    );
    r.set(
        "out.UserPromptSubmit",
        r#"{"systemMessage":"one\ntwo","hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"c1\nc2"}}"#,
    );
    let (said, ctx) = note_parts(
        &r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"))
            .out,
    );
    assert_eq!(said, "one — two");
    assert!(ctx.ends_with("]\nc1\nc2"), "{ctx}");
}

/// L3 N4: what a check the writer saved does not cover (its `gaps.json`) goes, short, into the engine's line that
/// announces the promise, and in full to the model of the session that asked for it, once, at its next prompt.
#[test]
fn what_a_new_promise_does_not_cover_reaches_the_user_and_the_model() {
    let r = Rig::new("gap-note");
    let ws = writer_ws(&r);
    let gap = "editing a note, and that the notes survive a restart of the server";
    let gaps = json!([{"promise": 2, "session": SID, "gap": gap}, {"promise": 1, "session": SID, "gap": "older"}]);
    fs::write(ws.join("gaps.json"), gaps.to_string()).unwrap();
    r.set(
        "out.Stop",
        &json!({"systemMessage": "Torpor: save point 4. undo it: torpor undo 3\nTorpor: new promise #2 \"Adding (and listing) notes works\" (guard only), from your words at turn 2. Drop it: torpor promise drop 2"}).to_string(),
    );
    let said = note_parts(&r.hook_ok("Stop", &r.fixture("Stop")).out).0;
    assert!(
        said.contains(&format!("\"Adding (and listing) notes works\" (guard only; not covered: {gap}), from your words at turn 2. Drop it: torpor promise drop 2")),
        "{said}"
    );
    let ctx = context_of(&r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit")));
    assert!(
        ctx.contains(&format!("#2 does not cover: {gap}"))
            && ctx.contains("#1 does not cover: older"),
        "{ctx}"
    );
    let again = context_of(&r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit")));
    assert!(!again.contains("does not cover"), "once: {again}");
    // another session in the folder did not ask for these checks: its model is not told
    let mut other = r.fixture("UserPromptSubmit");
    other["session_id"] = json!("00000000-0000-7000-8000-0000000000aa");
    assert!(!context_of(&r.hook_ok("UserPromptSubmit", &other)).contains("does not cover"));
}

/// Z item D: a check the user's engine keeps reaches the AI again at a resume and after a compaction, in the engine's
/// own `Promises:` line (Torpor adds no note of its own for it). After a compaction the engine sends it once the
/// transcript shows the compaction (its `compact_boundary` record); without one the AI hears nothing from the engine.
#[test]
fn real_engine_a_saved_check_reaches_the_ai_at_a_resume_and_after_a_compaction() {
    const WORDS: &str = "the proof page must answer";
    const SPEC: &str = r#"{"v":1,"name":"proof page answers","why":"the proof page must answer","start":{"cmd":"python3 -m http.server {port} --bind 127.0.0.1","ready":"/","timeout_s":90},"env":{},"steps":[{"http":{"method":"GET","path":"/proof.html"},"expect":{"status":200},"why":"the proof page answers"}],"unverifiable":false}"#;
    let r = Rig::real("real-checks-reach-the-ai");
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    r.hook_ok("Stop", &r.fixture("Stop"));
    // the page is made by hand, so the check is provable and the engine keeps it
    fs::write(r.cwd.join("proof.html"), "<p>proof</p>\n").unwrap();
    let (code, text) = engine_cli_stdin(&r, &["keep", WORDS, "--check", "-"], SPEC);
    assert_eq!(code, 0, "{text}");
    let promise = "Promises: #1 \"proof page answers\" (shadow, last passed)";

    let ctx = context_of(&r.hook_ok("SessionStart", &r.fixture("SessionStart-resume")));
    assert!(ctx.contains(promise), "resume: {ctx}");

    r.hook_ok("PreCompact", &r.fixture("PreCompact"));
    let compact = r.fixture("SessionStart-compact");
    let ctx = context_of(&r.hook_ok("SessionStart", &compact));
    assert!(
        !ctx.contains("Promises:"),
        "no compaction in the transcript: {ctx}"
    );
    let transcript = r.dir.join("transcript.jsonl");
    fs::write(
        &transcript,
        "{\"type\":\"user\",\"uuid\":\"u1\",\"message\":\"hello\"}\n{\"type\":\"system\",\"subtype\":\"compact_boundary\",\"uuid\":\"b1\"}\n",
    )
    .unwrap();
    let mut compact = compact;
    compact["transcript_path"] = json!(transcript);
    let ctx = context_of(&r.hook_ok("SessionStart", &compact));
    assert!(
        ctx.contains("after compaction 1") && ctx.contains(promise),
        "compaction: {ctx}"
    );
}

// ---------------------------------------------------------------- F4b: the gate is off until the user turns it on

/// The engine's promise gate is on in every folder it saves. The user chose hands-free (a broken check is told to the AI
/// before it says Done), so the first SessionStart in a saved folder switches it off there, once. After that the gate is the
/// user's: a `torpor gate on` typed in their own terminal stays on through every later session start, whatever its source.
/// The marker that keeps it so is a file in the store, which the engine's own guard refuses the model.
#[test]
fn real_engine_the_gate_is_off_in_a_new_folder_and_a_gate_on_of_the_users_stays_on() {
    let r = Rig::real("gate-default");
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    let state = gate_state(&r);
    assert!(state.starts_with("The gate is off"), "{state}");

    // the user turns it on in their own terminal (no CLAUDECODE there)
    let (code, text) = engine_cli(&r, &["gate", "on"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(gate_state(&r), "The gate is on.");
    for source in [
        "SessionStart",
        "SessionStart-resume",
        "SessionStart-clear",
        "SessionStart-compact",
        "SessionStart-fork",
    ] {
        r.hook_ok("SessionStart", &r.fixture(source));
        assert_eq!(gate_state(&r), "The gate is on.", "{source}");
    }

    // the model cannot take the marker away: the engine refuses a command on a file of its store
    let marker = fs::read_dir(r.store().join("muse/gate"))
        .unwrap()
        .next()
        .expect("one marker for the folder")
        .unwrap()
        .path();
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    let mut p = r.fixture("PreToolUse-bash");
    p["tool_input"]["command"] = json!(format!("rm {}", marker.display()));
    let run = r.hook_ok("PreToolUse", &p);
    assert!(
        run.out.contains(r#""permissionDecision":"deny""#),
        "{}",
        run.out
    );
    assert!(marker.exists());
}

/// A hook's environment may carry CLAUDECODE (Muse started from inside Claude Code): the engine refuses `gate off` then, as
/// it does for the model. Our `gate off` call does not carry it, so the default is still set.
#[test]
fn real_engine_the_gate_default_is_set_although_the_hooks_environment_has_claudecode() {
    let r = Rig::real("gate-default-cc");
    let payload = r.fixture("SessionStart").to_string();
    let run = r.hook_env("SessionStart", payload.as_bytes(), &[("CLAUDECODE", "1")]);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(r.problems().is_empty(), "{}", r.problems());
    let state = gate_state(&r);
    assert!(state.starts_with("The gate is off"), "{state}");
}

/// The gate default with a stand-in engine: `gate off` runs once per folder and only where the engine says it is saving. A
/// `gate off` that fails is one line in problems.log and not a word to the model; nothing is marked, so the next session
/// start tries again.
#[test]
fn the_gate_default_runs_once_per_folder_only_where_saving_and_a_failure_is_one_log_line() {
    let r = Rig::new("gate-default-fake");
    let off_calls = || r.calls().iter().filter(|c| *c == "gate off").count();
    let start = |source: &str| r.hook_ok("SessionStart", &r.fixture(source));

    // `torpor off` was typed in this folder: nothing is run
    r.set("out.status", r#"{"saving":false,"off":true}"#);
    start("SessionStart");
    assert_eq!(off_calls(), 0);

    // saving, but `gate off` fails: one log line, nothing for the model, no marker
    r.set("out.status", r#"{"saving":true}"#);
    r.set("code.gate", "1");
    let run = start("SessionStart");
    assert_eq!(off_calls(), 1);
    let log = r.problems();
    assert!(
        log.lines().count() == 1 && log.contains("gate default not set"),
        "{log}"
    );
    assert!(
        !run.out.contains("gate default") && !run.out.contains("not set"),
        "{}",
        run.out
    );

    // tried again at the next start; it works now, and is not run again in that folder
    fs::remove_file(r.fake.join("code.gate")).unwrap();
    start("SessionStart-resume");
    assert_eq!(off_calls(), 2);
    for source in ["SessionStart", "SessionStart-clear", "SessionStart-compact"] {
        start(source);
    }
    assert_eq!(off_calls(), 2, "once per folder");
    assert_eq!(r.problems().lines().count(), 1, "{}", r.problems());

    // another folder is its own
    let other = r.dir.join("other");
    fs::create_dir_all(&other).unwrap();
    let mut p = r.fixture("SessionStart");
    p["cwd"] = json!(other.to_str().unwrap());
    r.hook_ok("SessionStart", &p);
    assert_eq!(off_calls(), 3);
}

/// `gate off` first, the marker second: when the marker cannot be written the gate is off all the same, there is one line in
/// problems.log, and the next start runs `gate off` again (the other order would leave a marker with the gate still on, and
/// the user's choice would never apply). Once the marker can be written it is the last time.
#[test]
fn the_gate_default_is_set_before_it_is_marked_and_a_marker_that_cannot_be_written_is_one_log_line()
{
    let r = Rig::new("gate-default-mark");
    let off_calls = || r.calls().iter().filter(|c| *c == "gate off").count();
    r.set("out.status", r#"{"saving":true}"#);
    let muse = r.store().join("muse");
    fs::create_dir_all(&muse).unwrap();
    fs::write(
        muse.join("gate"),
        "a file where the marker folder should be",
    )
    .unwrap();

    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    assert_eq!(off_calls(), 1);
    let log = r.problems();
    assert!(
        log.lines().count() == 1 && log.contains("not remembered"),
        "{log}"
    );

    r.hook_ok("SessionStart", &r.fixture("SessionStart-resume"));
    assert_eq!(off_calls(), 2, "not marked: tried again");

    fs::remove_file(muse.join("gate")).unwrap();
    r.hook_ok("SessionStart", &r.fixture("SessionStart-clear"));
    assert_eq!(off_calls(), 3);
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    assert_eq!(off_calls(), 3, "marked now");
}

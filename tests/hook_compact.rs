//! Muse installs some compactions by itself and runs no SessionStart hook after them. Then the guide and the engine's
//! note come with the next prompt or tool result instead, once, and never after a compaction that did get its
//! SessionStart (src/hook/compact_note.rs). The real binary, a fake engine (and the pinned one in the last test), and a
//! synthetic Muse log that grows the way Muse's does.

mod hook_common;
#[path = "shim_common.rs"]
mod shim;

use hook_common::*;
use serde_json::{Value, json};
use shim::{LogBuilder, append, put_log};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

/// What the fake engine prints for a SessionStart once the session is under way: the note that names the saved check.
const NOTE: &str = "Torpor (after compaction, valid as of turn 4): check #1 stays saved";
const GUIDE: &str = "[Torpor guide: stays valid]";
const CHILD: &str = "00000000-0000-7000-8000-0000000000c1";

fn out(event: &str, context: &str) -> String {
    json!({"hookSpecificOutput": {"hookEventName": event, "additionalContext": context}})
        .to_string()
}

fn times(text: &str, what: &str) -> usize {
    text.matches(what).count()
}

/// One Muse session: its log on disk, the hooks Muse would run, and the compactions it installs.
struct Chat {
    r: Rig,
    b: LogBuilder,
    log: PathBuf,
    turns: u32,
}

impl Chat {
    /// A session that has started (source startup, the engine printing nothing yet).
    fn new(name: &str) -> Chat {
        Chat::on(Rig::new(name))
    }

    fn on(r: Rig) -> Chat {
        let mut b = LogBuilder::new(SID);
        b.metadata("1.4.13");
        let sessions = r.dir.join("home/.local/share/muse/sessions");
        let log = put_log(&sessions, SID, &b.new_text());
        let c = Chat {
            r,
            b,
            log,
            turns: 0,
        };
        c.r.hook_ok("SessionStart", &c.r.fixture("SessionStart"));
        c.r.set("out.SessionStart", &out("SessionStart", NOTE));
        c
    }

    fn prompt(&mut self) -> Run {
        self.turns += 1;
        self.b.accepted("chat", "main");
        append(&self.log, &self.b.new_text());
        let mut p = self.r.fixture("UserPromptSubmit");
        p["turn_id"] = json!(format!("00000000-0000-7000-8000-{:012}", self.turns + 1));
        self.r.hook_ok("UserPromptSubmit", &p)
    }

    fn tool(&self) -> Run {
        self.r
            .hook_ok("PostToolUse", &self.r.fixture("PostToolUse-bash"))
    }

    /// Muse installs a compaction: the record is in the log, no hook runs.
    fn compact(&mut self) {
        self.b.installed("run-1", "soft_threshold_async");
        append(&self.log, &self.b.new_text());
    }

    fn starts(&self) -> Vec<Value> {
        self.r.got("SessionStart")
    }
}

/// The guide and the note, each once, the note first.
fn told_once(ctx: &str) {
    assert_eq!((times(ctx, GUIDE), times(ctx, NOTE)), (1, 1), "{ctx}");
    assert!(ctx.find(NOTE) < ctx.find(GUIDE), "{ctx}");
}

#[test]
fn a_compaction_with_no_session_start_is_told_at_the_next_prompt_once() {
    let mut c = Chat::new("compact-prompt");
    assert_eq!(c.prompt().out, "", "a clean prompt adds nothing");
    assert_eq!(c.tool().out, "", "nor does a clean tool result");
    c.compact();
    let run = c.prompt();
    told_once(&context_of(&run));
    // the engine was asked what a SessionStart (compact) asks it: the session's folder, the translated log, no prompt
    let asks = c.starts();
    assert_eq!(asks.len(), 2, "the start, and the stand-in");
    let ask = &asks[1];
    assert_eq!(
        (ask["source"].as_str(), ask["hook_event_name"].as_str()),
        (Some("compact"), Some("SessionStart"))
    );
    assert_eq!(ask["session_id"], SID);
    assert_eq!(ask["cwd"], c.r.cwd.to_str().unwrap());
    let log = ask["transcript_path"].as_str().unwrap();
    assert!(
        log.ends_with(&format!("muse/transcripts/{SID}.jsonl")),
        "{log}"
    );
    for k in ["prompt", "turn_id", "tool_name", "agent_id"] {
        assert!(ask.get(k).is_none(), "{k} in {ask}");
    }
    // the prompt reached the engine as it was
    let prompts = c.r.got("UserPromptSubmit");
    assert_eq!(prompts.len(), 2);
    assert!(
        prompts[1]["prompt"]
            .as_str()
            .unwrap()
            .starts_with("Make exactly one bash tool call")
    );
    // nothing again, at a prompt or a tool result, and the engine is not asked again
    for _ in 0..2 {
        assert_eq!(c.prompt().out, "");
        assert_eq!(c.tool().out, "");
    }
    assert_eq!(c.starts().len(), 2);
}

#[test]
fn the_first_tool_result_after_a_compaction_can_carry_it() {
    let mut c = Chat::new("compact-tool");
    c.prompt();
    c.compact();
    let run = c.tool();
    told_once(&context_of(&run));
    assert!(
        run.out.contains("\"hookEventName\":\"PostToolUse\""),
        "{}",
        run.out
    );
    assert_eq!(c.tool().out, "", "the next tool result");
    assert_eq!(c.prompt().out, "", "and the next prompt");
    assert_eq!(c.starts().len(), 2);
}

#[test]
fn a_compaction_that_got_its_session_start_is_not_told_again() {
    let mut c = Chat::new("compact-hooked");
    c.prompt();
    c.compact();
    let ctx = context_of(&c.r.hook_ok("SessionStart", &c.r.fixture("SessionStart-compact")));
    told_once(&ctx);
    for _ in 0..2 {
        assert_eq!(c.prompt().out, "");
        assert_eq!(c.tool().out, "");
    }
    assert_eq!(
        c.starts().len(),
        2,
        "the start and the compact one, nothing asked in their place"
    );
    // the next compaction without a start hook is told, as the first one would have been
    c.compact();
    told_once(&context_of(&c.prompt()));
    assert_eq!(c.starts().len(), 3);
}

#[test]
fn two_compactions_before_the_next_hook_are_one_note_and_a_later_one_is_told_again() {
    let mut c = Chat::new("compact-two");
    c.prompt();
    c.compact();
    c.compact();
    told_once(&context_of(&c.prompt()));
    assert_eq!(c.prompt().out, "");
    c.compact();
    told_once(&context_of(&c.tool()));
    assert_eq!(c.tool().out, "");
    assert_eq!(c.starts().len(), 3);
}

#[test]
fn without_a_compaction_nothing_is_added_and_the_engine_is_asked_for_no_extra_start() {
    let mut c = Chat::new("compact-none");
    for _ in 0..3 {
        assert_eq!(c.prompt().out, "");
        assert_eq!(c.tool().out, "");
    }
    assert_eq!(c.starts().len(), 1);
}

#[test]
fn a_subagents_tool_result_does_not_take_it() {
    let mut c = Chat::new("compact-child");
    c.prompt();
    c.compact();
    let mut p = c.r.fixture("PostToolUse-bash");
    p["session_id"] = json!(CHILD);
    p["turn_id"] = json!(CHILD);
    assert_eq!(c.r.hook_ok("PostToolUse", &p).out, "");
    assert_eq!(c.starts().len(), 1, "the engine is not asked for the child");
    told_once(&context_of(&c.tool()));
}

#[test]
fn a_session_from_before_the_count_is_not_told_about_the_past_but_about_the_next_one() {
    let mut c = Chat::new("compact-old");
    c.prompt();
    c.compact();
    // as the session file was before `compact_told`: no such key
    let file = c.r.store().join("muse/session").join(format!("{SID}.json"));
    let mut s: Value = serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert!(s["compact_told"].is_number(), "{s}");
    s.as_object_mut().unwrap().remove("compact_told");
    fs::write(&file, s.to_string()).unwrap();
    assert_eq!(c.prompt().out, "", "not known whether it was told: nothing");
    assert_eq!(c.starts().len(), 1);
    c.compact();
    told_once(&context_of(&c.prompt()));
}

#[test]
fn an_engine_that_fails_for_the_stand_in_still_gives_the_guide_and_is_not_asked_again() {
    let mut c = Chat::new("compact-fail");
    c.prompt();
    c.compact();
    c.r.set("code.SessionStart", "1");
    c.r.set("err.SessionStart", "boom");
    let ctx = context_of(&c.prompt());
    assert_eq!((times(&ctx, GUIDE), times(&ctx, NOTE)), (1, 0), "{ctx}");
    let problems = c.r.problems();
    assert!(
        problems.contains("no compaction note") && problems.contains("boom"),
        "{problems}"
    );
    assert_eq!(c.prompt().out, "");
    assert_eq!(c.tool().out, "");
    assert_eq!(c.starts().len(), 2);
}

/// The stand-in is asked before the hook's own call (the engine builds its note for a compaction before it opens the
/// next turn); its `#n` header is lower than the one of the hook's own text and it comes first, as a real SessionStart
/// before the prompt would have it; a database turn is told once, not by both.
#[test]
fn the_note_comes_first_with_the_lower_number_and_a_database_turn_is_told_once() {
    let mut r = Rig::new("compact-order");
    let order = r.fake.join("order.log");
    let wrap = r.fake.join("wrap");
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> {}\nexec {} \"$@\"\n",
        order.display(),
        r.engine.display()
    );
    fs::write(&wrap, script).unwrap();
    fs::set_permissions(&wrap, fs::Permissions::from_mode(0o755)).unwrap();
    r.engine = wrap;
    let mut c = Chat::on(r);
    c.prompt();
    // a turn that changed a database: kept for the folder, not told yet
    c.r.set(
        "out.status",
        &json!({"saving": true, "last_turn": {"n": 3}}).to_string(),
    );
    let show = json!({"turn": {"n": 3, "start_point": 9}, "files": [{"path": "notes.db", "class": "tracked", "change": "deleted"}]});
    c.r.set("out.show.3", &show.to_string());
    stop_run(&c.r, 3);
    c.compact();
    c.r.set(
        "out.UserPromptSubmit",
        &out("UserPromptSubmit", "REAL-PROMPT-TEXT"),
    );
    fs::write(&order, "").unwrap();
    let ctx = context_of(&c.prompt());
    let calls = fs::read_to_string(&order).unwrap();
    let hooks: Vec<&str> = calls.lines().filter(|l| l.starts_with("hook ")).collect();
    assert_eq!(
        hooks,
        ["hook SessionStart", "hook UserPromptSubmit"],
        "{calls}"
    );
    let at = |what: &str| {
        ctx.find(what)
            .unwrap_or_else(|| panic!("{what} missing: {ctx}"))
    };
    let (head, note, guide, db, real) = (
        at("[Torpor note #"),
        at(NOTE),
        at(GUIDE),
        at("Turn 3 changed notes.db"),
        at("REAL-PROMPT-TEXT"),
    );
    assert_eq!(head, 0, "{ctx}");
    assert!(note < guide && guide < db && db < real, "{ctx}");
    // the numbers rise in reading order, and the database note is there once
    let numbers: Vec<u64> = ctx
        .match_indices("[Torpor note #")
        .map(|(i, m)| {
            ctx[i + m.len()..]
                .split(':')
                .next()
                .unwrap()
                .parse()
                .unwrap()
        })
        .collect();
    assert_eq!(numbers.len(), 2, "{ctx}");
    assert!(numbers[0] < numbers[1], "{numbers:?}");
    assert_eq!(
        (times(&ctx, "Turn 3 changed"), times(&ctx, GUIDE)),
        (1, 1),
        "{ctx}"
    );
    // a prompt after it repeats nothing but what the engine says on its own
    let again = context_of(&c.prompt());
    assert!(
        !again.contains(GUIDE) && !again.contains("Turn 3 changed") && !again.contains(NOTE),
        "{again}"
    );
}

/// With the pinned engine: a check the user's words made, a compaction Muse installed without a start hook, and the
/// next prompt names the check, once.
#[test]
fn real_engine_a_saved_check_reaches_the_ai_at_the_prompt_after_a_compaction_without_a_start_hook()
{
    const WORDS: &str = "the proof page must answer";
    const SPEC: &str = r#"{"v":1,"name":"proof page answers","why":"the proof page must answer","start":{"cmd":"python3 -m http.server {port} --bind 127.0.0.1","ready":"/","timeout_s":90},"env":{},"steps":[{"http":{"method":"GET","path":"/proof.html"},"expect":{"status":200},"why":"the proof page answers"}],"unverifiable":false}"#;
    let mut c = Chat::on(Rig::real("compact-real"));
    c.prompt();
    c.r.hook_ok("Stop", &c.r.fixture("Stop"));
    fs::write(c.r.cwd.join("proof.html"), "<p>proof</p>\n").unwrap();
    let (code, text) = engine_cli_stdin(&c.r, &["keep", WORDS, "--check", "-"], SPEC);
    assert_eq!(code, 0, "{text}");
    let promise = "Promises: #1 \"proof page answers\"";
    c.compact();
    let ctx = context_of(&c.prompt());
    assert!(
        ctx.contains("after compaction 1") && ctx.contains(promise),
        "{ctx}"
    );
    assert_eq!(times(&ctx, GUIDE), 1, "{ctx}");
    let again = context_of(&c.prompt());
    assert!(
        !again.contains("Promises:") && !again.contains(GUIDE),
        "{again}"
    );
    // a compaction that got its start hook: the same text from it, and nothing more at the prompt
    c.compact();
    let ctx = context_of(&c.r.hook_ok("SessionStart", &c.r.fixture("SessionStart-compact")));
    assert!(
        ctx.contains("after compaction 2") && ctx.contains(promise),
        "{ctx}"
    );
    let after = context_of(&c.prompt());
    assert!(
        !after.contains("Promises:") && !after.contains(GUIDE),
        "{after}"
    );
}

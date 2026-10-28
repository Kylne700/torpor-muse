//! Muse's own session log -> the Claude-style transcript the engine reads (src/transcript.rs).
//! The fixtures are redacted copies of real Muse Code 1.4.13 logs; see tests/fixtures/shim.
// Each test crate compiles these src files on its own: what only the binary or another test crate calls looks unused.
#[allow(dead_code)]
#[path = "../src/liveness.rs"]
mod liveness;
#[allow(dead_code)]
#[path = "../src/transcript.rs"]
mod transcript;
#[allow(dead_code)]
#[path = "../src/util.rs"]
mod util;

#[path = "shim_common.rs"]
mod common;

use common::*;
use serde_json::{Value, json};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Instant;
use transcript::{Paths, State};

struct W {
    _tmp: Tmp,
    p: Paths,
}

fn world(name: &str) -> W {
    let tmp = Tmp::new(name);
    let p = Paths {
        vault: tmp.join("vault"),
        sessions: tmp.join("sessions"),
    };
    W { _tmp: tmp, p }
}

impl W {
    fn translate(&self, sid: &str) -> Vec<Value> {
        lines(&transcript::translate(&self.p, sid).unwrap())
    }
    fn state(&self, sid: &str) -> State {
        transcript::read_state(&self.p.vault, sid).unwrap()
    }
    fn put(&self, text: &str) -> (String, std::path::PathBuf) {
        let sid = sid_of(text);
        let log = put_log(&self.p.sessions, &sid, text);
        (sid, log)
    }
}

fn attachment(uuid: &str, text: &Value) -> Value {
    json!({"type": "attachment", "uuid": uuid,
           "attachment": {"type": "hook_additional_context", "content": [text]}})
}

/// A second reading of a log, written apart from src/transcript.rs: plain `Value`s, no serde structs, no text prefilter.
fn oracle(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    for l in text.lines() {
        let r: Value = serde_json::from_str(l).unwrap();
        let (p, id) = (&r["payload"], r["id"].as_str().unwrap_or(""));
        let ev = &p["event"];
        match r["payload_type"].as_str().unwrap_or("") {
            "runtime.user_intent.accepted" => {
                let human = p["surface"] == "main"
                    && ["chat", "skill_invocation", "plugin_command"]
                        .contains(&p["semantic_kind"]["kind"].as_str().unwrap_or(""));
                let mut v = json!({"type": "user", "uuid": p["intent_id"], "promptId": p["intent_id"],
                    "isSidechain": p["surface"].as_str().is_some_and(|s| s != "main"),
                    "turnOrigin": if human { "human" } else { "unknown" }});
                if human {
                    v["promptSource"] = json!("typed");
                }
                out.push(v);
            }
            "runtime.session" => match ev["kind"].as_str().unwrap_or("") {
                "context_compaction_installed" => out.push(json!({
                    "type": "system", "subtype": "compact_boundary", "uuid": id,
                    "content": "Conversation compacted",
                    "compactMetadata": {"trigger": if ev["trigger"] == "manual_command" { "manual" } else { "auto" }}})),
                "context_block_updated" if ev["source"] == "runtime_hook" => {
                    out.push(attachment(id, &ev["text"]))
                }
                "context_block_update_batch_committed" => {
                    for (i, e) in ev["entries"].as_array().into_iter().flatten().enumerate() {
                        if e["update"]["source"] == "runtime_hook" {
                            out.push(attachment(&format!("{id}:{i}"), &e["update"]["text"]));
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    out
}

fn of_type<'a>(v: &'a [Value], t: &str) -> Vec<&'a Value> {
    v.iter().filter(|x| x["type"] == t).collect()
}

// ---------------------------------------------------------------- real logs

#[test]
fn every_real_log_translates_like_the_second_reading() {
    for name in FIXTURES {
        let w = world(name);
        let (sid, log) = w.put(&fixture(name, "/h"));
        let got = w.translate(&sid);
        assert_eq!(got, oracle(&fixture(name, "/h")), "{name}");
        let st = w.state(&sid);
        assert_eq!(st.bad, 0, "{name}");
        assert_eq!(st.off, std::fs::metadata(&log).unwrap().len(), "{name}");
        assert_eq!(
            st.lines as usize,
            fixture(name, "/h").lines().count(),
            "{name}"
        );
        assert_eq!(st.users + st.bounds + st.ctx, got.len() as u64, "{name}");
    }
}

#[test]
fn what_each_real_log_holds() {
    // name: (prompts, compaction boundaries, hook contexts). The numbers are read off the redacted logs.
    let table = [
        ("batch", 1, 0, 4),
        ("clear_resume", 4, 0, 0),
        ("fork", 1, 0, 0),
        ("fork_compaction", 2, 0, 1),
        ("hard", 1, 3, 4),
        ("manual", 4, 1, 2),
        ("noise", 1, 0, 1),
        ("rejected", 1, 0, 2),
        ("soft", 1, 1, 1),
    ];
    for (name, users, bounds, ctx) in table {
        let w = world(name);
        let (sid, _) = w.put(&fixture(name, "/h"));
        let got = w.translate(&sid);
        let st = w.state(&sid);
        assert_eq!(
            (st.users, st.bounds, st.ctx),
            (users, bounds, ctx),
            "{name}"
        );
        assert_eq!(of_type(&got, "user").len() as u64, users, "{name}");
        assert_eq!(of_type(&got, "system").len() as u64, bounds, "{name}");
        assert_eq!(of_type(&got, "attachment").len() as u64, ctx, "{name}");
    }
}

#[test]
fn a_hard_compaction_is_an_auto_boundary_with_the_records_own_id() {
    let w = world("hard");
    let (sid, _) = w.put(&fixture("hard", "/h"));
    let got = w.translate(&sid);
    let b: Vec<&Value> = of_type(&got, "system");
    let ids: Vec<&str> = b
        .iter()
        .map(|x| &x["uuid"].as_str().unwrap()[..8])
        .collect();
    assert_eq!(ids, ["1afe44ed", "902dba0f", "8c95b520"]);
    for x in b {
        assert_eq!(x["subtype"], "compact_boundary");
        assert_eq!(x["compactMetadata"]["trigger"], "auto");
    }
    // The log's checkpoints carry copies of the prompt: they must not make more prompts.
    let raw = fixture("hard", "/h");
    let marker = r#""payload_type":"runtime.user_intent.accepted""#;
    assert!(
        raw.matches(marker).count() > 1,
        "the log should hold embedded copies"
    );
    assert_eq!(of_type(&got, "user").len(), 1);
    assert_eq!(got[0]["type"], "user"); // the prompt comes first, as in the log
}

#[test]
fn a_soft_compaction_is_a_boundary_and_muses_own_context_is_not_an_attachment() {
    let w = world("soft");
    let (sid, _) = w.put(&fixture("soft", "/h"));
    let got = w.translate(&sid);
    let b = of_type(&got, "system");
    assert_eq!(b.len(), 1);
    assert!(b[0]["uuid"].as_str().unwrap().starts_with("eb3124ca"));
    assert_eq!(b[0]["compactMetadata"]["trigger"], "auto");
    // Two context blocks in the log (session_identity, runtime_hook): one attachment.
    let raw = fixture("soft", "/h");
    assert!(raw.contains(r#""source":"session_identity""#));
    assert_eq!(of_type(&got, "attachment").len(), 1);
}

#[test]
fn a_manual_compaction_is_a_manual_boundary() {
    let w = world("manual");
    let (sid, _) = w.put(&fixture("manual", "/h"));
    let got = w.translate(&sid);
    let b = of_type(&got, "system");
    assert_eq!(b.len(), 1);
    assert!(b[0]["uuid"].as_str().unwrap().starts_with("24806e82"));
    assert_eq!(b[0]["compactMetadata"]["trigger"], "manual");
}

#[test]
fn batched_context_updates_become_attachments_with_their_own_ids() {
    let w = world("batch");
    let (sid, _) = w.put(&fixture("batch", "/h"));
    let got = w.translate(&sid);
    let a = of_type(&got, "attachment");
    assert_eq!(a.len(), 4);
    let batched: Vec<&&Value> = a
        .iter()
        .filter(|x| x["uuid"].as_str().unwrap().ends_with(":0"))
        .collect();
    assert_eq!(batched.len(), 1, "one entry of the batch record");
    for x in &a {
        assert_eq!(x["attachment"]["type"], "hook_additional_context");
        assert!(x["attachment"]["content"][0].is_string());
    }
}

#[test]
fn clear_and_resume_keep_the_session_and_add_no_lines() {
    let raw = fixture("clear_resume", "/h");
    assert!(raw.contains("session.resumed") && raw.contains("conversation_rewind.received"));
    let w = world("clear_resume");
    let (sid, _) = w.put(&raw);
    let got = w.translate(&sid);
    assert_eq!(got.len(), 4);
    assert!(got.iter().all(|x| x["type"] == "user"));
    // Muse keeps the session id across /clear and resume, so the same file goes on (nothing starts over).
    let t = transcript::transcript_file(&w.p.vault, &sid);
    assert!(t.ends_with(format!("muse/transcripts/{sid}.jsonl")));
    assert_eq!(w.translate(&sid).len(), 4);
}

#[test]
fn a_forked_session_has_its_own_transcript() {
    let w = world("fork");
    let (parent, _) = w.put(&fixture("hard", "/h"));
    let (fork, _) = w.put(&fixture("fork", "/h"));
    let (fork2, _) = w.put(&fixture("fork_compaction", "/h"));
    assert_ne!(parent, fork);
    let (a, b, c) = (
        w.translate(&parent),
        w.translate(&fork),
        w.translate(&fork2),
    );
    let files: Vec<_> = [&parent, &fork, &fork2]
        .iter()
        .map(|s| transcript::transcript_file(&w.p.vault, s))
        .collect();
    assert!(files.iter().all(|f| f.is_file()));
    assert_eq!((a.len(), b.len(), c.len()), (1 + 3 + 4, 1, 3));
    // The fork log holds a copy of the parent's compaction and prompts (replayed, other ids): none of that is a line.
    let raw = fixture("fork_compaction", "/h");
    assert!(raw.contains("session.fork.compaction"));
    let marker = r#""payload_type":"runtime.user_intent.accepted""#;
    assert!(raw.matches(marker).count() > 2);
    assert_eq!(of_type(&c, "user").len(), 2);
    assert_eq!(of_type(&c, "system").len(), 0);
}

// ---------------------------------------------------------------- who typed it

#[test]
fn a_prompt_the_person_typed_is_human() {
    let w = world("typed");
    let mut b = LogBuilder::new("s-typed");
    let chat = b.accepted("chat", "main");
    let skill = b.accepted("skill_invocation", "main");
    let plugin = b.accepted("plugin_command", "main");
    w.put_synthetic(&b);
    let got = w.translate("s-typed");
    for (line, id) in got.iter().zip([chat, skill, plugin]) {
        assert_eq!(
            *line,
            json!({"type": "user", "uuid": id, "promptId": id, "isSidechain": false,
                   "turnOrigin": "human", "promptSource": "typed"})
        );
    }
}

/// SYNTHETIC: Muse's logs hold no injected prompt (every real accepted record is chat, skill_invocation or
/// plugin_command on `main`), so the kind below is made up to test the rule "anything else is not human".
#[test]
fn a_prompt_that_is_not_a_typed_kind_is_not_human() {
    let w = world("injected");
    let mut b = LogBuilder::new("s-injected");
    let other = b.accepted("synthetic_injected", "main");
    let child = b.accepted("chat", "observer");
    w.put_synthetic(&b);
    let got = w.translate("s-injected");
    assert_eq!(
        got[0],
        json!({"type": "user", "uuid": other, "promptId": other, "isSidechain": false, "turnOrigin": "unknown"})
    );
    assert_eq!(got[1]["isSidechain"], true);
    assert_eq!(got[1]["turnOrigin"], "unknown");
    assert_eq!(got[1]["uuid"], json!(child));
    assert!(got[1].get("promptSource").is_none());
}

impl W {
    fn put_synthetic(&self, b: &LogBuilder) {
        put_log(&self.p.sessions, &b.sid, &b.text());
    }
}

// ---------------------------------------------------------------- reading as the log grows

#[test]
fn it_goes_on_from_the_offset_and_never_rewrites_what_it_wrote() {
    let w = world("incremental");
    let mut b = LogBuilder::new("s-inc");
    let p1 = b.accepted("chat", "main");
    b.installed(&p1, "hard_threshold_blocking");
    let log = put_log(&w.p.sessions, "s-inc", &b.text());
    let first = w.translate("s-inc");
    assert_eq!(first.len(), 2);
    let file = transcript::transcript_file(&w.p.vault, "s-inc");
    let bytes1 = std::fs::read(&file).unwrap();
    let st1 = w.state("s-inc");

    assert_eq!(w.translate("s-inc"), first, "nothing new: nothing changes");
    assert_eq!(std::fs::read(&file).unwrap(), bytes1);

    let before = b.text().len();
    let p2 = b.accepted("chat", "main");
    b.cbu(&p2, "runtime_hook", "the text");
    append(&log, &b.text()[before..]);
    let second = w.translate("s-inc");
    assert_eq!(second.len(), 4);
    assert_eq!(second[..2], first[..]);
    assert!(std::fs::read(&file).unwrap().starts_with(&bytes1));
    let st2 = w.state("s-inc");
    assert_eq!(st2.lines - st1.lines, 2, "only the new lines were read");
    assert_eq!(st2.off, std::fs::metadata(&log).unwrap().len());
    assert_eq!(second, oracle(&b.text()));
}

#[test]
fn a_line_muse_is_still_writing_is_left_for_the_next_call() {
    let w = world("partial");
    let mut b = LogBuilder::new("s-part");
    b.accepted("chat", "main");
    let one = b.text();
    b.accepted("chat", "main");
    let both = b.text();
    let cut = one.len() + 30;
    let log = put_log(&w.p.sessions, "s-part", &both[..cut]);
    assert_eq!(w.translate("s-part").len(), 1);
    assert_eq!(w.state("s-part").off as usize, one.len());
    append(&log, &both[cut..]);
    let got = w.translate("s-part");
    assert_eq!(got.len(), 2);
    assert_eq!(got, oracle(&both));
}

#[test]
fn a_batch_whose_state_was_lost_is_cut_back_and_redone() {
    let w = world("crash");
    let mut b = LogBuilder::new("s-crash");
    let p1 = b.accepted("chat", "main");
    let log = put_log(&w.p.sessions, "s-crash", &b.text());
    w.translate("s-crash");
    let state_file = w.p.vault.join("muse/transcripts/s-crash.state");
    let saved = std::fs::read(&state_file).unwrap();

    let before = b.text().len();
    b.installed(&p1, "soft_threshold_async");
    b.accepted("chat", "main");
    append(&log, &b.text()[before..]);
    w.translate("s-crash"); // writes the transcript, then the state
    std::fs::write(&state_file, saved).unwrap(); // as if the process died between the two

    let got = w.translate("s-crash");
    assert_eq!(got, oracle(&b.text()), "no line twice, none missing");
    assert_eq!(w.state("s-crash").bounds, 1);
}

#[test]
fn a_replaced_shorter_log_starts_over() {
    let w = world("shrink");
    let mut b = LogBuilder::new("s-shrink");
    for _ in 0..5 {
        b.accepted("chat", "main");
    }
    put_log(&w.p.sessions, "s-shrink", &b.text());
    assert_eq!(w.translate("s-shrink").len(), 5);
    let mut c = LogBuilder::new("s-shrink");
    c.accepted("skill_invocation", "main");
    put_log(&w.p.sessions, "s-shrink", &c.text());
    let got = w.translate("s-shrink");
    assert_eq!(got, oracle(&c.text()));
    assert_eq!(got.len(), 1);
}

#[test]
fn lines_that_are_not_text_or_not_json_do_not_stop_the_reading() {
    let w = world("junk");
    let mut b = LogBuilder::new("s-junk");
    b.accepted("chat", "main");
    let mut raw = b.text().into_bytes();
    raw.extend_from_slice(&[0xff, 0xfe, b'\n']); // not UTF-8
    raw.extend_from_slice(b"{\"payload_type\":\"runtime.user_intent.accepted\" broken\n"); // wanted but not JSON
    raw.extend_from_slice(b"\n");
    let before = b.text().len();
    b.accepted("chat", "main");
    raw.extend_from_slice(&b.text().as_bytes()[before..]);
    let dir = w.p.sessions.join("2026/10/26/s-junk");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("session.jsonl"), raw).unwrap();
    let got = w.translate("s-junk");
    assert_eq!(got.len(), 2);
    assert_eq!(
        w.state("s-junk").bad,
        1,
        "the broken record is counted, doctor says so"
    );
}

#[test]
fn muses_own_context_and_hook_runs_make_no_lines() {
    // Nothing is made up: the delivery check has to fail when the log does not show the text.
    let w = world("empty");
    let mut b = LogBuilder::new("s-empty");
    b.metadata("1.4.13");
    let p = b.accepted("chat", "main");
    b.cbu(&p, "session_identity", "Muse's own block");
    b.terminal(&p, "k:stop:1", "Stop", "completed", None, "");
    put_log(&w.p.sessions, "s-empty", &b.text());
    let got = w.translate("s-empty");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["type"], "user");
    assert!(of_type(&got, "attachment").is_empty());
}

#[test]
fn a_missing_log_is_an_error_and_makes_no_transcript_path() {
    let w = world("missing");
    let e = transcript::translate(&w.p, "no-such-session").unwrap_err();
    assert!(e.contains("no session log of no-such-session"), "{e}");
    let payload = json!({"session_id": "no-such-session"});
    assert_eq!(
        transcript::path_for_in(&w.p, "UserPromptSubmit", &payload),
        None
    );
    let errors = std::fs::read_to_string(w.p.vault.join("muse/shim-errors.log")).unwrap();
    assert!(errors.contains("transcript UserPromptSubmit no-such-session: no session log"));
}

#[test]
fn path_for_gives_the_transcript_of_the_payloads_session() {
    let w = world("pathfor");
    let (sid, _) = w.put(&fixture("manual", "/h"));
    let payload =
        json!({"session_id": sid, "transcript_path": null, "hook_event_name": "UserPromptSubmit"});
    let got = transcript::path_for_in(&w.p, "UserPromptSubmit", &payload).unwrap();
    assert_eq!(
        Path::new(&got),
        transcript::transcript_file(&w.p.vault, &sid)
    );
    assert_eq!(lines(Path::new(&got)), oracle(&fixture("manual", "/h")));
    // A session id that could leave the folder, or none at all: no path, nothing written.
    for bad in [
        json!({"session_id": "../x"}),
        json!({"session_id": ""}),
        json!({"x": 1}),
        json!({"session_id": 7}),
    ] {
        assert_eq!(transcript::path_for_in(&w.p, "Stop", &bad), None, "{bad}");
    }
    assert!(!w.p.vault.join("x.jsonl").exists());
}

#[test]
fn an_old_transcript_is_still_given_when_the_log_cannot_be_read_any_more() {
    let w = world("oldfile");
    let (sid, log) = w.put(&fixture("manual", "/h"));
    let payload = json!({"session_id": sid});
    let first = transcript::path_for_in(&w.p, "Stop", &payload).unwrap();
    std::fs::remove_file(&log).unwrap();
    let again = transcript::path_for_in(&w.p, "Stop", &payload).unwrap();
    assert_eq!(
        first, again,
        "the engine's delivery check then fails instead of passing"
    );
}

#[test]
fn files_are_private() {
    let w = world("modes");
    let (sid, _) = w.put(&fixture("noise", "/h"));
    w.translate(&sid);
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let dir = w.p.vault.join("muse/transcripts");
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&dir.join(format!("{sid}.jsonl"))), 0o600);
    assert_eq!(mode(&dir.join(format!("{sid}.state"))), 0o600);
}

#[test]
fn parallel_hooks_never_write_a_line_twice() {
    let w = world("parallel");
    let mut b = LogBuilder::new("s-par");
    let log = put_log(&w.p.sessions, "s-par", &b.text());
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| {
                while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                    let _ = transcript::translate(&w.p, "s-par");
                }
            });
        }
        for i in 0..40 {
            let before = b.text().len();
            let p = b.accepted("chat", "main");
            if i % 4 == 0 {
                b.installed(&p, "hard_threshold_blocking");
            }
            append(&log, &b.text()[before..]);
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    assert_eq!(w.translate("s-par"), oracle(&b.text()));
}

// ---------------------------------------------------------------- size and speed (generous: debug build, busy CI)

fn big_log(sid: &str, lines: usize, prompt_every: usize) -> (LogBuilder, String) {
    let mut b = LogBuilder::new(sid);
    let mut text = String::new();
    for i in 0..lines {
        if i % prompt_every == 0 {
            let before = b.text().len();
            b.accepted("chat", "main");
            text.push_str(&b.text()[before..]);
        } else {
            text.push_str(&filler_line(480));
        }
    }
    (b, text)
}

#[test]
fn a_long_session_is_read_once_and_then_only_what_is_new() {
    // Release build, 147 real logs: first call 4.7 ms at most, a call with nothing new 42 us.
    let w = world("long");
    let (mut b, text) = big_log("s-long", 10_000, 100);
    assert!(text.len() > 5_000_000 && text.len() < 8 << 20);
    let log = put_log(&w.p.sessions, "s-long", &text);
    let t = Instant::now();
    let first = w.translate("s-long");
    let cold = t.elapsed();
    assert_eq!(first.len(), 100);
    let st1 = w.state("s-long");
    assert_eq!(st1.lines, 10_000);

    let t = Instant::now();
    w.translate("s-long");
    let idle = t.elapsed();

    let add = b.accepted("chat", "main");
    let line = b.text().lines().last().unwrap().to_string();
    append(&log, &format!("{line}\n"));
    let t = Instant::now();
    let after = w.translate("s-long");
    let warm = t.elapsed();
    println!(
        "long session ({} bytes): first {cold:?}, nothing new {idle:?}, one new prompt {warm:?}",
        text.len()
    );
    assert_eq!(after.len(), 101);
    assert_eq!(after[100]["uuid"], json!(add));
    assert_eq!(
        w.state("s-long").lines,
        10_001,
        "the old lines were not read again"
    );
    assert!(cold.as_secs() < 10 && idle.as_millis() < 1500 && warm.as_millis() < 1500);
}

#[test]
fn a_1_5_mb_prompt_line_is_read_quickly() {
    let w = world("bigprompt");
    let mut b = LogBuilder::new("s-big");
    b.accepted("chat", "main");
    let mut rec: Value = serde_json::from_str(b.text().trim_end()).unwrap();
    rec["payload"]["model_messages"] = json!([{"role": "user", "content": "x".repeat(1_500_000)}]);
    let line = format!("{rec}\n");
    assert!(line.len() > 1_500_000);
    put_log(&w.p.sessions, "s-big", &format!("{line}{line}"));
    let t = Instant::now();
    let got = w.translate("s-big");
    let took = t.elapsed();
    println!("two 1.5 MB prompt lines: {took:?}");
    assert_eq!(got.len(), 2);
    assert!(took.as_secs() < 5);
}

#[test]
fn big_tool_output_lines_are_not_looked_at() {
    // Muse cuts a tool output to about 20 KB in its log, so a 1.5 MB output is never a 1.5 MB line.
    let w = world("tools");
    let mut text = String::new();
    for _ in 0..200 {
        text.push_str(&filler_line(22_000));
    }
    let mut b = LogBuilder::new("s-tools");
    b.accepted("chat", "main");
    text.push_str(&b.text());
    put_log(&w.p.sessions, "s-tools", &text);
    assert_eq!(w.translate("s-tools").len(), 1);
    assert_eq!(w.state("s-tools").bad, 0);
}

#[test]
fn a_long_log_is_first_read_from_its_newest_part_only() {
    let w = world("cold");
    let mut b = LogBuilder::new("s-cold");
    let old = b.accepted("chat", "main");
    let mut text = b.text();
    for _ in 0..(20 << 20) / 4096 {
        text.push_str(&filler_line(4000));
    }
    let before = b.text().len();
    let newest = b.accepted("chat", "main");
    text.push_str(&b.text()[before..]);
    let log = put_log(&w.p.sessions, "s-cold", &text);
    assert!(text.len() > 20 << 20);

    let got = w.translate("s-cold");
    assert_eq!(got.len(), 1, "{old} is history");
    assert_eq!(got[0]["uuid"], json!(newest));
    assert_eq!(
        w.state("s-cold").off,
        std::fs::metadata(&log).unwrap().len()
    );

    // After that it goes on from the offset.
    let before = b.text().len();
    let next = b.accepted("chat", "main");
    append(&log, &b.text()[before..]);
    let got = w.translate("s-cold");
    assert_eq!(got.len(), 2);
    assert_eq!(got[1]["uuid"], json!(next));
}

#[test]
fn the_first_line_of_the_newest_part_is_not_lost_when_the_cut_is_a_line_start() {
    const TAIL: usize = 8 << 20; // transcript.rs COLD_TAIL
    let w = world("cut");
    let mut b = LogBuilder::new("s-cut");
    b.accepted("chat", "main");
    let mut text = b.text();
    let before = text.len();
    let wanted = b.accepted("chat", "main");
    let second = b.text()[before..].to_string();
    for _ in 0..(2 << 20) / 1024 {
        text.push_str(&filler_line(1000));
    }
    let tail_start = text.len();
    text.push_str(&second);
    let f0 = filler_line(0).len(); // a filler of n bytes is f0 + n long
    while TAIL - (text.len() - tail_start) >= f0 + 4000 + f0 {
        text.push_str(&filler_line(4000));
    }
    let rest = TAIL - (text.len() - tail_start);
    text.push_str(&filler_line(rest - f0));
    assert_eq!(
        text.len() - tail_start,
        TAIL,
        "the tail starts at a line start"
    );
    put_log(&w.p.sessions, "s-cut", &text);
    let got = w.translate("s-cut");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["uuid"], json!(wanted));
}

#[test]
fn a_line_longer_than_16_mb_is_skipped_and_counted() {
    let w = world("overlong");
    let mut b = LogBuilder::new("s-long-line");
    b.accepted("chat", "main");
    let log = put_log(&w.p.sessions, "s-long-line", &b.text());
    assert_eq!(w.translate("s-long-line").len(), 1);
    let mut more = "x".repeat(17 << 20);
    more.push('\n');
    let before = b.text().len();
    b.accepted("chat", "main");
    more.push_str(&b.text()[before..]);
    append(&log, &more);
    let got = w.translate("s-long-line");
    assert_eq!(got.len(), 2, "the prompt after the long line is read");
    let st = w.state("s-long-line");
    assert_eq!(st.bad, 1);
    assert_eq!(st.off, std::fs::metadata(&log).unwrap().len());
}

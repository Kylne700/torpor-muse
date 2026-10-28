//! The engine binary (TORPOR_BIN) reading the transcripts src/transcript.rs makes from Muse's own session logs:
//! who typed a prompt, the context rebuild after a compaction, and the delivery check. Hooks are sent to the engine
//! the way the hook shim sends them (Claude-style payload, `transcript_path` from `transcript::path_for_in`).
//! Without TORPOR_BIN these tests say so and stop (they fail instead when CI is set).
// Each test crate compiles these src files on its own, so what only the binary (or another test crate) calls looks unused here.
#[allow(dead_code)]
#[path = "../src/liveness.rs"]
mod liveness;
#[allow(dead_code)]
#[path = "../src/transcript.rs"]
mod transcript;

#[path = "shim_common.rs"]
mod common;

use common::{LogBuilder, Tmp, append, fixture, lines, put_log, sid_of};
use serde_json::{Value, json};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn engine_bin() -> Option<PathBuf> {
    match std::env::var_os("TORPOR_BIN")
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        Some(p) => Some(p),
        None if std::env::var_os("CI").is_some() => {
            panic!("TORPOR_BIN must name the engine binary when CI is set")
        }
        None => {
            eprintln!("SKIPPED: TORPOR_BIN does not name the engine binary");
            None
        }
    }
}

/// One engine vault with one project folder, and Muse's session folder of that home.
struct Eng {
    _t: Tmp,
    bin: PathBuf,
    home: PathBuf,
    vault: PathBuf,
    proj: PathBuf,
    sessions: PathBuf,
}

impl Eng {
    fn new(name: &str, bin: PathBuf) -> Eng {
        let t = Tmp::new(name);
        let (home, vault, proj) = (t.join("home"), t.join("vault"), t.join("proj"));
        fs::create_dir_all(&proj).unwrap();
        fs::write(proj.join("a.txt"), "one\n").unwrap();
        Eng {
            sessions: home.join(".local/share/muse/sessions"),
            _t: t,
            bin,
            home,
            vault,
            proj,
        }
    }

    fn paths(&self) -> transcript::Paths {
        transcript::Paths {
            vault: self.vault.clone(),
            sessions: self.sessions.clone(),
        }
    }

    /// The engine with a cleared environment of its own (what Muse gives a hook), stdin as given.
    fn run(&self, args: &[&str], stdin: &str) -> (i32, String, String) {
        let mut child = Command::new(&self.bin)
            .args(args)
            .current_dir(&self.proj)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", "/usr/bin:/bin")
            .env("TORPOR_HOME", &self.vault)
            .env("TZ", "UTC")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// One hook call, as the shim makes it: the transcript is brought up to date first. Returns what the engine
    /// printed (null when nothing).
    fn hook(&self, sid: &str, event: &str, extra: Value) -> Value {
        let mut p = json!({"session_id": sid, "cwd": self.proj, "hook_event_name": event});
        let path = transcript::path_for_in(&self.paths(), event, &p)
            .unwrap_or_else(|| panic!("no transcript for {sid}"));
        p["transcript_path"] = json!(path);
        for (k, v) in extra.as_object().unwrap() {
            p[k] = v.clone();
        }
        let (code, out, err) = self.run(&["hook", event], &p.to_string());
        assert_eq!(code, 0, "{event}: {err}");
        if out.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&out).unwrap_or_else(|e| panic!("{event} printed {out:?}: {e}"))
        }
    }

    fn turn(&self, sid: &str, prompt_id: &str, prompt: &str) {
        let ups = json!({"prompt": prompt, "prompt_id": prompt_id});
        self.hook(sid, "UserPromptSubmit", ups);
        self.hook(sid, "Stop", json!({"stop_hook_active": false}));
    }

    fn cli(&self, args: &[&str]) -> Value {
        let (code, out, err) = self.run(args, "");
        assert_eq!(code, 0, "{args:?}: {err}");
        serde_json::from_str(&out).unwrap_or_else(|e| panic!("{args:?} printed {out:?}: {e}"))
    }

    /// Who the engine says made each turn, by turn number.
    fn who(&self) -> Vec<(u64, String)> {
        let mut v: Vec<(u64, String)> = self
            .cli(&["log", "--json"])
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                (
                    t["turn"].as_u64().unwrap(),
                    t["who"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        v.sort();
        v
    }

    fn transcript_lines(&self, sid: &str) -> Vec<Value> {
        lines(&transcript::transcript_file(&self.vault, sid))
    }
}

/// The ids of the prompts the transcript calls human, in order.
fn human_prompts(l: &[Value]) -> Vec<String> {
    l.iter()
        .filter(|x| x["type"] == "user" && x["turnOrigin"] == "human")
        .map(|x| x["promptId"].as_str().unwrap().to_string())
        .collect()
}

fn startup(e: &Eng, sid: &str) {
    e.hook(sid, "SessionStart", json!({"source": "startup"}));
}

#[test]
fn a_prompt_typed_by_the_person_is_human_and_an_injected_one_is_not() {
    let Some(bin) = engine_bin() else { return };
    let e = Eng::new("origin", bin);
    // Turn 1 has the shape of a real capture (chat on the main surface). The other two are made up, because no
    // injected prompt was ever captured: a kind Muse does not use for typed prompts, and a prompt of a child session.
    let sid = "01a10000-0000-7000-8000-0000000000e1";
    let mut b = LogBuilder::new(sid);
    b.metadata("1.4.13");
    let typed = b.accepted("chat", "main");
    let log = put_log(&e.sessions, sid, &b.new_text());
    startup(&e, sid);
    e.turn(sid, &typed, "make a thing");
    let injected = b.accepted("background_result", "main");
    let child = b.accepted("chat", "child");
    append(&log, &b.new_text());
    e.turn(sid, &injected, "a result arrived");
    e.turn(sid, &child, "a child asks");
    let who = e.who();
    assert_eq!(
        who,
        [(1, "you".to_string()), (2, "?".into()), (3, "?".into())],
        "only the typed prompt is the person's: {who:?}"
    );
    let t = e.transcript_lines(sid);
    assert_eq!(human_prompts(&t), [typed]);
    assert!(
        t.iter()
            .any(|x| x["promptId"] == json!(child) && x["isSidechain"] == true)
    );
}

/// A real captured compaction through the engine, replayed the way the engine lives it: the log up to the first
/// compaction is there while the first turns run; then the capture arrives piece by piece, and after each compaction
/// in it the engine is told the session started again (source `compact`), as Muse's SessionStart hook does.
/// `deliver`: whether Muse's log then holds each rebuild text (as it does when Muse injected it). Returns the rebuild
/// texts and the engine's status after one more turn.
fn compaction(bin: PathBuf, fx: &str, deliver: bool) -> (Vec<String>, Value) {
    let e = Eng::new(&format!("compaction-{fx}-{deliver}"), bin);
    let text = fixture(fx, &e.home.to_string_lossy());
    let sid = sid_of(&text);
    let all: Vec<&str> = text.split_inclusive('\n').collect();
    let cuts: Vec<usize> = (0..all.len())
        .filter(|&i| all[i].contains("\"kind\":\"context_compaction_installed\""))
        .collect();
    assert!(!cuts.is_empty(), "a compaction in the capture");
    let log = put_log(&e.sessions, &sid, &all[..cuts[0]].concat());
    startup(&e, &sid);
    for id in human_prompts(&e.transcript_lines(&sid)) {
        e.turn(&sid, &id, "a turn before the compaction");
    }
    let mut b = LogBuilder::new(&sid);
    let mut rebuilds = Vec::new();
    for (k, &from) in cuts.iter().enumerate() {
        let to = cuts.get(k + 1).copied().unwrap_or(all.len());
        append(&log, &all[from..to].concat());
        let out = e.hook(&sid, "SessionStart", json!({"source": "compact"}));
        let ctx = out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap_or_else(|| panic!("no context after compaction {}: {out}", k + 1))
            .to_string();
        if deliver {
            b.cbu("run-after", "runtime_hook", &ctx);
            append(&log, &b.new_text());
        }
        rebuilds.push(ctx);
    }
    let next = b.accepted("chat", "main");
    append(&log, &b.new_text());
    e.turn(&sid, &next, "and then?");
    assert!(
        transcript::transcript_file(&e.vault, &sid).is_file(),
        "the engine left our files alone"
    );
    (rebuilds, e.cli(&["status", "--json"]))
}

#[test]
fn each_real_installed_compaction_is_one_rebuild_for_the_engine() {
    let Some(bin) = engine_bin() else { return };
    // manual: 1 compaction, soft (async, after the Stop): 1, hard (mid turn, three in a row): 3
    for (fx, n) in [("manual", 1), ("soft", 1), ("hard", 3)] {
        let (rebuilds, status) = compaction(bin.clone(), fx, true);
        assert_eq!(rebuilds.len(), n, "{fx}");
        for (k, ctx) in rebuilds.iter().enumerate() {
            let want = format!("Torpor (after compaction {},", k + 1);
            assert!(ctx.starts_with(&want), "{fx}: {ctx}");
        }
        assert_eq!(status["context"]["rebuilds"], n, "{fx}: {status}");
    }
}

#[test]
fn the_delivery_check_passes_when_muses_log_holds_the_text() {
    let Some(bin) = engine_bin() else { return };
    for fx in ["manual", "hard"] {
        let (_, status) = compaction(bin.clone(), fx, true);
        assert_eq!(
            status["context"]["rebuilds_not_delivered"], 0,
            "{fx}: {status}"
        );
    }
}

/// The negative control: the real log of `hard` holds other hook blocks after every compaction, but not this text.
#[test]
fn the_delivery_check_fails_when_muses_log_does_not_hold_the_text() {
    let Some(bin) = engine_bin() else { return };
    for (fx, n) in [("manual", 1), ("hard", 3)] {
        let (_, status) = compaction(bin.clone(), fx, false);
        assert_eq!(status["context"]["rebuilds"], n, "{fx}: {status}");
        assert_eq!(
            status["context"]["rebuilds_not_delivered"], n,
            "{fx}: {status}"
        );
    }
}

#[test]
fn a_forked_session_has_a_transcript_of_its_own() {
    let Some(bin) = engine_bin() else { return };
    let e = Eng::new("fork", bin);
    // the parent is made up; the fork is a real capture (/rewind: a new session id, the files stay)
    let parent = "01a10000-0000-7000-8000-0000000000f0";
    let mut b = LogBuilder::new(parent);
    let p1 = b.accepted("chat", "main");
    put_log(&e.sessions, parent, &b.new_text());
    startup(&e, parent);
    e.turn(parent, &p1, "first");
    let text = fixture("fork", &e.home.to_string_lossy());
    let fork = sid_of(&text);
    put_log(&e.sessions, &fork, &text);
    startup(&e, &fork);
    let mine = human_prompts(&e.transcript_lines(&fork));
    assert!(!mine.is_empty());
    assert_ne!(
        transcript::transcript_file(&e.vault, parent),
        transcript::transcript_file(&e.vault, &fork)
    );
    assert_eq!(human_prompts(&e.transcript_lines(parent)), [p1]);
    e.turn(&fork, &mine[0], "after the rewind");
    let who = e.who();
    assert_eq!(who.last().unwrap().1, "you", "{who:?}");
    assert_eq!(who.len(), 2, "{who:?}");
}

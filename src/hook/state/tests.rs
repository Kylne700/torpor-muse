//! Tests of the hook's own small state files: session state, marks, open turns, database turns and the problems log.

use super::*;
use crate::engine::testing::scratch;
use std::time::UNIX_EPOCH;

#[test]
fn the_stop_cap_is_two_per_turn_and_old_turns_are_forgotten() {
    let mut s = Session::default();
    assert!(s.allow_block("t1"));
    assert!(s.allow_block("t1"));
    assert!(!s.allow_block("t1"), "third block in the same turn");
    assert!(s.allow_block("t2"), "a new turn starts at zero");
    for i in 0..20 {
        s.allow_block(&format!("x{i}"));
    }
    assert_eq!(s.blocks.len(), 8);
    assert!(s.allow_block("t1"), "t1 was forgotten");
}

#[test]
fn our_own_stop_block_goes_out_once_per_turn_inside_the_same_cap() {
    let mut s = Session::default();
    assert!(!s.allow_note_block(""), "no turn id, no block");
    assert!(s.allow_note_block("t1"));
    assert!(!s.allow_note_block("t1"), "once per turn");
    assert_eq!(
        s.blocks,
        [("t1".to_string(), 1)],
        "the refusal counts nothing"
    );
    // the engine blocked once before ours in t2: ours is its second, the last the cap lets through
    assert!(s.allow_block("t2"));
    assert!(s.allow_note_block("t2"));
    assert!(!s.allow_block("t2"));
    // the engine used up the cap of t3 first: ours is dropped, and the turn is not marked as told
    assert!(s.allow_block("t3") && s.allow_block("t3"));
    assert!(!s.allow_note_block("t3"));
    assert_ne!(s.noted, "t3");
    // an old state file without the field still loads
    let old: Session = serde_json::from_str(r#"{"seq":1,"told":false,"blocks":[]}"#).unwrap();
    assert_eq!(old.noted, "");
}

#[test]
fn session_state_is_saved_between_calls_and_ids_cannot_escape() {
    let dir = scratch("state-session");
    assert_eq!(with_session(&dir, "s/../1", |s| s.next_seq()), Some(1));
    assert_eq!(with_session(&dir, "s/../1", |s| s.next_seq()), Some(2));
    assert!(
        dir.join("session/s1.json").is_file(),
        "{:?}",
        fs::read_dir(&dir)
    );
    end_session(&dir, "s/../1");
    assert_eq!(with_session(&dir, "s/../1", |s| s.next_seq()), Some(1));
}

#[test]
fn parallel_hooks_get_distinct_numbers() {
    let dir = scratch("state-parallel");
    let seen: Vec<u64> = (0..8)
        .map(|_| {
            let d = dir.clone();
            std::thread::spawn(move || with_session(&d, "p", |s| s.next_seq()).unwrap_or(0))
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    let mut sorted = seen.clone();
    sorted.sort();
    assert_eq!(sorted, (1..=8).collect::<Vec<_>>(), "{seen:?}");
}

#[test]
fn a_folder_has_one_open_turn_and_only_its_session_closes_it() {
    let dir = scratch("state-open");
    assert_eq!(open_get(&dir, "/w/a"), None);
    open_set(&dir, "/w/a", "s1", "t1");
    open_set(&dir, "/w/b", "s2", "t2");
    assert_eq!(open_get(&dir, "/w/a"), Some(("s1".into(), "t1".into())));
    assert_eq!(open_get(&dir, "/w/b"), Some(("s2".into(), "t2".into())));
    open_clear(&dir, "/w/a", "s2");
    assert!(open_get(&dir, "/w/a").is_some(), "another session's end");
    open_set(&dir, "/w/a", "s3", "t3");
    assert_eq!(open_get(&dir, "/w/a"), Some(("s3".into(), "t3".into())));
    open_clear(&dir, "/w/a", "s3");
    assert_eq!(open_get(&dir, "/w/a"), None);
    assert!(open_get(&dir, "/w/b").is_some(), "the other folder");
}

#[test]
fn an_open_turn_from_a_crashed_muse_is_not_used_and_is_swept() {
    let dir = scratch("state-open-old");
    let old = Open {
        cwd: "/w".into(),
        sid: "s".into(),
        turn: "t".into(),
        at: crate::util::now_secs() - OPEN_MAX_S - 1,
    };
    crate::engine::write_private(
        &folder_file(&dir, "open", "/w"),
        &serde_json::to_vec(&old).unwrap(),
    )
    .unwrap();
    assert_eq!(open_get(&dir, "/w"), None);
    // sweep: 15 days untouched goes, in every folder; a fresh file stays
    let stale = UNIX_EPOCH + Duration::from_secs(crate::util::now_secs() - 15 * 86_400);
    let old_ones = [
        "db/old.json",
        "open/old.json",
        "session/old.json",
        "beats/old.jsonl",
        "transcripts/old.jsonl",
        "transcripts/old.state",
        "transcripts/old.lock",
    ];
    for f in old_ones.iter().chain(&["open/new.json", "beats/new.jsonl"]) {
        let p = dir.join(f);
        crate::engine::write_private(&p, b"{}").unwrap();
        if !f.contains("new.") {
            fs::File::open(&p).unwrap().set_modified(stale).unwrap();
        }
    }
    sweep(&dir);
    assert!(old_ones.iter().all(|f| !dir.join(f).exists()));
    assert!(dir.join("open/new.json").exists() && dir.join("beats/new.jsonl").exists());
}

#[test]
fn a_folder_is_marked_per_kind_and_the_mark_is_never_swept() {
    let dir = scratch("state-gate");
    assert!(!marked(&dir, "gate", "/w/a"));
    assert!(mark(&dir, "gate", "/w/a"));
    assert!(marked(&dir, "gate", "/w/a"));
    assert!(!marked(&dir, "gate", "/w/b"), "another folder");
    assert!(!marked(&dir, "app", "/w/a"), "another kind");
    // 15 days untouched: `sweep` takes the other kinds of file, not these
    let stale = UNIX_EPOCH + Duration::from_secs(crate::util::now_secs() - 15 * 86_400);
    assert!(mark(&dir, "app", "/w/a") && mark(&dir, "no-key", "/w/a"));
    for sub in ["gate", "app", "no-key"] {
        let f = folder_file(&dir, sub, "/w/a");
        fs::File::open(&f).unwrap().set_modified(stale).unwrap();
    }
    sweep(&dir);
    assert!(marked(&dir, "gate", "/w/a") && marked(&dir, "app", "/w/a"));
    assert!(marked(&dir, "no-key", "/w/a"));
}

#[test]
fn a_problem_line_is_one_line_whatever_the_event_argument_holds() {
    let dir = scratch("state-problem-event");
    problem(&dir, "Bad\x1b[31m\nEvent", "s1", "wrong\tthing\r\n");
    let log = fs::read_to_string(dir.join("problems.log")).unwrap();
    assert_eq!(log.lines().count(), 1, "{log:?}");
    assert!(!log.chars().any(|c| c.is_control() && c != '\n'), "{log:?}");
    assert!(log.contains("Bad [31m Event s1 wrong thing"), "{log:?}");
}

#[test]
fn a_key_is_new_once_and_the_log_stays_small() {
    let dir = scratch("state-keys");
    assert!(new_key(&dir, "Stop", "brand_new"));
    assert!(!new_key(&dir, "Stop", "brand_new"));
    assert!(new_key(&dir, "Stop", "other"));
    for i in 0..3000 {
        problem(&dir, "Stop", "s", &format!("line {i} {}", "x".repeat(40)));
    }
    let len = fs::metadata(dir.join("problems.log")).unwrap().len();
    assert!(len <= LOG_CAP + 200, "{len}");
    let text = fs::read_to_string(dir.join("problems.log")).unwrap();
    assert!(text.contains("line 2999"), "newest lines are kept");
}

//! The check-writer against the mock Meta API (mock/server.mjs): what a saved check and the engine's verdict
//! tell the user. No real model and no money. Owner: S3b-writer.

mod writer_common;

use serde_json::{Value, json};
use std::fs;
use writer_common::*;

#[test]
fn a_saved_check_says_which_part_of_the_words_it_cannot_prove() {
    let sb = Sandbox::new("gap");
    say_proven(&sb);
    let mut answer = good_answer();
    answer["reason"] = json!("it cannot restart the server to see the data survive");
    let mock = Mock::ok(&sb.dir, &answer);
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 0, "{ctx}");
    assert!(
        r.out.starts_with("proven: ")
            && r.out.ends_with(
                "Not covered by this check: it cannot restart the server to see the data survive\n"
            ),
        "{ctx}"
    );
    // nothing to add when the steps cover the words
    let sb = Sandbox::new("nogap");
    say_proven(&sb);
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert!(!r.out.contains("Not covered"), "{:?}", r.out);
}

/// S2 on F4b: the engine's "In shadow for now ..." tail (the fake engine's is short) is true only with the gate on, and
/// the gate is off in a new folder. The line the user reads follows what `torpor gate` says.
#[test]
fn a_proven_checks_line_promises_a_stop_only_when_the_gate_is_on() {
    let off = "The gate is off. Promises show failures but do not stop the AI. Turn it on: torpor gate on\n";
    let cases = [
        ("on", Some("The gate is on.\n"), true),
        ("none", None, true), // the fake engine's own default is "on"
        ("off", Some(off), false),
        (
            "advisory",
            Some("The gate is advisory: two disputes were upheld within 7 days.\n"),
            false,
        ),
        (
            "unknown",
            Some("Torpor has no save points for this folder yet.\n"),
            false,
        ),
    ];
    for (tag, gate, keeps_tail) in cases {
        let sb = Sandbox::new(&format!("gate-{tag}"));
        say_proven(&sb);
        if let Some(g) = gate {
            sb.engine_file("gate.txt", g);
        }
        let mock = Mock::ok(&sb.dir, &good_answer());
        let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
        let ctx = format!("{tag}: out={:?} err={:?}", r.out, r.err);
        assert_eq!(r.code, 0, "{ctx}");
        assert!(
            r.out
                .starts_with("proven: Promise #1 \"login works\" saved. Proof: passed 5 of 5"),
            "{ctx}"
        );
        assert_eq!(r.out.contains("In shadow for now"), keeps_tail, "{ctx}");
        assert_eq!(r.out.contains("blocks nothing yet"), keeps_tail, "{ctx}");
        let says_told =
            r.out.contains("it does not stop the AI") && r.out.contains("`torpor gate on`");
        assert_eq!(says_told, !keeps_tail && tag != "unknown", "{ctx}");
        assert_eq!(sb.result()["outcome"], "proven", "{ctx}");
    }
}

/// dogfood1 M5: a step on a record already in the user's data (a note id the check did not create) is left out and
/// named as not covered; when no step is left, nothing is saved.
#[test]
fn a_step_on_a_record_the_check_did_not_create_is_left_out_and_named() {
    let edit = json!({"method": "PUT", "path": "/api/notes/mvqm4ta2", "body_json": "{\"text\":\"x\"}",
        "expect_status": 200, "json_has": [], "json_types": [], "body_contains": "", "why": "login works, then edit"});
    let sb = Sandbox::new("record");
    say_proven(&sb);
    let mut answer = good_answer();
    answer["steps"].as_array_mut().unwrap().push(edit.clone());
    let mock = Mock::ok(&sb.dir, &answer);
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 0, "{ctx}");
    assert!(
        r.out
            .contains("Not covered by this check: step 3 (PUT /api/notes/mvqm4ta2) left out"),
        "{ctx}"
    );
    let adds = sb.adds();
    assert!(
        adds.len() == 1 && !adds[0]["steps"].to_string().contains("mvqm4ta2"),
        "{adds:?}"
    );
    let sb = Sandbox::new("records");
    answer["steps"] = json!([edit]);
    let mock = Mock::ok(&sb.dir, &answer);
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 3, "{ctx}");
    assert!(
        r.out
            .starts_with("advisory: no check saved: step 1 (PUT /api/notes/mvqm4ta2)"),
        "{ctx}"
    );
    assert!(sb.adds().is_empty(), "{ctx}");
}

/// L3 N3: the engine's client drops a cookie the server clears, so a refusal after a logout step comes whether or not
/// the server ended the session. Such a check keeps its steps and is told to say so; one without a logout is unchanged.
#[test]
fn a_refusal_after_a_logout_step_is_named_as_not_covered() {
    let sb = Sandbox::new("logout");
    say_proven(&sb);
    let mut answer = good_answer();
    let out = json!({"method": "POST", "path": "/api/logout", "body_json": "", "expect_status": 200, "json_has": [],
        "json_types": [], "body_contains": "", "why": "logout works"});
    let refused = json!({"method": "GET", "path": "/api/me", "body_json": "", "expect_status": 401, "json_has": [],
        "json_types": [], "body_contains": "", "why": "logout works: the account page is closed"});
    answer["steps"]
        .as_array_mut()
        .unwrap()
        .extend([out, refused]);
    let mock = Mock::ok(&sb.dir, &answer);
    let r = sb.suggest(Some(&mock.base()), "logout works now", &[]);
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 0, "{ctx}");
    assert!(
        r.out.contains(
            "Not covered by this check: the check cannot tell whether the server ended the session"
        ),
        "{ctx}"
    );
    let adds = sb.adds();
    assert_eq!(
        adds[0]["steps"].as_array().map(Vec::len),
        Some(4),
        "{adds:?}"
    );
    // control: no logout step
    let sb = Sandbox::new("no-logout");
    say_proven(&sb);
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert!(r.code == 0 && !r.out.contains("session"), "{:?}", r.out);
    assert_eq!(sb.adds()[0]["why"], good_answer()["why"]);
}

/// L3 N4: what a check does not cover goes into its why (the Why line of `torpor promise show`; the engine takes at
/// most 200 characters, and the model's own words are never cut) and into `gaps.json`, where the hook finds it.
#[test]
fn the_not_covered_text_goes_into_the_why_and_to_the_hook() {
    let gap = "it cannot restart the server to see the data survive";
    let sb = Sandbox::new("gap-why");
    say_proven(&sb);
    let mut answer = good_answer();
    answer["reason"] = json!(gap);
    let mock = Mock::ok(&sb.dir, &answer);
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 0, "out={:?} err={:?}", r.out, r.err);
    assert_eq!(
        sb.adds()[0]["why"],
        format!("login works for alice Not covered: {gap}")
    );
    let ws: Vec<_> = fs::read_dir(sb.state().join("ws"))
        .unwrap()
        .flatten()
        .collect();
    let gaps: Value =
        serde_json::from_str(&fs::read_to_string(ws[0].path().join("gaps.json")).unwrap()).unwrap();
    assert_eq!(
        gaps,
        json!([{"promise": 1, "session": null, "gap": gap}]),
        "{gaps}"
    );
    // no room left in the why: it stays as the model wrote it
    let sb = Sandbox::new("gap-full-why");
    say_proven(&sb);
    answer["why"] = json!(format!("login works {}", "w".repeat(183)));
    let mock = Mock::ok(&sb.dir, &answer);
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 0, "out={:?} err={:?}", r.out, r.err);
    assert_eq!(sb.adds()[0]["why"], answer["why"]);
}

#[test]
fn a_not_a_claim_verdict_is_advisory_and_makes_no_check() {
    let sb = Sandbox::new("noclaim");
    let answer = json!({"verdict": "not_a_claim", "reason": "It is a question, not a statement.", "name": "", "why": "",
        "start_script": "", "ready_path": "", "node_env": "", "steps": []});
    let mock = Mock::ok(&sb.dir, &answer);
    let r = sb.suggest(Some(&mock.base()), "can you add a login?", &[]);
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert!(
        r.out.starts_with("advisory: ") && r.out.contains("question"),
        "{}",
        r.out
    );
    assert_eq!(mock.requests().len(), 1, "no retry for a clear answer");
    assert!(sb.adds().is_empty());
    assert_eq!(
        sb.result()["code"],
        "",
        "a normal outcome is not news for the user"
    );

    let sb = Sandbox::new("nocheck");
    let answer = json!({"verdict": "cannot_check", "reason": "It needs a browser to click the button.", "name": "", "why": "",
        "start_script": "", "ready_path": "", "node_env": "", "steps": []});
    let mock = Mock::ok(&sb.dir, &answer);
    let r = sb.suggest(Some(&mock.base()), "the signup button works", &[]);
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert!(
        r.out.contains("cannot check") && r.out.contains("browser"),
        "{}",
        r.out
    );
    assert_eq!(sb.result()["code"], "");
}

#[test]
fn what_the_engine_hands_back_is_told_apart() {
    // saved but advisory (the engine's own reason is shown)
    let sb = Sandbox::new("adv");
    sb.engine_says(0, "Promise #4 \"login works\" saved. Proof: passed 5 of 5 on X and failed 2 of 2 on Y. Advisory for now: Torpor shows failures but does not stop the AI, because praise written only in Hebrew stays advisory.\n", "");
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 0, "out={:?} err={:?}", r.out, r.err);
    assert!(
        r.out.starts_with("advisory: ") && r.out.contains("Hebrew"),
        "{}",
        r.out
    );
    assert_eq!(sb.result()["saved"], true);
    assert_eq!(sb.result()["promise"], 4);

    // not saved, because the check fails on the good version: one retry with the engine's words, then advisory
    let sb = Sandbox::new("notsaved");
    sb.engine_says(1, "", "Not saved: it fails on the version from the start of turn 2 in 5 of 5 runs (POST /api/login answered 200, expected 201).\n");
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert_eq!(mock.requests().len(), 2);
    assert!(mock.input(1).contains("expected 201"));
    assert!(
        r.out.starts_with("advisory: ") && r.out.contains("expected 201"),
        "{}",
        r.out
    );

    // not saved for a reason the check cannot fix: no retry
    let sb = Sandbox::new("notsaved2");
    sb.engine_says(1, "", "Not saved: there is no version to call good.\n");
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert_eq!(mock.requests().len(), 1);
    assert!(r.out.contains("no version to call good"), "{}", r.out);
}

#[test]
fn the_check_that_reaches_the_engine_is_the_models_check_made_safe() {
    let sb = Sandbox::new("conv");
    say_proven(&sb);
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 0, "out={:?} err={:?}", r.out, r.err);
    let adds = sb.adds();
    assert_eq!(adds.len(), 1);
    let check: &Value = &adds[0];
    assert_eq!(check["start"]["cmd"], "npm start --silent");
    assert_eq!(check["start"]["ready"], "/");
    assert_eq!(check["steps"][0]["http"]["method"], "POST");
    assert_eq!(check["steps"][0]["http"]["json"]["user"], "alice");
    assert_eq!(check["steps"][1]["expect"]["status"], 200);
    assert!(
        check.get("env").is_none(),
        "no env unless the code needs one"
    );
}

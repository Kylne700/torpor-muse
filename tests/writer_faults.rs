//! The check-writer against the mock Meta API: faults, retries, timeouts, the day cap and the ledger.

mod writer_common;

use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use writer_common::*;

/// M2 evidence: the ledger row of each call names the files whose names and starts were sent, never what is in them.
#[test]
fn the_ledger_names_the_files_that_were_sent() {
    let sb = Sandbox::new("sent");
    say_proven(&sb);
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 0, "out={:?} err={:?}", r.out, r.err);
    let rows = sb.ledger();
    let note = rows[0]["note"].as_str().unwrap_or("");
    assert!(
        note.starts_with("attempt 1, ") && note.ends_with("s; sent: package.json, server.js"),
        "{note}"
    );
}

#[test]
fn every_fault_ends_rejected_or_advisory_with_the_reason() {
    // (fixture, outcome, requests made, a piece of the reason, the code the hook tells the user about: "" = none)
    let table = [
        ("truncated", "rejected", 2, "ran out of room", "rejected"),
        ("refusal", "rejected", 2, "refused", "rejected"),
        ("empty", "rejected", 2, "no answer text", "rejected"),
        ("schema-400", "advisory", 1, "rejected the request", "api"),
        ("billing-402", "advisory", 1, "billing", "key"),
        ("server-500", "advisory", 2, "server error", "api"),
    ];
    for (fixture, outcome, calls, reason, code) in table {
        let sb = Sandbox::new("fault");
        let mock = Mock::start(&sb.dir, fixture, 0, &good_answer().to_string());
        let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
        let ctx = format!("{fixture}: out={:?} err={:?}", r.out, r.err);
        assert_eq!(r.code, 3, "{ctx}");
        assert!(r.out.starts_with(&format!("{outcome}: ")), "{ctx}");
        assert!(r.out.contains(reason), "{ctx}");
        assert_eq!(mock.requests().len(), calls, "{ctx}");
        assert!(sb.adds().is_empty(), "nothing reached the engine: {ctx}");
        let res = sb.result();
        assert_eq!(res["outcome"], outcome, "{ctx}");
        assert_eq!(res["saved"], false, "{ctx}");
        assert_eq!(res["code"], code, "{ctx}");
        assert_eq!(
            sb.ledger().len(),
            calls,
            "one ledger row per response: {ctx}"
        );
    }
}

#[test]
fn output_that_does_not_fit_the_schema_is_rejected_never_a_half_check() {
    let good = good_answer();
    let mut extra = good.clone();
    extra["note"] = json!("hi");
    let mut wrong = good.clone();
    wrong["steps"][0]["expect_status"] = json!("200");
    let mut missing = good.clone();
    missing.as_object_mut().unwrap().remove("node_env");
    let mut verdict = good.clone();
    verdict["verdict"] = json!("maybe");
    let cases = [
        ("prose", "Sure! Here is your check.".to_string()),
        ("fenced", format!("```json\n{good}\n```")),
        ("extra key", extra.to_string()),
        ("wrong type", wrong.to_string()),
        ("missing key", missing.to_string()),
        ("unknown verdict", verdict.to_string()),
        ("cut off", good.to_string()[..60].to_string()),
        ("empty object", "{}".to_string()),
        ("array", "[]".to_string()),
    ];
    for (what, text) in cases {
        let sb = Sandbox::new("shape");
        let mock = Mock::start(&sb.dir, "ok", 0, &text);
        let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
        let ctx = format!("{what}: out={:?} err={:?}", r.out, r.err);
        assert_eq!(r.code, 3, "{ctx}");
        assert!(r.out.starts_with("rejected: "), "{ctx}");
        assert_eq!(sb.result()["code"], "rejected", "{ctx}");
        assert_eq!(mock.requests().len(), 2, "one retry, then stop: {ctx}");
        assert!(sb.adds().is_empty(), "nothing reached the engine: {ctx}");
        let second = mock.input(1);
        assert!(
            second.contains("previous_answer") && second.contains("runner_message"),
            "the retry quotes the answer and the complaint: {ctx}"
        );
    }
}

#[test]
fn a_check_the_engine_turns_down_gets_one_retry_with_the_engines_words() {
    let sb = Sandbox::new("lint");
    sb.engine_says(
        3,
        "",
        "Rejected: step 1 expects a long number, which changes on every run\n",
    );
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 3, "{ctx}");
    assert!(
        r.out.starts_with("advisory: ") && r.out.contains("long number"),
        "{ctx}"
    );
    assert_eq!(mock.requests().len(), 2, "{ctx}");
    assert_eq!(sb.adds().len(), 2, "the engine saw both checks: {ctx}");
    let second = mock.input(1);
    let at = second.find("<<<DATA runner_message").expect(&ctx);
    assert!(second[at..].contains("expects a long number"), "{ctx}");
    assert_eq!(sb.ledger().len(), 2, "{ctx}");
}

#[test]
fn a_rate_limit_waits_for_retry_after_and_then_succeeds() {
    let sb = Sandbox::new("429");
    say_proven(&sb);
    let mock = Mock::start(&sb.dir, "rate-429,ok", 0, &good_answer().to_string());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 0, "{ctx}");
    assert!(r.out.starts_with("proven: "), "{ctx}");
    assert!(
        r.secs >= 2.0,
        "the mock says Retry-After: 2, took {}",
        r.secs
    );
    assert_eq!(mock.requests().len(), 2, "{ctx}");
    let rows = sb.ledger();
    assert_eq!(rows.len(), 2, "a 429 is a response and gets a row: {ctx}");
    assert_eq!(rows[0]["usd"], 0.0, "{rows:?}");
    assert!(rows[1]["output"].as_u64().unwrap() > 0, "{rows:?}");
    assert_eq!(sb.result()["outcome"], "proven");
}

#[test]
fn a_second_rate_limit_ends_advisory() {
    let sb = Sandbox::new("429b");
    let mock = Mock::start(&sb.dir, "rate-429", 0, &good_answer().to_string());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 3, "{}", r.out);
    assert!(
        r.out.starts_with("advisory: ") && r.out.contains("rate limited"),
        "{}",
        r.out
    );
    assert_eq!(mock.requests().len(), 2);
}

#[test]
fn a_dropped_connection_and_a_hang_end_within_the_timeout() {
    let sb = Sandbox::new("drop");
    let mock = Mock::start(&sb.dir, "drop-mid-body", 0, &good_answer().to_string());
    let r = sb.suggest(
        Some(&mock.base()),
        WORDS,
        &[("TORPOR_MUSE_HTTP_TIMEOUT_S", "5")],
    );
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 3, "{ctx}");
    assert!(
        r.out.starts_with("advisory: ") && r.out.contains("whole answer"),
        "{ctx}"
    );
    assert_eq!(mock.requests().len(), 2, "one retry: {ctx}");
    assert!(r.secs < 10.0, "{}", r.secs);

    let sb = Sandbox::new("hang");
    let mock = Mock::start(&sb.dir, "hang", 0, &good_answer().to_string());
    let r = sb.suggest(
        Some(&mock.base()),
        WORDS,
        &[
            ("TORPOR_MUSE_HTTP_TIMEOUT_S", "1"),
            ("TORPOR_MUSE_HARD_TIMEOUT_S", "60"),
        ],
    );
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 3, "{ctx}");
    assert!(
        r.out.starts_with("advisory: ") && r.out.contains("in time"),
        "{ctx}"
    );
    assert!(r.secs < 6.0, "the 1 s call limit holds: {}", r.secs);
    let rows = sb.ledger();
    assert_eq!(
        rows.len(),
        1,
        "a call that timed out is still recorded: {ctx}"
    );
    assert_eq!(rows[0]["est"], true, "{rows:?}");

    // The whole-run limit is shorter than the call limit: the call is cut at the run's end.
    let sb = Sandbox::new("hard");
    let mock = Mock::start(&sb.dir, "hang", 0, &good_answer().to_string());
    let r = sb.suggest(
        Some(&mock.base()),
        WORDS,
        &[
            ("TORPOR_MUSE_HTTP_TIMEOUT_S", "30"),
            ("TORPOR_MUSE_HARD_TIMEOUT_S", "3"),
        ],
    );
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert!(r.secs < 9.0, "{}", r.secs);
    assert!(sb.result()["outcome"] == "advisory");
}

#[test]
fn a_stuck_engine_is_stopped_by_the_run_time_limit() {
    let sb = Sandbox::new("watchdog");
    sb.engine_file("version.sleep", "40");
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(
        Some(&mock.base()),
        WORDS,
        &[("TORPOR_MUSE_HARD_TIMEOUT_S", "3")],
    );
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert!(r.out.contains("took too long"), "{}", r.out);
    assert!(
        r.secs < 12.0,
        "stopped near 3 s plus the grace, not 40 s: {}",
        r.secs
    );
    assert_eq!(
        mock.requests().len(),
        0,
        "nothing was sent before the engine answered"
    );
    let res = sb.result();
    assert_eq!(res["outcome"], "advisory");
    assert_eq!(res["code"], "slow", "the hook tells the user about it");
}

#[test]
fn a_contributor_model_is_refused_without_the_synthetic_marker() {
    let sb = Sandbox::new("contrib");
    fs::remove_file(sb.ws.join(".synthetic")).unwrap();
    let mock = Mock::ok(&sb.dir, &good_answer());
    let set = [("TORPOR_MUSE_MODEL", "muse-spark-1.3-contributor")];
    let r = sb.suggest(Some(&mock.base()), WORDS, &set);
    assert_eq!(r.code, 1, "out={:?} err={:?}", r.out, r.err);
    assert!(r.err.contains(".synthetic"), "{}", r.err);
    assert_eq!(mock.requests().len(), 0);

    // with the marker the same id is used, and the cheaper price lands in the ledger
    let sb = Sandbox::new("contrib2");
    say_proven(&sb);
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &set);
    assert_eq!(r.code, 0, "out={:?} err={:?}", r.out, r.err);
    assert_eq!(
        mock.requests()[0]["body"]["model"],
        "muse-spark-1.3-contributor"
    );
    assert!(
        sb.ledger()[0]["usd"].as_f64().unwrap() < 0.0005,
        "{:?}",
        sb.ledger()
    );
}

#[test]
fn the_day_cap_stops_the_run_before_the_call() {
    let sb = Sandbox::new("cap");
    let row = json!({"at": format!("{}T08:00:00Z", utc_day()), "day": utc_day(), "model": "muse-spark-1.3",
        "status": "completed", "input": 1, "cached": 0, "output": 1, "usd": 1.99, "est": false, "note": "earlier"});
    fs::write(sb.state().join("ledger.jsonl"), format!("{row}\n")).unwrap();
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert!(
        r.out.starts_with("advisory: ") && r.out.contains("cap"),
        "{}",
        r.out
    );
    assert!(r.out.contains("Nothing was sent"), "{}", r.out);
    assert_eq!(
        sb.result()["code"],
        "cap",
        "the hook tells the user about it"
    );
    assert_eq!(mock.requests().len(), 0, "the call was never made");
    assert_eq!(sb.ledger().len(), 1, "no row for a call that was not made");

    // yesterday's spending does not count today
    let sb = Sandbox::new("cap2");
    say_proven(&sb);
    let old = json!({"at": "2020-01-01T08:00:00Z", "day": "2020-01-01", "model": "muse-spark-1.3",
        "status": "completed", "input": 1, "cached": 0, "output": 1, "usd": 1.99, "est": false, "note": "old"});
    fs::write(sb.state().join("ledger.jsonl"), format!("{old}\n")).unwrap();
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 0, "out={:?} err={:?}", r.out, r.err);
    assert_eq!(sb.ledger().len(), 2);

    // a lower cap from the environment
    let sb = Sandbox::new("cap3");
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(
        Some(&mock.base()),
        WORDS,
        &[("TORPOR_MUSE_DAY_CAP_USD", "0.001")],
    );
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert_eq!(mock.requests().len(), 0);

    // a cap of zero in the config file (the README names it): every run stops before the call
    let sb = Sandbox::new("cap4");
    let cfg_dir = sb.home().join(".config/torpor-muse");
    fs::create_dir_all(&cfg_dir).unwrap();
    let cfg = cfg_dir.join("config");
    fs::write(&cfg, "day_cap_usd = 0\n").unwrap();
    fs::set_permissions(&cfg, fs::Permissions::from_mode(0o600)).unwrap();
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert!(r.out.contains("Nothing was sent"), "{}", r.out);
    assert_eq!(sb.result()["code"], "cap");
    assert_eq!(mock.requests().len(), 0);
}

#[test]
fn the_ledger_has_a_row_for_every_response_including_failed_ones() {
    let sb = Sandbox::new("rows");
    say_proven(&sb);
    let mock = Mock::start(&sb.dir, "server-500,ok", 0, &good_answer().to_string());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 0, "out={:?} err={:?}", r.out, r.err);
    let rows = sb.ledger();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(
        rows[0]["est"], true,
        "a failed call without usage is charged its worst case: {rows:?}"
    );
    assert!(rows[0]["usd"].as_f64().unwrap() > 0.0);
    assert_eq!(rows[1]["est"], false);
    // the first question about the words and the two tries: three requests, three rows
    assert_eq!(sb.result()["calls"], 3);
    assert_eq!(sb.ledger_all().len(), 3);
}

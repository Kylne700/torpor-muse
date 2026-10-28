//! The one sentence for the model: its words and size, how it joins the Stop block, which message is read.

use crate::hook::done::claim::{Claim, Topic};
use crate::hook::done::commands::Seen::{No, Ran};
use crate::hook::done::verdict::sentence;
use crate::hook::done::{Log, join, said};
use crate::hook::stop;
use serde_json::json;

fn claims(topics: &[Topic]) -> Vec<Claim> {
    topics
        .iter()
        .map(|&topic| Claim {
            topic,
            shown: String::new(),
        })
        .collect()
}

/// A log of a session that started here, with the one run of `topic` that `how` describes.
fn log(topic: Topic, how: &str, cmd: &str, code: i64) -> Log {
    let mut l = Log::default();
    l.start("startup");
    l.see("t1");
    let code = if how == "edited" { 0 } else { code };
    let (tests, build) = if topic == Topic::Test {
        (Ran(code), No)
    } else {
        (No, Ran(code))
    };
    match how {
        "none" => {}
        "failed" => l.ran(tests, build, cmd),
        _ => {
            l.ran(tests, build, cmd);
            l.edited();
        }
    }
    l
}

#[test]
fn the_sentence_says_the_fact_and_the_next_step() {
    let none = sentence(&log(Topic::Test, "none", "", 0), &claims(&[Topic::Test])).unwrap();
    assert_eq!(
        none,
        "You wrote that the tests pass, but no test run was seen in this session; run the tests now and report the real \
         result, or tell the user they were not run."
    );
    let failed = sentence(
        &log(Topic::Test, "failed", "npm test", 1),
        &claims(&[Topic::Test]),
    )
    .unwrap();
    assert_eq!(
        failed,
        "You wrote that the tests pass, but the last test run (`npm test`, this turn) exited with 1; run the tests now \
         and report the real result, or tell the user they were not run."
    );
    let edited = sentence(
        &log(Topic::Test, "edited", "npm test", 0),
        &claims(&[Topic::Test]),
    )
    .unwrap();
    assert_eq!(
        edited,
        "You wrote that the tests pass, but code was changed after the last passing test run (`npm test`, this turn); run \
         the tests again and report the real result, or tell the user they were not re-run."
    );
    let build = sentence(
        &log(Topic::Build, "failed", "cargo build", 101),
        &claims(&[Topic::Build]),
    )
    .unwrap();
    assert_eq!(
        build,
        "You wrote that the build succeeds, but the last build (`cargo build`, this turn) exited with 101; run the build \
         now and report the real result, or tell the user it was not run."
    );
    let rebuilt = sentence(
        &log(Topic::Build, "edited", "cargo build", 0),
        &claims(&[Topic::Build]),
    )
    .unwrap();
    assert!(
        rebuilt.contains(
            "after the last passing build (`cargo build`, this turn); run the build again"
        ),
        "{rebuilt}"
    );
    assert!(
        rebuilt.ends_with("tell the user it was not re-run."),
        "{rebuilt}"
    );
}

#[test]
fn the_sentence_says_when_the_run_was() {
    for (now, want) in [
        (1, "`pytest`, this turn"),
        (2, "`pytest`, the previous turn"),
        (5, "`pytest`, 4 turns ago"),
    ] {
        let mut l = log(Topic::Test, "failed", "pytest", 1);
        l.turn = now;
        assert!(
            sentence(&l, &claims(&[Topic::Test]))
                .unwrap()
                .contains(want),
            "{now}"
        );
    }
    let mut unknown = log(Topic::Test, "failed", "pytest", 1);
    unknown.turn = 0;
    let s = sentence(&unknown, &claims(&[Topic::Test])).unwrap();
    assert!(s.contains("(`pytest`) exited with 1"), "{s}");
}

#[test]
fn the_first_false_claim_is_named_tests_before_build_and_a_true_one_is_not() {
    let mut l = Log::default();
    l.start("startup");
    l.see("t1");
    let both = claims(&[Topic::Build, Topic::Test]);
    assert!(sentence(&l, &both).unwrap().contains("the tests pass"));
    l.ran(Ran(0), No, "pytest");
    assert!(
        sentence(&l, &both).unwrap().contains("the build succeeds"),
        "the tests are true, the build is not"
    );
    l.ran(No, Ran(0), "make");
    assert_eq!(sentence(&l, &both), None);
    assert_eq!(sentence(&l, &[]), None);
}

#[test]
fn the_sentence_is_plain_and_short_even_in_the_worst_case() {
    let cmd = "x".repeat(80);
    for topic in [Topic::Test, Topic::Build] {
        for how in ["none", "failed", "edited"] {
            let mut l = log(topic, how, &cmd, 255);
            l.turn = u32::MAX;
            let s = sentence(&l, &claims(&[topic])).unwrap();
            assert!(
                s.chars().count() <= 300,
                "{} characters: {s}",
                s.chars().count()
            );
            let plain: String = s.split('`').step_by(2).collect();
            assert_eq!(
                plain.matches(". ").count()
                    + plain.matches("? ").count()
                    + plain.matches("! ").count(),
                0,
                "{s}"
            );
            assert!(
                plain.ends_with('.') && plain.matches('.').count() == 1,
                "{s}"
            );
            assert!(!s.contains('\n') && s.starts_with("You wrote that "), "{s}");
            if how != "none" {
                assert!(
                    s.contains("4294967294 turns ago") || s.contains("4294967295 turns ago"),
                    "{s}"
                );
            }
        }
    }
}

#[test]
fn the_sentence_joins_the_one_block_of_the_stop() {
    let ours =
        "You wrote that the tests pass, but no test run was seen in this session.".to_string();
    let engine = format!("{}\n- a saved check broke", stop::HEAD);
    let both = join(Some(engine.clone()), Some(ours.clone())).unwrap();
    assert_eq!(both, format!("{engine}\n- {ours}"));
    assert_eq!(both.matches(stop::HEAD).count(), 1, "one header");
    let alone = join(None, Some(ours.clone())).unwrap();
    assert_eq!(alone, format!("{}\n- {ours}", stop::HEAD));
    assert_eq!(join(Some(engine.clone()), None), Some(engine));
    assert_eq!(join(None, None), None);
}

fn stop_of(message: &str, active: serde_json::Value) -> serde_json::Value {
    json!({"hook_event_name": "Stop", "last_assistant_message": message, "stop_hook_active": active})
}

#[test]
fn the_message_is_read_unless_the_stop_follows_a_block() {
    assert_eq!(
        said(&stop_of("The tests pass.", json!(false))),
        "The tests pass."
    );
    assert_eq!(
        said(&stop_of("The tests pass.", json!(null))),
        "The tests pass."
    );
    assert_eq!(
        said(&json!({"last_assistant_message": "The tests pass."})),
        "The tests pass."
    );
    assert_eq!(
        said(&stop_of("The tests pass.", json!(true))),
        "",
        "the Stop after our block"
    );
    assert_eq!(said(&json!({})), "");
    assert_eq!(said(&json!({"last_assistant_message": 7})), "");
}

#[test]
fn only_the_end_of_a_very_long_message_is_read() {
    let long = format!("{}END", "\u{e9}".repeat(30_000));
    let got = said(&stop_of(&long, json!(false)));
    assert_eq!(got.chars().count(), 20_000);
    assert!(got.ends_with("END"));
    let wide = "\u{1f600}".repeat(20_001);
    assert_eq!(said(&stop_of(&wide, json!(false))).chars().count(), 20_000);
    let exact = "a".repeat(20_000);
    assert_eq!(said(&stop_of(&exact, json!(false))), exact);
}

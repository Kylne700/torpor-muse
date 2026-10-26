//! The check-writer with the real Torpor engine (TORPOR_BIN) behind a mock model: the engine lints the check,
//! qualifies it on real saved versions of a synthetic app, and saves it. Skipped, with a message, without an
//! engine binary. Owner: S3b-writer.
mod writer_common;

use serde_json::json;
use writer_common::*;

const PRAISE: &str = "login works now, alice can sign in and then see her account";

fn with_engine(tag: &str) -> Option<Sandbox> {
    let mut sb = Sandbox::new(tag);
    sb.engine = Some(real_engine()?);
    Some(sb)
}

/// The run the Muse hook starts after the praise prompt, with the engine's hooks already fed.
fn hook_run(sb: &Sandbox, base: &str) -> Ran {
    sb.suggest(
        Some(base),
        "-",
        &[
            ("TORPOR_MUSE_FROM_HOOK", "s1"),
            ("TORPOR_MUSE_WORDS", PRAISE),
        ],
    )
}

#[test]
fn a_praise_becomes_a_proven_check_that_catches_a_break_and_ignores_a_harmless_edit() {
    let Some(sb) = with_engine("e2e") else { return };
    sb.engine_history(PRAISE);
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = hook_run(&sb, &mock.base());
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 0, "{ctx}");
    assert!(
        r.out.starts_with("proven: ") && r.out.contains("In shadow for now"),
        "{ctx}"
    );
    assert_eq!(sb.result()["outcome"], "proven", "{ctx}");

    let (_, list, _) = sb.torpor(&["promise", "list"], None);
    assert!(
        list.contains("login works") && list.contains("your words, turn 2"),
        "{list}"
    );

    // it passes on the app as it is
    let (_, out, _) = sb.torpor(&["check", "1"], None);
    assert!(out.contains("PASS") && !out.contains("FAIL"), "{out}");

    // a break is caught, a harmless edit is not flagged
    let file = sb.ws.join("server.js");
    let app = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, app.replace("!== body.password", "=== body.password")).unwrap();
    let (_, out, _) = sb.torpor(&["check", "1"], None);
    assert!(out.contains("FAIL"), "the break was not caught: {out}");
    std::fs::write(
        &file,
        app.replace("Small notes for small teams.", "Notes, kept small."),
    )
    .unwrap();
    let (_, out, _) = sb.torpor(&["check", "1"], None);
    assert!(out.contains("PASS") && !out.contains("FAIL"), "{out}");
    std::fs::write(&file, &app).unwrap();

    // the same praise again does not call the model a second time
    let again = hook_run(&sb, &mock.base());
    assert!(again.out.starts_with("skipped: "), "{}", again.out);
    assert_eq!(mock.requests().len(), 1);
}

#[test]
fn the_engines_own_lint_words_are_quoted_back_once_then_the_check_stays_advisory() {
    let Some(sb) = with_engine("lint") else {
        return;
    };
    sb.engine_history(PRAISE);
    let mut bad = good_answer();
    bad["steps"][0]["path"] = json!("http://evil.example/api/login");
    let mock = Mock::ok(&sb.dir, &bad);
    let r = hook_run(&sb, &mock.base());
    let ctx = format!("out={:?} err={:?}", r.out, r.err);
    assert_eq!(r.code, 3, "{ctx}");
    assert!(r.out.starts_with("advisory: "), "{ctx}");
    assert_eq!(mock.requests().len(), 2, "{ctx}");
    let second = mock.input(1);
    let at = second.find("<<<DATA runner_message").expect(&ctx);
    assert!(
        second[at..].contains("Rejected"),
        "the engine's words: {second}"
    );
    let (_, list, _) = sb.torpor(&["promise", "list"], None);
    assert!(!list.contains("login works"), "nothing was saved: {list}");
}

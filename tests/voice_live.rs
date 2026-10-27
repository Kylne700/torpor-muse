//! One live run of `torpor-muse say` on a synthetic workspace: a spoken clip goes to the real Meta Voice Transcribe
//! API, the words it hears go to the real writer (Muse Spark) and the real engine proves the check. Ignored by
//! default, because it costs money. Run by hand, with the key in the environment of this one command:
//!   META_API_KEY=... LIVE_STATE=<dir> VOICE_CLIP=<clip.wav> cargo test --test voice_live -- --ignored --nocapture
//! `VOICE_DRY=1` runs the same steps against the mock API instead (no key, no money, a silent clip is made).
mod writer_common;

use std::fs;
use std::path::PathBuf;
use writer_common::*;

const PRAISE: &str = "login works now for alice and bob";

#[test]
#[ignore = "costs money: a real transcription and a real writer call"]
fn live_say_one_spoken_claim() {
    let dry = std::env::var_os("VOICE_DRY").is_some();
    let get = |n: &str| std::env::var(n).unwrap_or_else(|_| panic!("set {n}"));
    let mut sb = Sandbox::new("voice-live");
    // The clip says "alice and bob" and the fixture app only has alice: bob is added to this copy, not to the fixture.
    let file = sb.ws.join("server.js");
    let app = fs::read_to_string(&file).unwrap();
    let alice = "['alice', 'wonderland-demo-pass']";
    assert!(app.contains(alice), "the fixture app changed");
    let both = format!("{alice}, ['bob', 'builder-demo-pass']");
    fs::write(&file, app.replace(alice, &both)).unwrap();
    sb.git(&["commit", "-aqm", "bob"]);
    sb.engine = Some(real_engine().expect("the live run needs the engine (TORPOR_BIN)"));
    if !dry {
        sb.state_override = Some(PathBuf::from(get("LIVE_STATE")));
    }
    sb.engine_history(PRAISE);

    let mock = dry.then(|| Mock::ok(&sb.dir, &good_answer()));
    let (key, clip) = if dry {
        (FAKE_KEY.to_string(), sb.clip("claim.wav", 2580))
    } else {
        (get("META_API_KEY"), PathBuf::from(get("VOICE_CLIP")))
    };
    let base = mock.as_ref().map(Mock::base);
    let r = sb.say(
        base.as_deref(),
        &[clip.to_str().unwrap()],
        &[("META_API_KEY", &key), ("TORPOR_MUSE_DAY_CAP_USD", "0.9")],
    );
    println!(
        "LIVE say exit={} {:.1}s\n{}{}",
        r.code, r.secs, r.out, r.err
    );
    for row in sb.ledger() {
        println!(
            "LIVE ledger {} {} {} usd={} est={} in={} out={} | {}",
            row["at"].as_str().unwrap_or(""),
            row["model"].as_str().unwrap_or(""),
            row["status"].as_str().unwrap_or(""),
            row["usd"],
            row["est"],
            row["input"],
            row["output"],
            row["note"].as_str().unwrap_or("")
        );
    }
    let mut lines = r.out.lines();
    assert!(
        lines.next().is_some_and(|l| l.starts_with("heard: \"")),
        "{}",
        r.out
    );
    assert!(
        lines.next().is_some_and(|l| l.starts_with("proven: ")),
        "{}{}",
        r.out,
        r.err
    );
    assert_eq!(r.code, 0);
}

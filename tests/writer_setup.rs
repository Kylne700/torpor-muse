//! The check-writer against the mock Meta API: the address, the key file, the start conditions, the hook's run.

mod writer_common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use writer_common::*;

#[test]
fn the_base_url_must_be_https_or_loopback_and_a_missing_key_is_said_plainly() {
    let sb = Sandbox::new("base");
    let r = sb.suggest(Some("http://example.com"), WORDS, &[]);
    assert_eq!(r.code, 1, "out={:?} err={:?}", r.out, r.err);
    assert!(r.err.contains("https"), "{}", r.err);

    let sb = Sandbox::new("nokey");
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[("-META_API_KEY", "")]);
    assert_eq!(r.code, 1, "out={:?} err={:?}", r.out, r.err);
    assert!(
        r.err.contains("META_API_KEY") && r.err.contains("key_file"),
        "{}",
        r.err
    );
    assert_eq!(mock.requests().len(), 0);
}

#[test]
fn the_key_can_come_from_a_private_key_file_named_in_the_config() {
    let sb = Sandbox::new("cfg");
    say_proven(&sb);
    let key_file = sb.dir.join("fake-key.txt");
    fs::write(&key_file, format!("{FAKE_KEY}\n")).unwrap();
    fs::set_permissions(&key_file, fs::Permissions::from_mode(0o600)).unwrap();
    let cfg_dir = sb.home().join(".config/torpor-muse");
    fs::create_dir_all(&cfg_dir).unwrap();
    let cfg = cfg_dir.join("config");
    fs::write(&cfg, format!("key_file = {}\n", key_file.display())).unwrap();
    fs::set_permissions(&cfg, fs::Permissions::from_mode(0o600)).unwrap();
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[("-META_API_KEY", "")]);
    assert_eq!(r.code, 0, "out={:?} err={:?}", r.out, r.err);
    let auth = mock.requests()[0]["headers"]["authorization"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        auth.contains(&format!("len={}", "Bearer ".len() + FAKE_KEY.len())),
        "{auth}"
    );

    // a config or a key file that others can read is refused, with zero calls
    for (what, target) in [("config", &cfg), ("key file", &key_file)] {
        fs::set_permissions(target, fs::Permissions::from_mode(0o644)).unwrap();
        let r = sb.suggest(Some(&mock.base()), WORDS, &[("-META_API_KEY", "")]);
        assert_eq!(r.code, 1, "{what}: out={:?} err={:?}", r.out, r.err);
        assert!(r.err.contains("chmod 600"), "{what}: {}", r.err);
        fs::set_permissions(target, fs::Permissions::from_mode(0o600)).unwrap();
    }
    assert_eq!(mock.requests().len(), 1, "the refused runs sent nothing");
    assert!(
        !sb.private_text().contains(FAKE_KEY),
        "the config holds a path, never the key"
    );
}

#[test]
fn a_folder_without_a_start_script_is_advisory_and_costs_nothing() {
    let sb = Sandbox::new("nostart");
    sb.write("package.json", "{\"name\":\"x\"}\n");
    fs::remove_file(sb.ws.join("server.js")).unwrap();
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), WORDS, &[]);
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert!(
        r.out.starts_with("advisory: ") && r.out.contains("start"),
        "{}",
        r.out
    );
    assert_eq!(
        sb.result()["code"],
        "app",
        "the hook tells the user about it"
    );
    assert_eq!(mock.requests().len(), 0);
}

#[test]
fn a_second_run_in_a_busy_folder_does_not_start() {
    let sb = Sandbox::new("busy");
    say_proven(&sb);
    let mock = Mock::start(&sb.dir, "slow", 3000, &good_answer().to_string());
    let base = mock.base();
    let first = std::thread::scope(|s| {
        let h = s.spawn(|| sb.suggest(Some(&base), WORDS, &[]));
        // wait until the first run has made its call
        let t = std::time::Instant::now();
        while mock.requests().is_empty() && t.elapsed().as_secs() < 10 {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let second = sb.suggest(Some(&base), WORDS, &[]);
        assert_eq!(second.code, 1, "out={:?} err={:?}", second.out, second.err);
        assert!(second.err.contains("already"), "{}", second.err);
        h.join().unwrap()
    });
    assert_eq!(first.code, 0, "out={:?} err={:?}", first.out, first.err);
    assert_eq!(mock.requests().len(), 1);
}

fn hook_env(words: &str) -> [(&str, &str); 2] {
    [
        ("TORPOR_MUSE_FROM_HOOK", "session-1"),
        ("TORPOR_MUSE_WORDS", words),
    ]
}

#[test]
fn from_the_hook_the_run_asks_the_engine_first_and_spends_nothing_when_it_should_not() {
    // Torpor is not saving here
    let sb = Sandbox::new("hook1");
    sb.engine_file(
        "status.json",
        "{\"saving\":false,\"off\":false,\"last_turn\":{\"n\":2}}",
    );
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), "-", &hook_env(WORDS));
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert!(
        r.out.starts_with("skipped: ") && r.out.contains("not saving"),
        "{}",
        r.out
    );
    assert_eq!(mock.requests().len(), 0);

    // the assistant already saved a promise from the user's words at this turn
    let sb = Sandbox::new("hook2");
    sb.engine_file(
        "list.txt",
        "#1 login works · advisory · passed 21:13 · your words, turn 2\n",
    );
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), "-", &hook_env(WORDS));
    assert_eq!(r.code, 3, "out={:?} err={:?}", r.out, r.err);
    assert!(
        r.out.starts_with("skipped: ") && r.out.contains("already saved"),
        "{}",
        r.out
    );
    assert_eq!(mock.requests().len(), 0);

    // a promise from an earlier turn does not count: the run goes ahead
    let sb = Sandbox::new("hook3");
    say_proven(&sb);
    sb.engine_file(
        "list.txt",
        "#1 login works · advisory · passed 21:13 · your words, turn 1\n",
    );
    let mock = Mock::ok(&sb.dir, &good_answer());
    let r = sb.suggest(Some(&mock.base()), "-", &hook_env(WORDS));
    assert_eq!(r.code, 0, "out={:?} err={:?}", r.out, r.err);
    assert_eq!(mock.requests().len(), 1);
    assert_eq!(sb.result()["session"], "session-1");
}

/// A folder with no start script is news for the user only where Torpor is saving: after `torpor off` the run ends
/// at the engine's answer, as in any other folder, and the next prompt says nothing about an app.
#[test]
fn a_folder_that_is_no_app_is_news_only_where_torpor_is_saving() {
    let saving = "{\"saving\":true,\"off\":false,\"last_turn\":{\"n\":2}}";
    let off = "{\"saving\":false,\"off\":true,\"last_turn\":{\"n\":2}}";
    for (tag, status, code) in [("hook-app-on", saving, "app"), ("hook-app-off", off, "")] {
        let sb = Sandbox::new(tag);
        sb.write("package.json", "{\"name\":\"x\"}\n");
        fs::remove_file(sb.ws.join("server.js")).unwrap();
        sb.engine_file("status.json", status);
        let mock = Mock::ok(&sb.dir, &good_answer());
        let r = sb.suggest(Some(&mock.base()), "-", &hook_env(WORDS));
        assert_eq!(r.code, 3, "{tag}: out={:?} err={:?}", r.out, r.err);
        assert_eq!(sb.result()["code"], code, "{tag}: {}", r.out);
        assert_eq!(mock.requests().len(), 0, "{tag}");
    }
}

#[test]
fn a_blank_prompt_is_a_usage_error() {
    let sb = Sandbox::new("usage");
    let r = sb.suggest(None, "   ", &[]);
    assert_eq!(r.code, 2, "out={:?} err={:?}", r.out, r.err);
    assert!(r.err.contains("usage"), "{}", r.err);
}

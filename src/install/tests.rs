//! Tests for `init` and `uninstall`: our entries, the checks on the file, and writing it back.

use super::entries::is_ours;
use super::*;
use crate::engine::testing::scratch;
use crate::hook::RETIRED;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn foreign() -> Value {
    json!({
        "schema_version": 1,
        "runtime_capabilities": {"plugin:x": {"enabled": false}},
        "unknown_future_key": {"a": [1, 2, {"b": null}]},
        "hooks": {
            "state": {"approved": ["x"]},
            "Stop": [{"hooks": [{"type": "command", "command": "echo foreign-stop", "timeout": 7}]}],
            "PreToolUse": [{"matcher": "bash", "hooks": [{"type": "command", "command": "echo foreign-pre", "timeout": 3}]}]
        }
    })
}

fn ours_doc(extra: Value) -> Value {
    let mut doc = extra;
    let root = doc.as_object_mut().unwrap();
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .unwrap();
    for ev in EVENTS {
        let cmd = command("/s/home", "/s/bin/torpor", "/s/bin/torpor-muse", ev.name);
        hooks
            .entry(ev.name)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(group(ev, cmd));
    }
    doc
}

#[test]
fn prune_removes_exactly_our_entries() {
    let mut doc = ours_doc(foreign());
    assert_eq!(prune(&mut doc), EVENTS.len());
    assert_eq!(doc, foreign());
    let mut fresh = ours_doc(json!({"schema_version": 1}));
    prune(&mut fresh);
    assert_eq!(
        fresh,
        json!({"schema_version": 1}),
        "an empty hooks object goes too"
    );
}

#[test]
fn retired_rows_are_removed_and_never_written_but_a_foreign_one_stays() {
    let mut before = foreign();
    before["hooks"]["PostCompact"] =
        json!([{"hooks": [{"type": "command", "command": "echo foreign-compact", "timeout": 3}]}]);
    let mut doc = ours_doc(before.clone());
    for name in RETIRED {
        let cmd = command("/s/home", "/s/bin/torpor", "/s/bin/torpor-muse", name);
        let row = json!({"hooks": [{"type": "command", "command": cmd, "timeout": 5}]});
        doc["hooks"]
            .as_object_mut()
            .unwrap()
            .entry(*name)
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(row);
    }
    let err = check_result(&doc, &before).unwrap_err();
    assert!(
        err.contains("PostToolBatch must not be registered"),
        "{err}"
    );
    assert_eq!(prune(&mut doc), EVENTS.len() + RETIRED.len());
    assert_eq!(doc, before);
}

#[test]
fn what_we_write_passes_our_own_check_and_a_broken_edit_does_not() {
    let before = foreign();
    let mut doc = ours_doc(before.clone());
    check_result(&doc, &before).unwrap();
    // a string timeout (Muse drops the whole file), a missing fork, an extra key, a changed foreign hook
    for (path, bad) in [
        ("/hooks/Stop/1/hooks/0/timeout", json!("35")),
        ("/hooks/Stop/1/hooks/0/timeout", json!(35.5)),
        (
            "/hooks/SessionStart/0/matcher",
            json!("startup|resume|clear|compact"),
        ),
        ("/hooks/Stop/1/hooks/0/async", json!(true)),
        ("/hooks/Stop/0/hooks/0/command", json!("echo changed")),
        ("/hooks/state", json!({})),
        ("/unknown_future_key", json!(1)),
    ] {
        let mut broken = doc.clone();
        let (parent, key) = path.rsplit_once('/').unwrap();
        let slot = broken.pointer_mut(parent).unwrap().as_object_mut().unwrap();
        slot.insert(key.into(), bad);
        assert!(
            check_result(&broken, &before).is_err(),
            "{path} should be refused"
        );
    }
    doc["hooks"]["Stop"][1]["hooks"][0]["timeout"] = json!(35);
    check_result(&doc, &before).unwrap();
}

#[test]
fn foreign_problems_are_refused_not_repaired() {
    let p = Path::new("settings.json");
    let mut d = foreign();
    d["hooks"]["Stop"][0]["hooks"][0]["timeout"] = json!("7");
    assert!(check_foreign(&d, p).unwrap_err().contains("whole number"));
    let mut d = foreign();
    d["hooks"]["Stop"] = json!({"not": "a list"});
    assert!(check_foreign(&d, p).is_err());
    let mut d = foreign();
    d["schema_version"] = json!(2);
    assert!(check_foreign(&d, p).is_err());
    d.as_object_mut().unwrap().remove("schema_version");
    assert!(check_foreign(&d, p).is_err());
    check_foreign(&foreign(), p).unwrap();
}

#[test]
fn commands_survive_odd_paths_and_are_recognised_from_any_install() {
    let c = command("/a b/h'ome", "/x/torpor", "/x/torpor-muse", "Stop");
    assert_eq!(
        c,
        r"env TORPOR_HOME='/a b/h'\''ome' TORPOR_BIN=/x/torpor /x/torpor-muse hook Stop"
    );
    assert!(is_ours(&json!({"command": c}), "Stop"));
    assert!(!is_ours(&json!({"command": c}), "SessionStart"));
    assert!(!is_ours(
        &json!({"command": "env TORPOR_HOME=/h torpor hook Stop"}),
        "Stop"
    ));
    assert!(!is_ours(
        &json!({"command": "echo torpor-muse hook Stop"}),
        "Stop"
    ));
    assert!(!is_ours(&json!({"type": "http"}), "Stop"));
}

#[test]
fn a_crash_between_the_temp_write_and_the_rename_leaves_the_old_file() {
    let dir = scratch("install-crash");
    let target = dir.join("settings.json");
    fs::write(&target, b"old contents").unwrap();
    let leftovers = || {
        fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() != "settings.json")
            .count()
    };
    // an error at that moment: old file intact, temp file cleaned up
    let r = write_atomic(&target, b"new contents", 0o600, || {
        Err(io::Error::other("disk gone"))
    });
    assert!(r.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"old contents");
    assert_eq!(leftovers(), 0);
    // a hard stop at that moment (the panic skips the cleanup): old file intact, temp file stays
    let r = catch_unwind(AssertUnwindSafe(|| {
        let _ = write_atomic(&target, b"new contents", 0o600, || panic!("power cut"));
    }));
    assert!(r.is_err());
    assert_eq!(fs::read(&target).unwrap(), b"old contents");
    assert_eq!(leftovers(), 1, "the stale temp file is what a crash leaves");
    // the next write succeeds even with that stale file around
    write_atomic(&target, b"new contents", 0o600, || Ok(())).unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"new contents");
    assert_eq!(
        leftovers(),
        0,
        "a stale temp file with our process id is replaced"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn refusal_says_what_init_would_refuse() {
    let dir = scratch("install-refusal");
    let target = dir.join("settings.json");
    assert_eq!(refusal(&target), None, "no file: init makes one");
    let plant = |text: &str| fs::write(&target, text).unwrap();
    plant(r#"{"schema_version":1,"hooks":{}}"#);
    assert_eq!(refusal(&target), None);
    plant(r#"{"schema_version":2}"#);
    assert!(refusal(&target).unwrap().contains("schema_version is 2"));
    plant("{}");
    assert!(refusal(&target).unwrap().contains("schema_version is null"));
    plant("[1]");
    assert!(refusal(&target).unwrap().contains("not a JSON object"));
    let real = dir.join("real.json");
    fs::rename(&target, &real).unwrap();
    std::os::unix::fs::symlink(&real, &target).unwrap();
    assert!(refusal(&target).unwrap().contains("symbolic link"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_file_mode_is_kept() {
    let dir = scratch("install-mode");
    let target = dir.join("settings.json");
    fs::write(&target, b"{}").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    write_atomic(&target, b"{\"a\":1}", 0o600, || Ok(())).unwrap();
    assert_eq!(
        fs::metadata(&target).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let _ = fs::remove_dir_all(&dir);
}

//! `torpor-muse init` / `uninstall` as a user runs them: the real binary, a throw-away HOME, a fake engine.
//! Owner: S3a-core (research/59 D6 fragile settings, D7 integer timeouts, D14 no approval for settings hooks).

use serde_json::{Value, json};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const EXE: &str = env!("CARGO_BIN_EXE_torpor-muse");

/// Settings as a user may have them: compact, no final newline, foreign hooks (one without a timeout), keys we do not
/// know, and the reserved `hooks.state`.
const PLANTED: &str = r#"{"schema_version":1,"future_key":{"a":[1,{"b":null}],"c":"d"},"runtime_capabilities":{"plugin:x:y":{"enabled":false}},"hooks":{"state":{"seen":["x"]},"SessionStart":[{"matcher":"startup","hooks":[{"type":"command","command":"echo hello","timeout":3}]}],"PreToolUse":[{"matcher":"bash","hooks":[{"type":"command","command":"/opt/audit/pre"}]}],"Stop":[{"hooks":[{"type":"command","command":"/opt/audit/stop","timeout":20}]}]}}"#;

/// The events `init` registers, and the ones it leaves alone (src/hook.rs EVENTS, as the Muse docs name them).
const REGISTERED: [&str; 13] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PostToolBatch",
    "Stop",
    "StopFailure",
    "SessionEnd",
    "PreCompact",
    "PostCompact",
    "SubagentStart",
    "SubagentStop",
];
const NOT_REGISTERED: [&str; 6] = [
    "PreLLMCall",
    "PostLLMCall",
    "PermissionRequest",
    "Notification",
    "Interrupt",
    "SessionFork",
];

fn pinned() -> &'static str {
    include_str!("../engine.lock")
        .lines()
        .find_map(|l| l.strip_prefix("version "))
        .expect("engine.lock has a version line")
        .trim()
}

/// A stand-in engine: answers `version`, and keeps what `hook <Event>` is given in `got.<Event>` (its TORPOR_HOME in
/// `home.<Event>`).
fn fake_engine(dir: &Path, version: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let p = dir.join("torpor");
    let body = format!(
        "#!/bin/sh\nd=$(dirname \"$0\")\ncase \"$1\" in\nversion) echo 'torpor {version}' ;;\nhook) echo \"$TORPOR_HOME\" > \"$d/home.$2\"; cat > \"$d/got.$2\" ;;\nesac\nexit 0\n"
    );
    fs::write(&p, body).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    p
}

struct Rig {
    dir: PathBuf,
    home: PathBuf,
    engine: PathBuf,
}

impl Rig {
    fn new(name: &str) -> Rig {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("core-install-{name}"));
        let _ = fs::remove_dir_all(&dir);
        let home = dir.join("home dir"); // a space on purpose: the settings path has one
        fs::create_dir_all(&home).unwrap();
        let engine = fake_engine(&dir.join("bin dir"), pinned());
        Rig { dir, home, engine }
    }

    fn settings(&self) -> PathBuf {
        self.home.join(".config/muse/settings.json")
    }

    fn backup(&self) -> PathBuf {
        self.home.join(".config/muse/settings.json.torpor-muse.bak")
    }

    fn plant(&self, text: &str) {
        fs::create_dir_all(self.settings().parent().unwrap()).unwrap();
        fs::write(self.settings(), text).unwrap();
    }

    fn read(&self) -> Value {
        serde_json::from_slice(&fs::read(self.settings()).unwrap()).unwrap()
    }

    /// `torpor-muse <args>` with the environment a user shell would have, and nothing else.
    fn command(&self, args: &[&str]) -> Command {
        let mut c = Command::new(EXE);
        c.args(args)
            .env_clear()
            .env("HOME", &self.home)
            .env("PATH", "/usr/bin:/bin")
            .env("TORPOR_HOME", self.dir.join("it's a store"))
            .env("TORPOR_BIN", &self.engine);
        c
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        Self::finish(&mut self.command(args))
    }

    /// `run` with TORPOR_HOME unset: the store is `~/.torpor`, as for a user who followed the README.
    fn run_default_store(&self, args: &[&str]) -> (i32, String, String) {
        Self::finish(self.command(args).env_remove("TORPOR_HOME"))
    }

    fn finish(c: &mut Command) -> (i32, String, String) {
        let o = c.output().unwrap();
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    }

    /// Everything in the settings directory: a refused or crashed edit leaves nothing but the file.
    fn settings_dir_files(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(self.settings().parent().unwrap())
            .map(|d| {
                d.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

/// (event, group, handler) for every handler in the file.
fn handlers(doc: &Value) -> Vec<(String, Value, Value)> {
    let mut v = Vec::new();
    for (event, groups) in doc["hooks"].as_object().into_iter().flatten() {
        for g in groups.as_array().into_iter().flatten() {
            for h in g["hooks"].as_array().into_iter().flatten() {
                v.push((event.clone(), g.clone(), h.clone()));
            }
        }
    }
    v
}

/// Our handlers by event.
fn ours(doc: &Value) -> Vec<(String, Value, Value)> {
    handlers(doc)
        .into_iter()
        .filter(|(_, _, h)| {
            h["command"]
                .as_str()
                .is_some_and(|c| c.starts_with("env TORPOR_HOME=") && c.contains("torpor-muse"))
        })
        .collect()
}

#[test]
fn round_trip_keeps_everything_foreign_and_gives_back_the_same_bytes() {
    let r = Rig::new("round-trip");
    r.plant(PLANTED);
    let (code, out, err) = r.run(&["init"]);
    assert_eq!(code, 0, "{out}{err}");

    let doc = r.read();
    let orig: Value = serde_json::from_str(PLANTED).unwrap();
    assert_eq!(doc["schema_version"], json!(1));
    for k in ["future_key", "runtime_capabilities"] {
        assert_eq!(doc[k], orig[k], "{k} must be untouched");
    }
    assert_eq!(
        doc["hooks"]["state"], orig["hooks"]["state"],
        "hooks.state is never written"
    );
    for ev in ["SessionStart", "PreToolUse", "Stop"] {
        assert_eq!(
            doc["hooks"][ev][0], orig["hooks"][ev][0],
            "the foreign {ev} hook stays first and unchanged"
        );
    }

    // one entry of ours per registered event; D7: `timeout` is a whole number of seconds; AD10: nothing async
    let mine = ours(&doc);
    let mut names: Vec<&str> = mine.iter().map(|(e, _, _)| e.as_str()).collect();
    names.sort_unstable();
    let mut want = REGISTERED.to_vec();
    want.sort_unstable();
    assert_eq!(names, want);
    for (event, group, h) in &mine {
        assert_eq!(h["type"], "command");
        let t = h["timeout"]
            .as_u64()
            .unwrap_or_else(|| panic!("{event}: timeout {} is not an integer", h["timeout"]));
        assert!((1..=60).contains(&t), "{event}: {t}");
        let c = h["command"].as_str().unwrap();
        assert!(c.starts_with("env TORPOR_HOME="), "{c}");
        assert!(
            c.contains(" TORPOR_BIN=") && c.ends_with(&format!(" hook {event}")),
            "{c}"
        );
        assert!(group.get("async").is_none() && h.get("async").is_none());
    }
    let stop = mine.iter().find(|(e, _, _)| e == "Stop").unwrap();
    assert_eq!(
        stop.2["timeout"],
        json!(35),
        "the Stop gate may use 20 s plus its save"
    );
    let start = mine.iter().find(|(e, _, _)| e == "SessionStart").unwrap();
    assert!(
        start.1["matcher"]
            .as_str()
            .unwrap()
            .split('|')
            .any(|m| m == "fork"),
        "D1: fork is matched"
    );
    for e in NOT_REGISTERED {
        assert!(doc["hooks"].get(e).is_none(), "{e} must not be registered");
    }
    assert!(!fs::read_to_string(r.settings()).unwrap().contains("async"));

    // the foreign timeouts are still what they were (the one without a timeout got none)
    assert!(
        doc["hooks"]["PreToolUse"][0]["hooks"][0]
            .get("timeout")
            .is_none()
    );
    assert_eq!(
        fs::read(r.backup()).unwrap(),
        PLANTED.as_bytes(),
        "the original is saved once"
    );

    let (code, out, err) = r.run(&["uninstall"]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(
        fs::read(r.settings()).unwrap(),
        PLANTED.as_bytes(),
        "byte for byte what it was"
    );
    assert!(!r.backup().exists());
    assert_eq!(r.settings_dir_files(), ["settings.json"]);
}

#[test]
fn init_twice_changes_nothing_the_second_time() {
    let r = Rig::new("twice");
    r.plant(PLANTED);
    assert_eq!(r.run(&["init"]).0, 0);
    let first = fs::read(r.settings()).unwrap();
    let (code, out, _) = r.run(&["init"]);
    assert_eq!(code, 0);
    assert!(out.contains("already installed"), "{out}");
    assert_eq!(fs::read(r.settings()).unwrap(), first);
    assert_eq!(ours(&r.read()).len(), REGISTERED.len(), "never doubled");
    assert_eq!(
        fs::read(r.backup()).unwrap(),
        PLANTED.as_bytes(),
        "the backup stays the original"
    );
}

#[test]
fn a_fresh_install_is_removed_completely() {
    let r = Rig::new("fresh");
    assert_eq!(r.run(&["init"]).0, 0);
    let doc = r.read();
    assert_eq!(doc["schema_version"], json!(1));
    assert_eq!(ours(&doc).len(), REGISTERED.len());
    assert_eq!(
        r.settings_dir_files(),
        ["settings.json"],
        "no backup of a file that did not exist"
    );
    assert_eq!(r.run(&["uninstall"]).0, 0);
    assert!(
        !r.settings().exists(),
        "init made the file, so uninstall deletes it"
    );
    let (code, out, _) = r.run(&["uninstall"]);
    assert_eq!(code, 0, "a second uninstall is not an error: {out}");
}

#[test]
fn init_then_uninstall_on_an_empty_home_leaves_it_empty() {
    let r = Rig::new("empty-home");
    assert_eq!(r.run_default_store(&["init"]).0, 0);
    assert!(
        r.home.join(".torpor/muse/engine-ok").is_file(),
        "init made the cache and the folders around it"
    );
    let (code, out, err) = r.run_default_store(&["uninstall"]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(out.lines().count(), 1, "nothing is left to report: {out}");
    assert_eq!(
        fs::read_dir(&r.home).unwrap().count(),
        0,
        "HOME is as it was before init"
    );
    let (code, out, _) = r.run_default_store(&["uninstall"]);
    assert_eq!((code, out.lines().count()), (0, 1), "{out}");
}

#[test]
fn uninstall_keeps_the_ledger_and_the_saved_checks_and_says_so() {
    let r = Rig::new("keeps");
    assert_eq!(r.run_default_store(&["init"]).0, 0);
    let ledger = r.home.join(".torpor/muse/suggest/ledger.jsonl");
    let result = r
        .home
        .join(".torpor/muse/suggest/ws/0123456789abcdef/result.json");
    let saved_check = r.home.join(".torpor/0123456789abcdef/promises.json"); // the engine's own store
    for f in [&ledger, &result, &saved_check] {
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, "mine").unwrap();
    }
    let (code, out, err) = r.run_default_store(&["uninstall"]);
    assert_eq!(code, 0, "{out}{err}");
    for f in [&ledger, &result, &saved_check] {
        assert_eq!(fs::read_to_string(f).unwrap(), "mine", "{}", f.display());
    }
    assert!(!r.home.join(".torpor/muse/engine-ok").exists());
    assert!(!r.home.join(".config").exists(), "the empty folders go");
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 2, "{out}");
    let state = r.home.join(".torpor/muse");
    for want in [state.to_str().unwrap(), "ledger", "your choice"] {
        assert!(lines[1].contains(want), "{want:?} not in {:?}", lines[1]);
    }
}

#[test]
fn a_muse_config_folder_with_other_files_stays() {
    let r = Rig::new("config-stays");
    assert_eq!(r.run_default_store(&["init"]).0, 0);
    let other = r.home.join(".config/muse/keybindings.json");
    fs::write(&other, "mine").unwrap();
    let (code, out, err) = r.run_default_store(&["uninstall"]);
    assert_eq!(code, 0, "{out}{err}");
    assert_eq!(fs::read_to_string(&other).unwrap(), "mine");
    assert!(
        !r.settings().exists(),
        "init made settings.json, so it goes"
    );
    assert_eq!(out.lines().count(), 1, "{out}");
    assert!(
        !r.home.join(".torpor").exists(),
        "the store folders were empty"
    );
}

#[test]
fn without_a_home_uninstall_leaves_the_folders_of_the_current_directory() {
    let r = Rig::new("no-home");
    let cwd = r.dir.join("cwd");
    fs::create_dir_all(cwd.join(".config/muse")).unwrap();
    fs::create_dir_all(cwd.join(".torpor/muse")).unwrap();
    let o = r
        .command(&["uninstall"])
        .env_remove("HOME")
        .env_remove("TORPOR_HOME")
        .current_dir(&cwd)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    assert!(cwd.join(".config/muse").is_dir() && cwd.join(".torpor/muse").is_dir());
}

#[test]
fn uninstall_keeps_what_the_user_added_after_init() {
    let r = Rig::new("added");
    r.plant(PLANTED);
    assert_eq!(r.run(&["init"]).0, 0);
    let mut doc = r.read();
    doc["hooks"]["PreToolUse"].as_array_mut().unwrap().push(
        json!({"matcher":"bash","hooks":[{"type":"command","command":"/opt/audit/later","timeout":9}]}),
    );
    doc["another_key"] = json!(true);
    fs::write(r.settings(), serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
    let (code, out, _) = r.run(&["uninstall"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("your other settings are kept")
            && out.contains("settings.json.torpor-muse.bak"),
        "the saved copy stays, and the message says where: {out}"
    );
    assert!(r.backup().exists());
    let after = r.read();
    assert_eq!(after["another_key"], json!(true));
    assert_eq!(after["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
    assert_eq!(
        after["hooks"]["PreToolUse"][1]["hooks"][0]["command"],
        "/opt/audit/later"
    );
    assert!(ours(&after).is_empty());
    assert!(
        !fs::read_to_string(r.settings())
            .unwrap()
            .contains("torpor-muse")
    );
}

#[test]
fn a_symlinked_settings_file_is_refused_and_left_alone() {
    let r = Rig::new("symlink");
    let real = r.dir.join("real-settings.json");
    fs::write(&real, PLANTED).unwrap();
    fs::create_dir_all(r.settings().parent().unwrap()).unwrap();
    symlink(&real, r.settings()).unwrap();
    for args in [["init"], ["uninstall"]] {
        let (code, _, err) = r.run(&args);
        assert_eq!(code, 1, "{args:?}");
        assert!(err.contains("symbolic link"), "{err}");
        assert!(
            fs::symlink_metadata(r.settings())
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), PLANTED);
        assert_eq!(
            r.settings_dir_files(),
            ["settings.json"],
            "no backup, no temp file"
        );
    }
    // a dangling link is no better: init must not create a file through it
    fs::remove_file(r.settings()).unwrap();
    symlink(r.dir.join("does-not-exist.json"), r.settings()).unwrap();
    assert_eq!(r.run(&["init"]).0, 1);
    assert!(!r.dir.join("does-not-exist.json").exists());
}

#[test]
fn settings_that_muse_would_choke_on_are_refused_not_repaired() {
    let cases: [(&str, &str); 9] = [
        ("{ not json", "not valid JSON"),
        (r#"{"schema_version":2}"#, "schema_version"),
        (r#"{"hooks":{}}"#, "schema_version"),
        ("[]", "top level"),
        (
            r#"{"schema_version":1,"hooks":[]}"#,
            "`hooks` is not an object",
        ),
        (
            r#"{"schema_version":1,"hooks":{"Stop":{}}}"#,
            "is not a list",
        ),
        (
            r#"{"schema_version":1,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"x","timeout":"10"}]}]}}"#,
            "timeout",
        ),
        (
            r#"{"schema_version":1,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"x","timeout":1.5}]}]}}"#,
            "timeout",
        ),
        (
            r#"{"schema_version":1,"hooks":{"Stop":[{"hooks":[{"type":"command","command":"x","timeout":-1}]}]}}"#,
            "timeout",
        ),
    ];
    let r = Rig::new("refused");
    for (text, why) in cases {
        r.plant(text);
        let (code, out, err) = r.run(&["init"]);
        assert_eq!(code, 1, "{text}: {out}");
        assert!(err.contains(why), "{text}: wanted {why:?} in {err:?}");
        assert_eq!(
            fs::read_to_string(r.settings()).unwrap(),
            text,
            "the file is untouched"
        );
        assert_eq!(
            r.settings_dir_files(),
            ["settings.json"],
            "no backup, no temp file"
        );
    }
}

#[test]
fn without_the_pinned_engine_init_touches_nothing() {
    let r = Rig::new("engine");
    r.plant(PLANTED);
    // wrong version
    let wrong = fake_engine(&r.dir.join("wrong"), "9.9.9");
    let o = r
        .command(&["init"])
        .env("TORPOR_BIN", wrong)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("9.9.9"));
    // missing
    let o = r
        .command(&["init"])
        .env("TORPOR_BIN", r.dir.join("nope"))
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    // not found anywhere
    let o = r
        .command(&["init"])
        .env_remove("TORPOR_BIN")
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("TORPOR_BIN"));
    assert_eq!(fs::read_to_string(r.settings()).unwrap(), PLANTED);
    assert_eq!(r.settings_dir_files(), ["settings.json"]);
}

#[test]
fn xdg_config_home_wins_over_home() {
    let r = Rig::new("xdg");
    let xdg = r.dir.join("xdg");
    let o = r
        .command(&["init"])
        .env("XDG_CONFIG_HOME", &xdg)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    assert!(xdg.join("muse/settings.json").is_file());
    assert!(!r.settings().exists());
    let o = r
        .command(&["uninstall"])
        .env("XDG_CONFIG_HOME", &xdg)
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(0));
    assert!(!xdg.join("muse/settings.json").exists());
}

/// Muse runs a hook as `$SHELL -c <command>` with a cleared environment (D5). Run exactly that, with awkward paths
/// (a space in the engine's folder, a quote in the store's), and see the payload reach the engine.
#[test]
fn the_installed_command_works_in_the_environment_muse_gives_it() {
    let r = Rig::new("command");
    assert_eq!(r.run(&["init"]).0, 0);
    let doc = r.read();
    let (_, _, h) = ours(&doc)
        .into_iter()
        .find(|(e, _, _)| e == "SessionStart")
        .unwrap();
    let cmd = h["command"].as_str().unwrap();

    let mut child = Command::new("/bin/sh")
        .args(["-c", cmd])
        .env_clear()
        .env("HOME", &r.home)
        .env("PATH", "/usr/bin:/bin")
        .env("USER", "someone")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let payload =
        r#"{"session_id":"s-1","hook_event_name":"SessionStart","cwd":"/tmp","source":"startup"}"#;
    std::io::Write::write_all(&mut child.stdin.take().unwrap(), payload.as_bytes()).unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );

    let got =
        fs::read_to_string(r.dir.join("bin dir/got.SessionStart")).expect("the engine was called");
    let got: Value = serde_json::from_str(&got).unwrap();
    assert_eq!(got["session_id"], "s-1");
    let home = fs::read_to_string(r.dir.join("bin dir/home.SessionStart")).unwrap();
    assert_eq!(
        home.trim_end(),
        r.dir.join("it's a store").to_str().unwrap(),
        "the baked TORPOR_HOME reached the engine"
    );
}

// ---------------------------------------------------------------- the command line

#[test]
fn asking_for_help_prints_the_usage_and_succeeds() {
    let r = Rig::new("help");
    for args in [
        vec!["help"],
        vec!["--help"],
        vec!["-h"],
        vec!["suggest", "--help"],
        vec!["suggest", "-h"],
    ] {
        let (code, out, err) = r.run(&args);
        assert_eq!(code, 0, "{args:?}: {err}");
        assert!(out.starts_with("usage: torpor-muse "), "{args:?}: {out}");
        assert!(err.is_empty(), "{args:?}: {err}");
    }
    // a mistake is still an error, on stderr
    let (code, out, err) = r.run(&["frobnicate"]);
    assert_eq!((code, out.is_empty()), (2, true));
    assert!(err.starts_with("usage: torpor-muse "), "{err}");
}

#[test]
fn an_argument_that_is_not_utf8_is_an_error_not_a_crash() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let r = Rig::new("non-utf8");
    let bad = OsStr::from_bytes(b"/tmp/\xff\xfe.wav");
    let (code, _, err) = Rig::finish(r.command(&["version"]).arg(bad));
    assert_eq!(code, 2, "{err}");
    assert!(err.starts_with("usage: torpor-muse "), "{err}");
    let (code, _, err) = Rig::finish(r.command(&["say"]).arg(bad));
    assert!(matches!(code, 1..=3), "rc {code}: {err}");
    assert!(!err.contains("panicked"), "{err}");
}

// ---------------------------------------------------------------- doctor

#[test]
fn doctor_names_what_stops_init_instead_of_sending_the_user_to_it() {
    let r = Rig::new("doctor-refusal");
    for (text, why) in [
        (r#"{"schema_version":2}"#, "schema_version"),
        ("{}", "schema_version"),
        ("[1]", "top level"),
    ] {
        r.plant(text);
        let (code, out, _) = r.run(&["doctor"]);
        assert_eq!(code, 1, "{text}: {out}");
        assert!(
            out.contains("no torpor-muse hook can be installed") && out.contains(why),
            "{text}: {out}"
        );
        assert!(!out.contains("do: torpor-muse init"), "{text}: {out}");
    }
    // a settings file that is a symbolic link: init would not touch it either
    let real = r.dir.join("real-settings.json");
    fs::write(&real, r#"{"schema_version":1}"#).unwrap();
    fs::remove_file(r.settings()).unwrap();
    symlink(&real, r.settings()).unwrap();
    let (code, out, _) = r.run(&["doctor"]);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("symbolic link"), "{out}");
    assert!(!out.contains("do: torpor-muse init"), "{out}");
    // a file init can use, or none at all, still gets "run init"
    fs::remove_file(r.settings()).unwrap();
    for text in [Some(r#"{"schema_version":1}"#), None] {
        if let Some(t) = text {
            r.plant(t);
        } else {
            fs::remove_file(r.settings()).unwrap();
        }
        let (code, out, _) = r.run(&["doctor"]);
        assert_eq!(code, 1, "{out}");
        assert!(out.contains("do: torpor-muse init"), "{out}");
        assert!(!out.contains("can be installed"), "{out}");
    }
}

#[test]
fn doctor_says_when_the_check_writer_has_no_key_file_and_why() {
    let r = Rig::new("doctor-writer");
    assert_eq!(r.run(&["init"]).0, 0);
    let line_of = |out: &str| {
        out.lines()
            .find(|l| l.starts_with("warn") && l.contains("check-writer off"))
            .map(str::to_owned)
    };
    let (_, out, _) = r.run(&["doctor"]);
    let line = line_of(&out).unwrap_or_else(|| panic!("{out}"));
    assert!(line.contains("key_file"), "{line}");
    assert!(out.contains("do: put your Meta API key"), "{out}");
    // a META_API_KEY in this shell does not reach Muse's hooks: doctor does not count it
    let (_, out, _) = Rig::finish(
        r.command(&["doctor"])
            .env("META_API_KEY", "LLM|0|fake-key-for-tests"),
    );
    assert!(line_of(&out).is_some(), "{out}");
    // a config naming a key file only the user can read: nothing to say
    let cfg = r.home.join(".config/torpor-muse/config");
    let key = r.home.join("key.txt");
    fs::create_dir_all(cfg.parent().unwrap()).unwrap();
    fs::write(&cfg, "key_file = ~/key.txt\n").unwrap();
    fs::write(&key, "LLM|0|fake-key-for-tests\n").unwrap();
    fs::set_permissions(&cfg, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).unwrap();
    let (_, out, _) = r.run(&["doctor"]);
    assert!(line_of(&out).is_none(), "{out}");
    // the same file readable by others: named
    fs::set_permissions(&key, fs::Permissions::from_mode(0o644)).unwrap();
    let (_, out, _) = r.run(&["doctor"]);
    assert!(
        line_of(&out).is_some_and(|l| l.contains("only you can read")),
        "{out}"
    );
}

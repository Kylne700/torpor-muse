//! A database in use: the turn that rewrote one, the delete denied until its app stops, and the way out of both.

mod hook_common;

use hook_common::*;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// dogfood1 M1: a turn whose shell command rewrote a database (the app made a new, empty one). `torpor undo N` and a
/// plain `torpor back` leave database content as it is, so the Stop line names the command that does put the data back,
/// the next prompt gives the model that command with the engine's path, and each, run exactly as printed, brings the
/// old bytes back. A turn that changed no database keeps today's text.
#[test]
fn real_engine_a_turn_that_rewrote_a_database_names_the_command_that_puts_it_back() {
    let sh = |r: &Rig, cmd: &str, path: &str| {
        let o = r
            .command("/bin/sh", &["-c", cmd])
            .env("PATH", path)
            .env_remove("TORPOR_BIN")
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        );
        (o.status.code().unwrap_or(-1), text)
    };
    let turn = |r: &Rig, file: &str| {
        r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
        let (mut pre, mut post) = (r.fixture("PreToolUse-bash"), r.fixture("PostToolUse-bash"));
        for p in [&mut pre, &mut post] {
            p["tool_input"]["command"] = json!(format!("rm -f {file} && node server.js"));
            p["tool_use_id"] = json!("call_rm_db");
        }
        r.hook_ok("PreToolUse", &pre);
        fs::write(r.cwd.join(file), b"SQLite format 3\0").unwrap();
        r.hook_ok("PostToolUse", &post);
        note_parts(&r.hook_ok("Stop", &r.fixture("Stop")).out).0
    };
    let next = |r: &Rig, n: u32| {
        let mut f = r.fixture("UserPromptSubmit");
        f["turn_id"] = json!(format!("00000000-0000-7000-8000-00000000000{n}"));
        f
    };
    let old = b"SQLite format 3\0the user's notes".to_vec();

    let r = Rig::real("real-db");
    let db = r.cwd.join("notes.db");
    fs::write(&db, &old).unwrap();
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    let said = turn(&r, "notes.db");
    assert!(
        said.contains("Torpor: save point ")
            && said.contains(" · turn 1 changed ")
            && !said.contains("undo it"),
        "the engine's line stays, without the command the person does not type: {said}"
    );
    // L3 N2: the database line comes first and ends with the command within 100 characters (the status bar is short)
    let first = said.split(" — ").next().unwrap_or("");
    let user = first.split_once("then ").map_or("", |(_, c)| c).to_owned();
    assert!(
        first.starts_with("Torpor: turn 1 changed notes.db. Get it back: stop the app, then ")
            && first.chars().count() <= 100
            && user.starts_with("torpor back --id ")
            && user.ends_with(" --with-data"),
        "{said}"
    );
    // the user's terminal finds the engine as `torpor` on its PATH
    let path = format!("{}:/usr/bin:/bin", r.engine.parent().unwrap().display());
    let (code, text) = sh(&r, &user, &path);
    assert_eq!((code, fs::read(&db).unwrap()), (0, old.clone()), "{text}");

    fs::write(&db, b"SQLite format 3\0").unwrap(); // broken again, for the model's copy of the command
    let (_, ctx) = note_parts(&r.hook_ok("UserPromptSubmit", &next(&r, 3)).out);
    let model = ctx
        .rsplit_once("The command: ")
        .map_or("", |(_, c)| c)
        .to_owned();
    assert_eq!(
        model,
        format!("{}{}", r.engine.display(), &user["torpor".len()..]),
        "{ctx}"
    );
    assert!(ctx.contains("every other file"), "{ctx}");
    let (code, text) = sh(&r, &model, "/usr/bin:/bin");
    assert_eq!((code, fs::read(&db).unwrap()), (0, old), "{text}");
    let again = r.hook_ok("UserPromptSubmit", &next(&r, 4)).out;
    assert!(
        !again.contains("--with-data"),
        "the model is told once: {again}"
    );
    // with saving off the engine's last turn stays the database one: its line must not come back at every Stop
    assert_eq!(engine_cli(&r, &["off"]).0, 0);
    let stop = r.hook_ok("Stop", &r.fixture("Stop")).out;
    assert!(!stop.contains("--with-data"), "{stop}");

    // control: the same turn on a file that is not a database keeps today's text, and the model hears nothing
    let c = Rig::real("real-db-control");
    fs::write(c.cwd.join("notes.txt"), &b"the user's notes"[..]).unwrap();
    c.hook_ok("SessionStart", &c.fixture("SessionStart"));
    let said = turn(&c, "notes.txt");
    assert!(
        said.starts_with("Torpor: save point ") && !said.contains('\n'),
        "{said}"
    );
    assert!(
        !said.contains("--with-data") && !said.contains("database"),
        "{said}"
    );
    let out = c.hook_ok("UserPromptSubmit", &next(&c, 3)).out;
    assert!(out.is_empty(), "{out}");
}

fn rows_in(db: &Path) -> String {
    let o = Command::new("python3")
        .args(["-I", "-c", "import sqlite3,sys; print(sqlite3.connect(sys.argv[1]).execute('select count(*) from t').fetchone()[0])"])
        .arg(db)
        .output()
        .unwrap();
    String::from_utf8_lossy(&o.stdout).trim().to_owned()
}

/// F4 item 4 (drill D07). The engine saves no database while its app holds a log that is not empty, and says nothing
/// about it when a command deletes that database, so `torpor back --with-data` could not bring it back ("its database
/// was busy"). The hook now denies such a command with a reason (Muse runs on); once the app has stopped the very same
/// command goes through, the engine then saves the file, and `back --with-data` brings the 300 rows back.
#[test]
fn real_engine_a_command_that_would_delete_a_database_in_use_is_denied_until_its_app_stops() {
    const RM: &str = "rm -f data/app.db data/app.db-wal data/app.db-shm";
    let r = Rig::real("real-busy-db");
    let db = r.cwd.join("data/app.db");
    fs::create_dir(r.cwd.join("data")).unwrap();
    fs::write(r.cwd.join("base.txt"), "keep\n").unwrap();
    let app = BusyApp::start(&db);
    let wal = r.cwd.join("data/app.db-wal");
    assert!(
        fs::metadata(&wal).unwrap().len() > 0,
        "the app's log is not empty"
    );

    let turn2 = |mut p: Value| {
        p["turn_id"] = json!("00000000-0000-7000-8000-000000000012");
        p
    };
    let pre = |cmd: &str, id: &str| {
        let mut p = turn2(r.fixture("PreToolUse-bash"));
        p["tool_input"]["command"] = json!(cmd);
        p["tool_use_id"] = json!(id);
        p
    };
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    r.hook_ok("Stop", &r.fixture("Stop"));
    r.hook_ok("UserPromptSubmit", &turn2(r.fixture("UserPromptSubmit")));

    // while the app runs: the delete is denied, by file names and by folder, with a reason the model can act on
    for cmd in [RM, "rm -rf data"] {
        let out = r.hook_ok("PreToolUse", &pre(cmd, "call_rm_busy")).out;
        let v: Value = serde_json::from_str(&out).unwrap_or_else(|e| panic!("{cmd}: {e}: {out:?}"));
        let (d, why) = (
            v["hookSpecificOutput"]["permissionDecision"].as_str(),
            v["hookSpecificOutput"]["permissionDecisionReason"]
                .as_str()
                .unwrap_or(""),
        );
        assert_eq!(d, Some("deny"), "{cmd}: {out}");
        for w in [
            "not run",
            "data/app.db",
            "in use",
            "Stop the app",
            "run the command again",
            "ask the user",
        ] {
            assert!(why.contains(w), "{cmd}: {w:?} is missing from {why}");
        }
    }
    // commands that name no busy database, and read-only uses of it, are not stopped
    for cmd in [
        "ls data",
        "cat data/app.db > /dev/null",
        "rm -f missing.txt",
        "cp data/app.db /tmp/copy-of-app.db.bak",
    ] {
        let out = r.hook_ok("PreToolUse", &pre(cmd, "call_other")).out;
        assert!(!out.contains("permissionDecision"), "{cmd}: {out}");
    }
    assert!(
        db.exists() && wal.exists(),
        "the hook never touches the files"
    );

    // the app stops (its log goes into the database file); the same command now goes through
    app.stop();
    assert!(db.exists() && !wal.exists());
    assert_eq!(rows_in(&db), "300");
    let out = r.hook_ok("PreToolUse", &pre(RM, "call_rm_free")).out;
    assert!(!out.contains("permissionDecision"), "{out}");
    for s in ["", "-wal", "-shm"] {
        let _ = fs::remove_file(format!("{}{s}", db.display()));
    }
    let mut post = turn2(r.fixture("PostToolUse-bash"));
    post["tool_input"]["command"] = json!(RM);
    post["tool_use_id"] = json!("call_rm_free");
    r.hook_ok("PostToolUse", &post);
    let last_stop = r.hook_ok("Stop", &turn2(r.fixture("Stop"))).out;
    assert!(!db.exists());

    // `back --with-data` to the save point the engine took when the command went through brings the 300 rows back;
    // the one it took while the app still ran (the denied call) had nothing to bring back: that is what the deny saves
    let (code, shown) = engine_cli(&r, &["show", "2", "--json"]);
    assert_eq!(code, 0, "{shown}");
    let shown: Value = serde_json::from_str(&shown).unwrap();
    let pres: Vec<u64> = shown["points"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["kind"].as_str().is_some_and(|k| k.starts_with("pre_")))
        .filter_map(|p| p["id"].as_u64())
        .collect();
    assert!(pres.len() >= 2, "{shown}");
    let (early, late) = (pres[0].to_string(), pres[pres.len() - 1].to_string());
    // F4 item 3: that very Stop told the model (a block) and the user (the line) the save point that has the 300 rows, not
    // the turn's start (where the app was running and the engine saved no database)
    let told: Value = serde_json::from_str(&last_stop).unwrap_or(Value::Null);
    let cmd = format!("back --id {late} --with-data");
    let why = blocked(&told).unwrap_or_else(|| panic!("no block: {last_stop}"));
    assert!(
        why.contains("This turn deleted data/app.db (database files).")
            && why.contains(&format!("`{} {cmd}`", r.engine.display())),
        "{why}"
    );
    assert!(
        said(&told).starts_with(&format!(
            "Torpor: turn 2 changed data/app.db. Get it back: stop the app, then torpor {cmd}"
        )),
        "{last_stop}"
    );
    let (_, text) = engine_cli(&r, &["back", "--id", &early, "--with-data", "--dry-run"]);
    assert!(text.contains("its database was busy"), "{text}");
    let (code, text) = engine_cli(&r, &["back", "--id", &late, "--with-data"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(rows_in(&db), "300", "{text}");

    // the drill's own D07: the writer was killed (SIGKILL), no app runs, its log is still not empty. The engine saves
    // such a database no more than a live one, so the delete is denied and the reason names the way out (the next
    // tests walk it); with saving off (nothing is saved anyway) nothing is denied
    let stale = r.cwd.join("data/stale.db");
    drop(BusyApp::start(&stale));
    assert!(
        fs::metadata(format!("{}-wal", stale.display()))
            .unwrap()
            .len()
            > 0
    );
    let out = r
        .hook_ok("PreToolUse", &pre("rm data/stale.db", "call_rm_stale"))
        .out;
    assert!(
        out.contains("permissionDecision")
            && out.contains("a crash left its files behind")
            && out.contains("SELECT count(*) FROM sqlite_master")
            && out.contains("If it is refused again, ask the user")
            && out.contains("in their own terminal")
            && !out.contains("stays refused"),
        "{out}"
    );
    assert_eq!(engine_cli(&r, &["off"]).0, 0);
    let out = r
        .hook_ok("PreToolUse", &pre("rm data/stale.db", "call_rm_stale_off"))
        .out;
    assert!(!out.contains("permissionDecision"), "{out}");
}

// ------------------------------------------------ F4c item 10: the way out of a database a crash left behind

/// The state a database is left in, made with python's sqlite3 module (the library real apps use); the table `t` has
/// 5 committed rows. `wal`: the app was killed with them only in its log. `hot`: killed in the middle of a transaction
/// (a small cache pushes the unfinished pages into the file), so a journal waits to be rolled back. `persist` and
/// `truncate`: closed cleanly in a journal mode that keeps the `-journal` file (zero header, empty). `app`: an app
/// still runs and holds the database open (the one thing that is returned, so the test keeps it alive).
fn leftover(db: &Path, kind: &str) -> Option<BusyApp> {
    const SRC: &str = r#"
import os, signal, sqlite3, sys
kind, db = sys.argv[1], sys.argv[2]
con = sqlite3.connect(db, isolation_level=None)
if kind == "wal":
    con.execute("PRAGMA journal_mode=WAL")
    con.execute("PRAGMA wal_autocheckpoint=0")
elif kind != "hot":
    con.execute("PRAGMA journal_mode=" + kind)
con.execute("create table t(id integer primary key, v text)")
con.execute("begin")
for i in range(5):
    con.execute("insert into t(v) values (?)", ("row%d" % i,))
con.execute("commit")
if kind == "hot":
    con.execute("PRAGMA cache_size=5")
    con.execute("begin")
    for i in range(3000):
        con.execute("insert into t(v) values (?)", ("x" * 500,))
if kind in ("wal", "hot"):
    os.kill(os.getpid(), signal.SIGKILL)  # the app dies: no close, no checkpoint, no rollback
con.close()
"#;
    if kind == "app" {
        return Some(BusyApp::start(db));
    }
    let o = Command::new("python3")
        .args(["-I", "-c", SRC, kind])
        .arg(db)
        .output()
        .expect("python3 is needed for the database tests");
    assert_eq!(
        o.status.success(),
        !matches!(kind, "wal" | "hot"),
        "{kind}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    None
}

/// The commands the deny text offers. The `sqlite3` program is not on every machine (not on the one these were written
/// on), so its form is run only where it is installed.
fn exit_forms() -> Vec<&'static str> {
    let mut forms = vec!["python3 -c"];
    if Command::new("sqlite3").arg("-version").output().is_ok() {
        forms.push("sqlite3 ");
    }
    forms
}

fn second_turn(mut p: Value) -> Value {
    p["turn_id"] = json!("00000000-0000-7000-8000-000000000012");
    p
}

/// A Bash call in the second turn: the reason when the hook denies it, None when it goes through.
fn denial(r: &Rig, cmd: &str, id: &str) -> Option<String> {
    let mut p = second_turn(r.fixture("PreToolUse-bash"));
    p["tool_input"]["command"] = json!(cmd);
    p["tool_use_id"] = json!(id);
    let out = r.hook_ok("PreToolUse", &p).out;
    let v: Value = serde_json::from_str(&out).unwrap_or(Value::Null);
    let h = &v["hookSpecificOutput"];
    match h["permissionDecision"].as_str() {
        Some("deny") => Some(
            h["permissionDecisionReason"]
                .as_str()
                .unwrap_or("")
                .to_owned(),
        ),
        other => {
            assert!(other.is_none(), "{cmd}: {out}");
            None
        }
    }
}

/// What the model meets after the app is gone, with the real engine and real SQLite files: `rm data/app.db` is denied,
/// the model runs the command the text names (`form`: the start of the one it picks), and tries the delete again.
/// `wal` and `hot` are cleared by that one read, so the delete goes through with the engine holding a copy; the other
/// kinds are not, so the delete stays denied and the text sends the model to the user.
fn exit_scenario(kind: &str, form: &str) {
    let r = Rig::real(&format!(
        "real-exit-{kind}-{}",
        form.split_whitespace().next().unwrap()
    ));
    let db = r.cwd.join("data/app.db");
    fs::create_dir(r.cwd.join("data")).unwrap();
    fs::write(r.cwd.join("base.txt"), "keep\n").unwrap();
    let _app = leftover(&db, kind);
    let side = |s: &str| PathBuf::from(format!("{}{s}", db.display()));
    let (wal, shm, jrn) = (side("-wal"), side("-shm"), side("-journal"));
    // the state is what the guard calls busy (so a missing state fails here, not later)
    match kind {
        "wal" | "app" => assert!(fs::metadata(&wal).unwrap().len() > 0 && shm.exists()),
        "hot" => assert_eq!(
            fs::read(&jrn).unwrap()[..8],
            [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7],
            "a journal that waits to be rolled back"
        ),
        "persist" => assert_eq!(fs::read(&jrn).unwrap()[..8], [0; 8]),
        "truncate" => assert_eq!(fs::metadata(&jrn).unwrap().len(), 0),
        _ => unreachable!("{kind}"),
    }
    r.hook_ok("SessionStart", &r.fixture("SessionStart"));
    r.hook_ok("UserPromptSubmit", &r.fixture("UserPromptSubmit"));
    r.hook_ok("Stop", &r.fixture("Stop"));
    r.hook_ok(
        "UserPromptSubmit",
        &second_turn(r.fixture("UserPromptSubmit")),
    );

    let why = denial(&r, "rm data/app.db", "call_rm_1")
        .unwrap_or_else(|| panic!("{kind}: the delete is not denied"));
    assert!(
        why.contains("a crash left its files behind")
            && why.contains("If it is refused again, ask the user")
            && !why.contains("stays refused"),
        "{why}"
    );
    // the way out is in the text; with `<db>` filled in it goes through the hook (neither the guard nor the engine
    // denies it) and runs
    let exit = why
        .split('`')
        .skip(1)
        .step_by(2)
        .find(|c| c.starts_with(form))
        .unwrap_or_else(|| panic!("no {form:?} command in {why}"))
        .replace("<db>", "data/app.db");
    assert_eq!(denial(&r, &exit, "call_exit"), None, "{exit}");
    let run = Command::new("sh")
        .args(["-c", &exit])
        .current_dir(&r.cwd)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{exit}: {}",
        String::from_utf8_lossy(&run.stderr)
    );

    let clears = matches!(kind, "wal" | "hot");
    let left: Vec<&PathBuf> = [&wal, &shm, &jrn]
        .into_iter()
        .filter(|p| p.exists())
        .collect();
    assert!(db.exists(), "the read never removes the database");
    assert_eq!(
        left.is_empty(),
        clears,
        "{kind}: left after the read: {left:?}"
    );
    // every committed row is there; the unfinished transaction of the `hot` kind is the one thing rolled back
    assert_eq!(rows_in(&db), if kind == "app" { "300" } else { "5" });

    let retry = denial(&r, "rm data/app.db", "call_rm_2");
    if !clears {
        let again = retry.unwrap_or_else(|| panic!("{kind}: the delete went through"));
        assert!(again.contains("ask the user"), "{again}");
        return;
    }
    assert_eq!(
        retry, None,
        "{kind}: the files are gone, the same delete goes through"
    );
    fs::remove_file(&db).unwrap();
    let mut post = second_turn(r.fixture("PostToolUse-bash"));
    post["tool_input"]["command"] = json!("rm data/app.db");
    post["tool_use_id"] = json!("call_rm_2");
    r.hook_ok("PostToolUse", &post);
    let last_stop = r.hook_ok("Stop", &second_turn(r.fixture("Stop"))).out;

    // the engine's own record, as in D07: the save point taken while the files were still there has no copy of the
    // database ("its database was busy"), the one taken at the retried delete has it, and `back` returns its 5 rows
    let (code, shown) = engine_cli(&r, &["show", "2", "--json"]);
    assert_eq!(code, 0, "{shown}");
    let shown: Value = serde_json::from_str(&shown).unwrap();
    let pres: Vec<u64> = shown["points"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["kind"].as_str().is_some_and(|k| k.starts_with("pre_")))
        .filter_map(|p| p["id"].as_u64())
        .collect();
    assert!(pres.len() >= 2, "{shown}");
    let (early, late) = (pres[0].to_string(), pres[pres.len() - 1].to_string());
    let told: Value = serde_json::from_str(&last_stop).unwrap_or(Value::Null);
    assert!(
        blocked(&told).is_some_and(|w| w.contains(&format!("back --id {late} --with-data"))),
        "the Stop names the save point that has the copy: {last_stop}"
    );
    let (_, text) = engine_cli(&r, &["back", "--id", &early, "--with-data", "--dry-run"]);
    assert!(text.contains("its database was busy"), "{text}");
    let (_, text) = engine_cli(&r, &["back", "--id", &late, "--with-data", "--dry-run"]);
    assert!(!text.contains("busy"), "{text}");
    let (code, text) = engine_cli(&r, &["back", "--id", &late, "--with-data"]);
    assert_eq!(code, 0, "{text}");
    assert_eq!(rows_in(&db), "5", "{text}");
}

/// F4c item 10. A killed app leaves a `-wal` with rows only in it. The deny text names one read; it folds the log into
/// the file and removes `-wal` and `-shm`, the same delete then goes through, and the engine keeps a copy with the rows.
#[test]
fn real_engine_one_read_clears_the_files_a_killed_app_left_and_the_delete_then_goes_through_saved()
{
    for form in exit_forms() {
        exit_scenario("wal", form);
    }
}

/// F4c item 10. An app killed in the middle of a transaction leaves a hot `-journal`. The same read rolls back only the
/// unfinished transaction (the 5 committed rows stay), removes the journal, and the delete then goes through saved.
#[test]
fn real_engine_one_read_rolls_back_a_hot_journal_and_the_delete_then_goes_through_saved() {
    for form in exit_forms() {
        exit_scenario("hot", form);
    }
}

/// F4c item 10, the open limit. A `-journal` that a clean close left behind in PERSIST mode (its header is zeroed)
/// survives the read, so the delete stays denied; the text says to ask the user.
#[test]
fn real_engine_a_persist_journal_a_clean_close_left_survives_the_read_and_the_delete_stays_denied()
{
    for form in exit_forms() {
        exit_scenario("persist", form);
    }
}

/// F4c item 10, the open limit, as above for TRUNCATE mode (the journal file is empty).
#[test]
fn real_engine_a_truncate_journal_a_clean_close_left_survives_the_read_and_the_delete_stays_denied()
{
    for form in exit_forms() {
        exit_scenario("truncate", form);
    }
}

/// F4c item 10. While another program still has the database open (the app runs), the read leaves its log and the
/// retried delete stays denied: the guard is right to refuse it.
#[test]
fn real_engine_the_read_leaves_the_log_of_a_database_an_app_still_has_open_and_the_delete_stays_denied()
 {
    for form in exit_forms() {
        exit_scenario("app", form);
    }
}

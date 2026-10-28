//! Tests of the busy-database guard: the project they run on, which commands are named, which pass, and the read budget.

use super::*;
use std::fs;

mod find;
mod git_clean;
mod words;

/// A project with a busy database (`data/app.db` with a `-wal` that holds rows), one that is idle, one that is
/// not SQLite, a busy one in a deep folder (journal), and a busy one next to the project.
fn project(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("tm-busy-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    let root = d.join("proj");
    for sub in ["data", "sub/deep", "lone"] {
        fs::create_dir_all(root.join(sub)).unwrap();
    }
    fs::create_dir_all(d.join("beside")).unwrap();
    let sqlite = |p: PathBuf| fs::write(p, [MAGIC.as_slice(), &[0u8; 100]].concat()).unwrap();
    sqlite(root.join("data/app.db"));
    fs::write(root.join("data/app.db-wal"), "frames").unwrap();
    fs::write(root.join("data/app.db-shm"), "").unwrap();
    sqlite(root.join("idle.db"));
    fs::write(root.join("idle.db-wal"), "").unwrap(); // an empty wal is not "in use"
    fs::write(root.join("plain.db"), "just text, not SQLite").unwrap();
    fs::write(root.join("plain.db-wal"), "frames").unwrap();
    sqlite(root.join("sub/deep/x.SQLITE"));
    fs::write(root.join("sub/deep/x.SQLITE-journal"), "j").unwrap();
    fs::write(root.join("lone/a.txt"), "a").unwrap();
    fs::write(root.join("lone/app.db"), "not SQLite").unwrap();
    fs::write(root.join("notes.txt"), "n").unwrap();
    sqlite(d.join("beside/other.db"));
    fs::write(d.join("beside/other.db-wal"), "frames").unwrap();
    root
}

fn hits(root: &Path, cmd: &str) -> Vec<String> {
    targets(cmd, root, root)
}

#[test]
fn a_command_that_removes_moves_or_overwrites_a_busy_database_is_named() {
    let root = project("denied");
    let app = "data/app.db".to_string();
    for cmd in [
        "rm -f data/app.db data/app.db-wal data/app.db-shm",
        "rm data/app.db",
        "rm -f data/app.db-wal",
        "rm -f data/app.db-shm",
        "rm -rf data",
        "rm -rf data/",
        "rm -rf ./data/.",
        "rm -f data/*",
        "rm -f data/*.db",
        "rm -f data/app.d?",
        "rm -f data/app.[a-z]b",
        "rm -f data/app.db{,-wal,-shm}",
        "rm -f data/{app,x}.db",
        "rm -f sub/../data/app.db",
        "cd data && rm -f app.db",
        "cd data; rm app.db",
        "cd ./data\nrm app.db",
        "echo clean up; rm data/app.db",
        "ls || rm data/app.db",
        "true & rm data/app.db",
        "sudo rm -f data/app.db",
        "sudo -n rm -f data/app.db",
        "FOO=1 BAR=2 rm data/app.db",
        "env FOO=1 rm data/app.db",
        "nohup rm data/app.db &",
        "if true; then rm data/app.db; fi",
        "bash -c 'rm -f data/app.db'",
        "sh -lc \"cd data && rm app.db\"",
        "rm -- data/app.db",
        "\\rm data/app.db",
        "/bin/rm data/app.db",
        "rm \"data/app.db\"",
        "rm 'data/app.db'",
        "rm -v data/app.db 2>/dev/null",
        "unlink data/app.db",
        "shred -u data/app.db",
        "mv data/app.db /tmp/app.db.old",
        "mv data /tmp/data.old",
        "mv notes.txt data/app.db",
        "mv -f lone/app.db data/",
        "cp lone/app.db data/",
        "cp notes.txt data/app.db",
        "cp -f idle.db data/app.db",
        "install -m 644 notes.txt data/app.db",
        "truncate -s 0 data/app.db",
        "> data/app.db",
        ": > data/app.db",
        "echo x >> data/app.db",
        "cat notes.txt >| data/app.db",
        "echo x &> data/app.db",
        "echo $(rm data/app.db)",
        "echo `rm data/app.db`",
        "find data -name '*.db' -delete",
        "find . -name '*.db' -exec rm {} \\;",
        "find data -type f -exec rm -f {} +",
        "git clean -fdx",
        "git clean -f data",
        "git -C data clean -fd",
        "cd data && git clean -fd",
        "git rm -f data/app.db",
        "git mv data/app.db data/old.db",
    ] {
        let got = hits(&root, cmd);
        assert!(
            got.contains(&app),
            "{cmd:?} must name data/app.db, got {got:?}"
        );
    }
    // a glob over the project and a folder in it also find the deep one
    for cmd in ["rm -rf *", "rm -rf sub", "rm -r ./sub/", "git clean -fd"] {
        assert!(
            hits(&root, cmd).contains(&"sub/deep/x.SQLITE".to_string()),
            "{cmd:?}"
        );
    }
    assert!(
        hits(&root, "rm -f sub/deep/x.SQLITE-journal").contains(&"sub/deep/x.SQLITE".to_string())
    );
    assert_eq!(
        hits(&root, "rm -rf *"),
        vec!["data/app.db".to_string(), "sub/deep/x.SQLITE".to_string()]
    );
    fs::remove_dir_all(root.parent().unwrap()).unwrap();
}

#[test]
fn commands_that_name_no_busy_database_or_only_read_one_pass() {
    let root = project("passed");
    for cmd in [
        "ls data",
        "cat data/app.db > /dev/null",
        "cat data/app.db | head -c 16",
        "sqlite3 data/app.db .dump > dump.sql",
        "head -c 100 data/app.db >/tmp/x.bin",
        "cp data/app.db /tmp/backup.db",
        "cp data/app.db backup.db",
        "mv notes.txt other.txt",
        "mv lone/a.txt lone/b.txt",
        "rm notes.txt",
        "rm -f lone/a.txt",
        "rm -rf lone",
        "rm idle.db idle.db-wal",   // not in use: the engine saves it
        "rm plain.db plain.db-wal", // not SQLite
        "rm -f missing.db",
        "rm data", // a folder without -r: rm refuses
        "rm -f *.txt",
        "rm -f lone/*",
        "rm -rf .hidden*",
        "echo rm data/app.db",
        "echo 'rm -f data/app.db'",
        "grep -r app.db data",
        "git status",
        "git clean -n",
        "git clean -fdn",
        "git clean --dry-run -fd",
        "git -C lone clean -fd",
        "git rm --cached data/app.db",
        "git log -- data/app.db",
        "ls # rm data/app.db",
        "find data -name '*.db'",
        "find data -name '*.db' -print",
        "rm $DB",
        "rm \"$HOME/data/app.db\"",
        "rm ~/data/app.db",
        "rm -f ../beside/other.db", // next to the project, not in it
        "rm -rf ../beside",
        "rm -rf lone && cd data",
        "sqlite3 data/app.db \"DROP TABLE t\"", // a known limit, see the module comment
        "",
        "   ",
        "rm -f 'unterminated",
    ] {
        let got = hits(&root, cmd);
        assert!(got.is_empty(), "{cmd:?} must pass, got {got:?}");
    }
    fs::remove_dir_all(root.parent().unwrap()).unwrap();
}

#[test]
fn a_folder_or_glob_walk_stays_inside_its_budget_and_a_stopped_app_passes() {
    let root = project("budget");
    // a database that stops being busy (the app closed it: no wal left) is not named any more
    assert_eq!(hits(&root, "rm data/app.db"), ["data/app.db"]);
    fs::remove_file(root.join("data/app.db-wal")).unwrap();
    assert!(hits(&root, "rm -f data/app.db data/app.db-shm").is_empty());
    // the workdir decides what a relative word means
    assert_eq!(
        targets("rm -f app.db", &root.join("sub/deep"), &root),
        Vec::<String>::new()
    );
    assert_eq!(
        targets("rm -f x.SQLITE", &root.join("sub/deep"), &root),
        ["sub/deep/x.SQLITE"]
    );
    // a glob over a huge folder reads at most LOOK entries
    let big = root.join("big");
    fs::create_dir_all(&big).unwrap();
    for i in 0..50 {
        fs::write(big.join(format!("f{i}.txt")), "x").unwrap();
    }
    let mut spy = Spy {
        root: root.canonicalize().unwrap(),
        left: 10,
        hits: vec![],
    };
    let got = spy.paths(&root, "big/*", true);
    assert!(
        got.len() <= 10 && spy.left == 0,
        "{} {}",
        got.len(),
        spy.left
    );
    fs::remove_dir_all(root.parent().unwrap()).unwrap();
}

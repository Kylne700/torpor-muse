//! Tests of `git clean`: denied unless git itself says the busy database is left alone.

use super::*;

/// `git init` in the project folder with this `.gitignore`.
fn repo(name: &str, ignore: &str) -> PathBuf {
    let root = project(name);
    let ok = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .status()
        .is_ok_and(|s| s.success());
    assert!(ok, "git init");
    fs::write(root.join(".gitignore"), ignore).unwrap();
    root
}

#[test]
fn git_clean_is_denied_unless_git_itself_says_the_database_is_left_alone() {
    let app = "data/app.db".to_string();
    let deep = "sub/deep/x.SQLITE".to_string();
    // not a repository, no way to ask git: the deny of before
    let none = project("clean-none");
    assert!(hits(&none, "git clean -fd").contains(&app));
    assert!(hits(&none, "git clean -fd -e '*.db*'").contains(&app));
    // every file of both databases ignored
    let all = repo("clean-all", "*.db*\n*.SQLITE*\n");
    for cmd in [
        "git clean -fd",
        "git clean -f data",
        "git clean -fdq",
        "git -C data clean -fd",
        "git clean -fdn",
    ] {
        assert!(hits(&all, cmd).is_empty(), "{cmd:?}");
    }
    for cmd in [
        "git clean -fdx",
        "git clean -ffd",
        "git clean -fdf",
        "git clean -fdX",
        "git clean -f -d --force",
    ] {
        let got = hits(&all, cmd);
        // -x: the rules are not used; -ff: nested repositories go too; -X: only ignored files go
        assert!(
            got.contains(&app) && got.contains(&deep),
            "{cmd:?} -> {got:?}"
        );
    }
    // only the database file is ignored, not its -wal and -shm: git would take those
    let part = repo("clean-part", "*.db\n*.SQLITE\n");
    assert!(hits(&part, "git clean -fd").contains(&app));
    // -e names, in every spelling, also keep the -wal and -shm files
    let plain = repo("clean-e", "");
    for cmd in [
        "git clean -fdx -e '*.db*' -e '*.SQLITE*'",
        "git clean -fdx -e'*.db*' -e'*.SQLITE*'",
        "git clean -fdxe '*.db*' -e '*.SQLITE*'",
        "git clean -fdx --exclude '*.db*' --exclude='*.SQLITE*'",
        "git clean -fd -e '*.db*' -e '*.SQLITE*'",
        "git clean -e '*.db*' -e '*.SQLITE*' -fd",
    ] {
        let got = hits(&plain, cmd);
        assert!(got.is_empty(), "{cmd:?} -> {got:?}");
    }
    for cmd in [
        "git clean -fdx -e dev.db",
        "git clean -fdx -e '*.db' -e '*.SQLITE'",
        "git clean -fdx -e '*.db*'", // the deep one is not excluded
        "git clean -fd",
        "git clean -fdx",
    ] {
        assert!(!hits(&plain, cmd).is_empty(), "{cmd:?}");
    }
    assert!(hits(&plain, "git clean -fdx -e '*.db*'").contains(&deep));
    // -X takes only what the rules ignore: nothing is ignored here
    assert!(hits(&plain, "git clean -fdX").is_empty());
    assert!(hits(&all, "git clean -fdX -e zzz").contains(&app));
    // an -e pattern is not a path to clean
    assert!(hits(&plain, "git clean -fd -e '*.db*' -e '*.SQLITE*' notes.txt").is_empty());
    assert!(hits(&plain, "git clean -fd notes.txt data").contains(&app));
}

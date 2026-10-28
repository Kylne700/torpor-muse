//! Tests of the outside guard: which deletes and moves are stopped, on a user's computer and in the L4 sandbox, how
//! the reason names them, and the scan of text written to a file.

use super::*;
use place::usable_tmp;

/// A user at /Users/u working in /Users/u/proj, no TMPDIR.
fn desk() -> Env {
    Env {
        root: vec!["/Users/u/proj".into()],
        home: Some("/Users/u".into()),
        tmp: None,
        store: Some("/Users/u/.torpor".into()),
    }
}

/// The L4 sandbox (scripts/sandbox/muse-sandbox.sh): HOME (`h` here) and TMPDIR in its run folder under /tmp, the
/// project in TMPDIR.
fn sandbox() -> Env {
    let home = PathBuf::from("/tmp/s/h");
    Env {
        root: vec!["/tmp/s/tmp/drill.1/ws".into()],
        tmp: usable_tmp("/tmp/s/tmp".into(), Some(&home)),
        store: Some(home.join(".torpor")),
        home: Some(home),
    }
}

/// What the reason names for `cmd` run in the project folder.
fn hits(e: &Env, cmd: &str) -> Vec<String> {
    stopped(cmd, e.root[0].clone(), e)
}

fn stops(e: &Env, cmd: &str) -> bool {
    !hits(e, cmd).is_empty()
}

#[test]
fn each_verb_is_stopped_outside_the_project_and_left_alone_inside() {
    let e = desk();
    for verb in [
        "rm -rf",
        "rm",
        "rmdir",
        "unlink",
        "shred -u",
        "truncate -s 0",
    ] {
        let outside = format!("{verb} /Users/u/notes.txt");
        assert_eq!(hits(&e, &outside), ["/Users/u/notes.txt"], "{verb}");
        assert!(!stops(&e, &format!("{verb} notes.txt")), "{verb}");
        assert!(
            !stops(&e, &format!("{verb} /Users/u/proj/sub/notes.txt")),
            "{verb}"
        );
    }
    // `mv`: what it moves counts, not where it goes
    assert_eq!(hits(&e, "mv ~/notes.txt docs/"), ["/Users/u/notes.txt"]);
    assert_eq!(hits(&e, "mv -t docs ~/a ~/b"), ["/Users/u/a", "/Users/u/b"]);
    assert!(!stops(&e, "mv notes.txt ~/backup/"));
    // `dd` writes over `of=`
    assert!(stops(
        &e,
        "dd if=/dev/zero of=/Users/u/disk.img bs=1M count=1"
    ));
    assert!(!stops(&e, "dd if=/dev/zero of=out.img count=1"));
    assert!(!stops(&e, "dd if=/Users/u/disk.img of=/dev/null"));
    // `truncate`'s size and reference are no files it empties
    assert!(!stops(&e, "truncate --reference /Users/u/x app.log"));
    // `find` removes what it finds below where it starts
    assert_eq!(
        hits(&e, "find ~/Downloads -name '*.tmp' -delete"),
        ["/Users/u/Downloads/*"]
    );
    assert!(stops(&e, "find /Users/u/other -type f -exec rm -f {} +"));
    assert!(stops(&e, "find .. -name x -execdir mv {} /tmp \\;"));
    assert!(!stops(&e, "find ~ -name '*.log'"));
    assert!(!stops(&e, "find . -name '*.o' -delete"));
    assert!(!stops(&e, "find /tmp/t -delete"));
    // `xargs rm` removes what the command in front prints, when that can be told
    assert!(stops(&e, "find ~/Downloads -name '*.tmp' | xargs rm -f"));
    assert!(stops(&e, "echo ~/old | xargs rm -rf"));
    assert!(stops(&e, "xargs -n 1 rm -rf /Users/u/x < list.txt"));
    assert!(!stops(&e, "find . -name '*.tmp' | xargs rm -f"));
    assert!(!stops(&e, "git ls-files -z | xargs -0 rm -f"));
    assert!(!stops(&e, "echo ~/old | xargs ls"));
}

#[test]
fn shells_eval_wrappers_and_an_earlier_cd_are_followed() {
    let e = desk();
    for cmd in [
        "sh -c 'rm -rf ~/old'",
        "bash -lc \"cd .. && rm -rf other\"",
        "eval 'rm -rf $HOME/x'",
        "cd /Users/u && rm -rf old",
        "cd; rm -rf old",
        "cd ..; rm -rf other-project",
        "make build && rm -rf ../other",
        "cargo build | tee log; rm -rf ~/x",
        "sudo rm -rf ~/x",
        "env X=1 nice rm -rf ~/x",
    ] {
        assert!(stops(&e, cmd), "{cmd}");
    }
    for cmd in [
        "cd sub && rm -rf build",
        "cd ~ && rm -rf proj/build",
        "cd /tmp && rm -rf build-1",
        "cd \"$X\" && rm -rf build", // the folder cannot be told: the engine decides
        "cd - && rm -rf build",
        "bash -c 'rm -rf dist'",
        "sh -c 'cd /tmp && rm -rf x' && rm -rf dist", // the `cd` stays in its shell
        "bash scripts/clean.sh",
    ] {
        assert!(!stops(&e, cmd), "{cmd}");
    }
}

#[test]
fn the_home_folder_is_known_in_each_spelling() {
    let e = desk();
    for w in [
        "~",
        "~/",
        "$HOME",
        "${HOME}",
        "\"$HOME\"",
        "~/old-notes",
        "$HOME/old-notes",
        "${HOME}/old-notes",
        "\"$HOME/My Notes\"",
        "/Users/u/proj/../old",
    ] {
        assert!(stops(&e, &format!("rm -rf {w}")), "{w}");
    }
    assert_eq!(hits(&e, "rm -rf ~"), ["/Users/u"]);
    // another variable, another user's home: cannot be told
    for w in ["$HOMER/x", "~user/x", "$X", "$X/y"] {
        assert!(!stops(&e, &format!("rm -rf {w}")), "{w}");
    }
}

#[test]
fn temp_folders_are_allowed_but_not_a_temp_folder_itself() {
    let e = desk();
    for cmd in [
        "rm -rf /tmp/build-1",
        "rm -rf /var/tmp/cache",
        "mv /tmp/out.txt .",
        "rm -rf /tmp/x*",
        "rm -f /tmp/$X",
        "rm -rf /tmp/*",
    ] {
        assert!(!stops(&e, cmd), "{cmd}");
    }
    for cmd in ["rm -rf /tmp", "rm -rf /var/tmp/", "rm -rf /tmp/.."] {
        assert!(stops(&e, cmd), "{cmd}");
    }
    // $TMPDIR counts unless it is `/`, the home folder or a folder that holds it
    let home = Path::new("/tmp/s/h");
    assert_eq!(
        usable_tmp("/tmp/s/tmp".into(), Some(home)),
        Some("/tmp/s/tmp".into())
    );
    assert!(usable_tmp("/tmp/s/h/tmp".into(), Some(home)).is_some());
    for bad in ["/", "/tmp", "/tmp/s", "/tmp/s/h"] {
        assert_eq!(usable_tmp(bad.into(), Some(home)), None, "{bad}");
    }
    let e = Env {
        tmp: Some("/scratch/t".into()),
        ..desk()
    };
    assert!(!stops(&e, "rm -rf /scratch/t/x"));
    assert!(stops(&e, "rm -rf /scratch/t"));
    assert!(stops(&e, "rm -rf /scratch/other"));
}

/// Lead decision for stage O: in the L4 sandbox HOME and the project lie under /tmp, and the home folder still counts.
#[test]
fn in_the_sandbox_the_home_folder_under_tmp_is_stopped() {
    let e = sandbox();
    assert!(e.tmp.is_some());
    assert_eq!(hits(&e, "rm -rf $HOME/old-notes"), ["/tmp/s/h/old-notes"]);
    assert_eq!(hits(&e, "rm -rf ~/old-notes"), ["/tmp/s/h/old-notes"]);
    assert!(!stops(&e, "rm -rf /tmp/build-1"));
    assert!(!stops(&e, "rm -rf $TMPDIR/x"));
    assert!(!stops(&e, "rm -rf /tmp/s/tmp/x"));
    assert!(!stops(&e, "rm -f /tmp/*.sock")); // no name it matches holds the home folder
    assert!(!stops(&e, "rm -rf build ../ws/dist"));
    for cmd in [
        "rm -rf /tmp/s",
        "rm -rf /tmp/*",
        "rm -rf /tmp/s*",
        "rm -rf ~/.torpor/x",
        "find /tmp -name '*.log' -delete", // reaches into the home folder
    ] {
        assert!(stops(&e, cmd), "{cmd}");
    }
}

#[test]
fn a_target_cut_short_is_stopped_only_when_its_fixed_start_is_outside() {
    let e = desk();
    for (cmd, shown) in [
        ("rm -rf ~/$X", "/Users/u/*"), // could be the store
        ("rm -rf /opt/$X", "/opt/*"),
        ("rm -rf ~/*", "/Users/u/*"),
        ("rm -rf /*", "/*"),
        ("rm -rf /Users/u/pro*", "/Users/u/pro*"),
        ("rm -rf ~/.cache/*", "/Users/u/.cache/*"),
        ("rm -rf /Users/*/old", "/Users/*/old"),
    ] {
        assert_eq!(hits(&e, cmd), [shown], "{cmd}");
    }
    for cmd in [
        "rm -rf $X",
        "rm -rf $X/y",
        "rm -rf \"$(pwd)/build\"",
        "rm -rf `pwd`/x",
        "rm -rf *.log",
        "rm -rf build-*",
        "rm -rf ./dist/*",
        "rm -rf src/{a,b}.rs",
    ] {
        assert!(!stops(&e, cmd), "{cmd}");
    }
}

/// Rule 2 comes before rule 1: a project that is the home folder (Muse started in `~`) still keeps it and the store.
#[test]
fn the_home_folder_and_the_store_are_stopped_from_a_project_that_holds_them() {
    let e = Env {
        root: vec!["/Users/u".into()],
        ..desk()
    };
    for cmd in ["rm -rf ~", "rm -rf .", "rm -rf .torpor", "rm -rf ./.*"] {
        assert!(stops(&e, cmd), "{cmd}");
    }
    assert!(!stops(&e, "rm -rf ./*")); // `*` does not name `.torpor`
    assert!(!stops(&e, "rm -rf notes"));
}

/// research/71c probes C01-C09.
#[test]
fn the_71c_probes() {
    let e = desk();
    assert_eq!(hits(&e, "rm -rf tests/ patches/ plan/ ~/"), ["/Users/u"]); // C01
    assert_eq!(hits(&e, "rm -rf ~"), ["/Users/u"]); // C02
    assert_eq!(hits(&e, "rm -rf $HOME/projects"), ["/Users/u/projects"]); // C03
    assert_eq!(
        hits(&e, "rm -rf ../other-project"),
        ["/Users/u/other-project"]
    ); // C04
    assert_eq!(hits(&e, "bash -c \"rm -rf ~\""), ["/Users/u"]); // C05
    // C06: `npm run clean`, whose script is `rm -rf $HOME/old-stuff`
    let npm = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/outside/npm");
    let n = Env {
        root: vec![npm.clone()],
        ..desk()
    };
    assert_eq!(
        stopped("npm run clean", npm.clone(), &n),
        ["/Users/u/old-stuff"]
    );
    assert_eq!(
        stopped("pnpm run-script clean", npm.clone(), &n),
        ["/Users/u/old-stuff"]
    );
    assert!(stopped("npm run build", npm.clone(), &n).is_empty());
    assert!(stopped("npm test", npm, &n).is_empty());
    // C07, C08: a script run by name and a program are not read (reader.rs shortcut): the engine decides
    assert!(!stops(&e, "./clean.sh"));
    assert!(!stops(&e, "cargo test"));
    // C09: text written to a file
    let c09 = text::find("fn t(){ run(\"20; rm -rf ~\"); }");
    assert_eq!(c09, Some(("rm -rf ~".into(), true)));
}

#[test]
fn text_that_deletes_the_home_folder_or_root_is_found() {
    for (text, cmd, home) in [
        ("\"20; rm -rf ~\"", "rm -rf ~", true), // incident #105342
        ("os.system('rm -rf ~/')", "rm -rf ~/", true),
        ("x && /bin/rm -r ${HOME}", "rm -r ${HOME}", true),
        ("rm -rf \"$HOME\"/", "rm -rf $HOME", true),
        (
            "sudo rm -rf --no-preserve-root /",
            "rm -rf --no-preserve-root /",
            false,
        ),
        ("exec(\"rm -fr /*\")", "rm -fr /*", false),
    ] {
        assert_eq!(text::find(text), Some((cmd.into(), home)), "{text}");
    }
    for text in [
        "rm -rf ~/proj/build",
        "rm -rf /tmp/x",
        "rm -rf $HOME/.cache/app",
        "echo rm -rf ~",
        "# rm -rf ~ would be bad",
        "firm -rf ~",
        "let home = \"~\";",
    ] {
        assert_eq!(text::find(text), None, "{text}");
    }
}

#[test]
fn the_ordinary_commands_are_not_stopped() {
    let list = include_str!("../../../tests/fixtures/outside/ordinary.txt");
    let cmds: Vec<&str> = list
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    assert_eq!(cmds.len(), 200);
    let stopped_in =
        |e: Env| -> Vec<&str> { cmds.iter().copied().filter(|c| stops(&e, c)).collect() };
    assert_eq!(stopped_in(desk()), Vec::<&str>::new());
    // in the sandbox the home folder lies under /tmp, and `find /tmp ... -delete` could reach it
    let reach = ["find /tmp -maxdepth 1 -name 'npm-*' -mtime +1 -delete"];
    assert_eq!(stopped_in(sandbox()), reach);
}

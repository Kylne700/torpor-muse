//! Tests of how a command line is read: globs, words, braces, a target directory and here-documents.

use super::*;
use crate::hook::busy::args::{is_assign, target_dir};
use crate::hook::busy::glob::{expand, fnmatch, glob_match};
use crate::hook::busy::lexer::{Tok, lex};

#[test]
fn globs_and_words_are_read_like_a_shell_does() {
    for (pat, name, yes) in [
        ("*.db", "app.db", true),
        ("*.db", ".hidden.db", false),
        (".*", ".hidden", true),
        ("app.d?", "app.db", true),
        ("app.[a-c]b", "app.db", false),
        ("app.[!a-c]b", "app.db", true),
        ("a*b*c", "aXbYc", true),
        ("a*b*c", "aXbY", false),
        ("[", "[", true),
        ("a*", "a", true),
        ("*", "", true),
        ("?", "", false),
        ("a*b?c*", "aXXb1cYY", true),
    ] {
        assert_eq!(glob_match(pat, name), yes, "{pat} vs {name}");
    }
    // several stars on a name that cannot match must not take forever
    assert!(!glob_match("a*a*a*a*a*a*a*b", &"a".repeat(60)));
    assert!(fnmatch("*.db", ".hidden.db")); // find's -name, unlike the shell, lets a star match a leading dot
    let words = |s: &str| {
        lex(s, true)
            .into_iter()
            .filter_map(|t| match t {
                Tok::Word(w, g) => Some((w, g)),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        words(r#"rm "a b" 'c*' d\ e *.f # note"#),
        [
            ("rm", false),
            ("a b", false),
            ("c*", false),
            ("d e", false),
            ("*.f", true)
        ]
        .map(|(w, g)| (w.to_string(), g))
    );
    assert!(is_assign("A_1=x") && !is_assign("=x") && !is_assign("1A=x") && !is_assign("a-b=1"));
}

#[test]
fn braces_expand_like_a_shell_and_stay_bounded() {
    assert_eq!(
        expand("app.db{,-wal,-shm}"),
        ["app.db", "app.db-wal", "app.db-shm"]
    );
    assert_eq!(expand("{a,{b,c}}.db"), ["a.db", "b.db", "c.db"]);
    assert_eq!(expand("{a,b}{c,d}"), ["ac", "ad", "bc", "bd"]);
    assert_eq!(expand("plain"), ["plain"]);
    assert_eq!(expand("x{y}z"), ["x*z"]); // no comma: not an expansion, read as a star
    assert_eq!(expand("x{y,z"), ["x*"]); // never closed
    assert!(expand(&"{a,b}".repeat(12)).len() < 300);
    let root = project("braces");
    let app = "data/app.db".to_string();
    assert!(hits(&root, "rm data/app.{log,txt}").is_empty());
    assert!(hits(&root, "rm data/app.{log,db-wal}").contains(&app));
    assert!(hits(&root, "rm data/{idle,app}.db").contains(&app));
}

#[test]
fn a_target_directory_is_where_the_files_go_and_only_what_is_there_is_overwritten() {
    let root = project("target");
    let app = "data/app.db".to_string();
    // lone/app.db is not SQLite and notes.txt is not in data: nothing in use is overwritten
    for cmd in [
        "cp -t lone data/app.db",
        "cp -t lone/ data/app.db",
        "cp -t backup data/app.db",
        "cp -t data notes.txt",
        "cp --target-directory=data notes.txt",
        "install -m 644 -t data notes.txt",
    ] {
        assert!(hits(&root, cmd).is_empty(), "{cmd:?}");
    }
    for cmd in [
        "cp -t data lone/app.db",
        "cp -tdata lone/app.db",
        "cp -rt data lone/app.db",
        "cp -at data lone/app.db",
        "cp lone/app.db -t data",
        "cp --target-directory=data lone/app.db",
        "cp --target-directory data lone/app.db",
        "cp --target data lone/app.db",
        "cp -v -- lone/app.db data/",
        "mv -t data lone/app.db",
        "mv -t lone data/app.db", // what mv moves away counts, as without -t
        "install -t data lone/app.db",
    ] {
        assert!(hits(&root, cmd).contains(&app), "{cmd:?}");
    }
    let w = |s: &str| (s.to_string(), false);
    assert_eq!(
        target_dir(&[w("-rt"), w("d"), w("a")]),
        (Some(w("d")), vec![w("a")])
    );
    assert_eq!(target_dir(&[w("a"), w("b")]), (None, vec![w("a"), w("b")]));
}

#[test]
fn here_document_text_is_not_a_command_unless_a_shell_or_script_reads_it() {
    let root = project("heredoc");
    let app = "data/app.db".to_string();
    for cmd in [
        "cat > reset.sh <<'EOF'\nrm data/app.db\nEOF",
        "cat > reset.sh <<EOF\nrm data/app.db\nEOF\nchmod +x reset.sh",
        "cat <<-EOF\n\trm data/app.db\n\tEOF\necho done",
        "cat <<A <<B\nrm data/app.db\nA\nrm data/app.db\nB",
        "cat <<EOF > notes.txt\nrm data/app.db\nEOF",
        "python3 <<EOF\nrm data/app.db\nEOF", // shortcut: not shell, so not read
        "grep x <<< 'rm data/app.db'",
    ] {
        assert!(hits(&root, cmd).is_empty(), "{cmd:?}");
    }
    for cmd in [
        "cat <<EOF\nhello\nEOF\nrm data/app.db",
        "bash <<EOF\nrm data/app.db\nEOF",
        "sh <<'EOF'\nrm data/app.db\nEOF",
        "sudo bash <<EOF\nrm data/app.db\nEOF",
        "cat <<EOF | sh\nrm data/app.db\nEOF",
        "cat > x.sh <<EOF\nrm data/app.db\nEOF\nsh x.sh",
        "cat > x.sh <<EOF\nrm data/app.db\nEOF\n./x.sh",
        "cat > x.sh <<EOF\nrm data/app.db\nEOF\nbash scripts/x.sh",
        "source /dev/stdin <<EOF\nrm data/app.db\nEOF",
        "cat <<EOF\nrm data/app.db\n", // no end word: not a here-document
        "echo $((1 << 2))\nrm data/app.db",
        "cat <<EOF\nrm data/app.db\nEOFX\n",
    ] {
        assert!(hits(&root, cmd).contains(&app), "{cmd:?}");
    }
}

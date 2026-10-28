//! Tests of `find ... -delete` and `-exec rm`: which `-name` and `-type` tests can reach a busy database.

use super::*;

#[test]
fn find_tests_that_cannot_reach_a_database_let_the_command_through() {
    let root = project("find");
    let (app, deep) = ("data/app.db".to_string(), "sub/deep/x.SQLITE".to_string());
    for cmd in [
        "find . -name .DS_Store -delete",
        "find . -name '*.pyc' -delete",
        "find data -iname '*.PYC' -delete",
        "find . -type f -name '*.orig' -exec rm {} +",
        "find . -type d -name __pycache__ -exec rm -rf {} +",
        "find . -type d -name zzz -exec mv {} /tmp \\;",
        "find ./ -type d -name zzz -exec rm -rf {} +",
        "find . -type d -delete", // a database is a file: rmdir of a folder that holds one fails
        "find . -type l -delete",
        "find . -name '*.db' -type d -delete",
        "find . -name 'app.dbx' -delete",
        "find data -name 'app.db' -type d -exec rm -rf {} +",
        "find . -maxdepth 1 -name '*.log' -delete",
        "find . -name 'app.d[a]' -delete",
        "find -L . -name zzz -delete",
    ] {
        assert!(
            hits(&root, cmd).is_empty(),
            "{cmd:?} -> {:?}",
            hits(&root, cmd)
        );
    }
    // a name test is on the last word of a path, case as asked, and it sees the -wal, -shm and -journal files too
    for cmd in [
        "find . -name '*.db' -delete",
        "find . -name 'app.db' -delete",
        "find . -iname 'APP.DB' -delete",
        "find . -name 'app.d?' -type f -delete",
        "find . -name 'app.db-wal' -delete",
        "find . -name '*-shm' -exec rm {} +",
        "find . -name '*.db' -a -type f -delete",
        "find data -type f -delete",
        "find data -delete",
        "find . -delete -name '*.pyc'", // the action runs before the test
        "find . -name '[a]pp.db' -delete",
        "find . -name 'app\\.db' -delete", // a pattern we cannot read counts as matching
        "find . -name \"$X\" -delete",
    ] {
        assert!(hits(&root, cmd).contains(&app), "{cmd:?}");
    }
    assert!(hits(&root, "find . -name 'x.SQLITE' -iname 'X.*' -delete").contains(&deep));
    assert!(!hits(&root, "find . -name 'x.sqlite' -delete").contains(&deep)); // -name is case sensitive
    // tests this parser does not judge: as if there were none (the common `-prune -o` idiom stays a false alarm)
    for cmd in [
        "find . -name '*.pyc' -o -name '*.db' -delete",
        "find . ! -name '*.db' -delete",
        "find . -not -name zzz -delete",
        "find . \\( -name a -o -name b \\) -delete",
        "find . -name node_modules -prune -o -name '*.pyc' -delete",
        "find . -path './data/*' -delete",
        "find . -regex '.*app.*' -delete",
        "find . -name zzz -exec echo {} \\; -delete",
    ] {
        assert!(hits(&root, cmd).contains(&app), "{cmd:?}");
    }
    // a folder test with rm -rf / mv takes the database along with the folder it is in
    assert_eq!(
        hits(&root, "find . -type d -name data -exec rm -rf {} +"),
        std::slice::from_ref(&app)
    );
    assert_eq!(
        hits(&root, "find . -type d -name deep -exec rm -rf {} +"),
        std::slice::from_ref(&deep)
    );
    assert_eq!(
        hits(&root, "find . -name sub -exec mv {} /tmp/x \\;"),
        std::slice::from_ref(&deep)
    );
    assert!(hits(&root, "find . -type d -exec rm -rf {} +").contains(&app));
    // a start point that holds the project folder, other than `.`, is not judged
    for start in [
        root.display().to_string(),
        "..".to_string(),
        "../proj".to_string(),
    ] {
        let got = hits(
            &root,
            &format!("find {start} -type d -name zzz -exec rm -rf {{}} +"),
        );
        assert!(got.contains(&app), "{start} -> {got:?}");
    }
}

//! The programs the brief names as a test run or a build, by their subcommands and goals. `None` for a program or a
//! subcommand it does not name: `segment::classify` then guesses from the words of the command.

use super::segment::{BUILD, BUILD_TEST, EITHER, Hit, NONE, TEST, TEST_BUILD};

/// Package-manager and builder subcommands that change or read the project but do not build it.
#[rustfmt::skip]
const QUIET: &[&str] = &[
    "fmt", "format", "tree", "metadata", "add", "remove", "rm", "update", "upgrade", "fetch", "clean", "new", "init",
    "version", "help", "tidy", "mod", "get", "env", "doc", "list", "restore", "tool", "nuget", "tasks", "dependencies",
    "projects", "properties", "login", "logout", "search", "outdated", "vendor", "work", "generate-lockfile",
    "install-hooks",
];
#[rustfmt::skip]
const HOUSEKEEPING: &[&str] = &[
    "ci", "install", "i", "add", "remove", "rm", "uninstall", "update", "upgrade", "up", "outdated", "audit", "ls",
    "list", "info", "view", "init", "link", "pack", "publish", "cache", "config", "set", "get", "version", "why", "prune",
    "dedupe", "login", "logout", "whoami", "help", "start", "dev", "serve", "preview",
];
/// Flags that take their value in the next word, so that it is no task name.
const NPM_VALUED: &[&str] = &[
    "--prefix",
    "-C",
    "--cwd",
    "--filter",
    "-F",
    "-w",
    "--workspace",
];
const MAVEN_VALUED: &[&str] = &[
    "-pl",
    "-f",
    "-T",
    "-s",
    "-rf",
    "-P",
    "--projects",
    "--file",
    "--threads",
];
#[rustfmt::skip]
const GRADLE_VALUED: &[&str] = &[
    "-x", "--exclude-task", "-p", "--project-dir", "-b", "-c", "--build-file", "--settings-file", "--tests",
];
const MAKE_VALUED: &[&str] = &[
    "-C",
    "-f",
    "-j",
    "-l",
    "-I",
    "-o",
    "-W",
    "--directory",
    "--file",
    "--jobs",
];

/// The words that are not flags, without the values of the flags in `valued`, and without `NAME=value` words.
fn tasks<'a>(args: &'a [String], valued: &[&str]) -> Vec<&'a str> {
    let (mut out, mut skip) = (Vec::new(), false);
    for a in args {
        if std::mem::take(&mut skip) {
            continue;
        }
        if a.starts_with('-') {
            skip = valued.contains(&a.as_str());
        } else if !a.contains('=') {
            out.push(a.as_str());
        }
    }
    out
}

fn merge(a: Hit, b: Hit) -> Hit {
    Hit {
        test: a.test.max(b.test),
        build: a.build.max(b.build),
        trivial: false,
    }
}

/// `via`: a package launcher (`npx`) started the program.
pub(super) fn known(cmd: &str, args: &[String], via: bool) -> Option<Hit> {
    let has = |x: &str| args.iter().any(|a| a == x);
    let a0 = args.first().map(String::as_str);
    Some(match cmd {
        "npm" | "yarn" | "pnpm" | "bun" => return scripts(cmd, args),
        "jest" | "vitest" | "mocha" | "ava" | "pytest" | "py.test" | "rspec" | "phpunit"
        | "ctest" | "tox" | "nox" => TEST,
        "tap" if via => TEST,
        "playwright" if a0 == Some("test") => TEST,
        "cypress" if a0 == Some("run") => TEST,
        "deno" if a0 == Some("test") => TEST,
        "rake" if has("test") => TEST,
        "node"
            if args
                .iter()
                .any(|a| a == "--test" || a.starts_with("--test-")) =>
        {
            TEST
        }
        c if c.starts_with("python") || c == "py" => match (a0, args.get(1).map(String::as_str)) {
            (Some("-m"), Some("pytest" | "unittest" | "tox" | "nox")) => TEST,
            _ => return None,
        },
        "tsc" if has("-v") || has("-w") => NONE,
        "tsc" => BUILD,
        "vite" | "next" if a0 == Some("build") => BUILD,
        "webpack"
            if !args
                .iter()
                .any(|a| matches!(a.as_str(), "serve" | "server" | "-w")) =>
        {
            BUILD
        }
        "cargo" => return cargo(args),
        "go" => return go(args),
        "mvn" | "mvnw" => return maven(args),
        "gradle" | "gradlew" => return gradle(args),
        "dotnet" => match tasks(args, &[]).first().copied()? {
            "test" => TEST_BUILD,
            "build" => BUILD,
            s if QUIET.contains(&s) => NONE,
            _ => return None,
        },
        "make" | "gmake" => return make(args),
        _ => return None,
    })
}

/// npm, yarn, pnpm and bun: `test`, `run test:unit`, `run build`, `yarn build`, `bun build`.
fn scripts(name: &str, args: &[String]) -> Option<Hit> {
    let words = tasks(args, NPM_VALUED);
    let mut it = words.iter().copied();
    let sub = it.next()?;
    let script = |s: &str| {
        if args.iter().any(|a| a == "--if-present") {
            Some(EITHER)
        } else if s.ends_with("watch") {
            Some(NONE)
        } else if s.starts_with("test") {
            Some(TEST)
        } else if s == "build" {
            Some(BUILD)
        } else {
            None
        }
    };
    match sub {
        "test" | "t" | "tst" => Some(TEST),
        "run" | "run-script" | "rum" | "urn" => it.next().map_or(Some(NONE), script),
        s if HOUSEKEEPING.contains(&s) => Some(NONE),
        // npm has no other bare script word than `build`
        s if name != "npm" || s == "build" => script(s),
        _ => None,
    }
}

fn cargo(args: &[String]) -> Option<Hit> {
    let mut it = args
        .iter()
        .map(String::as_str)
        .filter(|a| !a.starts_with('+') && !a.starts_with('-'));
    match it.next()? {
        "test" | "t" => Some(TEST_BUILD),
        "nextest" => matches!(it.next(), Some("run" | "r")).then_some(TEST_BUILD),
        "build" | "b" | "check" | "c" => Some(BUILD),
        s if QUIET.contains(&s) => Some(NONE),
        _ => None,
    }
}

fn go(args: &[String]) -> Option<Hit> {
    match tasks(args, &[]).first().copied()? {
        "test" => Some(TEST_BUILD),
        "build" | "vet" => Some(BUILD),
        s if QUIET.contains(&s) => Some(NONE),
        _ => None,
    }
}

fn maven(args: &[String]) -> Option<Hit> {
    let goals = tasks(args, MAVEN_VALUED);
    let one = |g: &str| match g {
        "test" | "verify" | "integration-test" => TEST_BUILD,
        "compile" | "test-compile" => BUILD,
        "package" => BUILD_TEST,
        "install" | "deploy" => EITHER,
        _ => NONE,
    };
    let hit = goals.iter().map(|g| one(g)).fold(NONE, merge);
    (hit != NONE || goals.iter().all(|g| *g == "clean")).then_some(hit)
}

fn gradle(args: &[String]) -> Option<Hit> {
    let names = tasks(args, GRADLE_VALUED);
    let one = |t: &str| match t.rsplit(':').next().unwrap_or(t) {
        "test" | "check" => TEST_BUILD,
        "build" => BUILD_TEST,
        s if QUIET.contains(&s) || s == "wrapper" => NONE,
        _ => EITHER,
    };
    (!names.is_empty()).then(|| names.iter().map(|t| one(t)).fold(NONE, merge))
}

fn make(args: &[String]) -> Option<Hit> {
    if args.iter().any(|a| {
        matches!(
            a.as_str(),
            "-n" | "-p" | "-q" | "--just-print" | "--dry-run" | "--question"
        )
    }) {
        return Some(NONE);
    }
    let targets = tasks(args, MAKE_VALUED);
    if targets.is_empty() {
        return Some(BUILD_TEST);
    }
    let one = |t: &str| match t {
        "test" | "tests" | "check" => TEST_BUILD,
        "build" | "all" => BUILD_TEST,
        s if QUIET.contains(&s) || s == "distclean" => NONE,
        _ => EITHER,
    };
    Some(targets.iter().map(|t| one(t)).fold(NONE, merge))
}

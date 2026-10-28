//! One command of a line (`cd app && npm test` has two): a test run or a build by the lists in the brief (`programs`), or
//! one that may be (an unknown runner, a script, a tool that compiles on the way).

use super::programs;
use super::split::Seg;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub(super) enum Mark {
    #[default]
    No,
    /// Not on the lists, but it may be one.
    Maybe,
    Listed,
}
use Mark::{Listed, Maybe, No};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(super) struct Hit {
    pub(super) test: Mark,
    pub(super) build: Mark,
    /// `cd`, `export`, `echo` and the like: they do not hide the failure of the command after them.
    pub(super) trivial: bool,
}

const fn mk(test: Mark, build: Mark) -> Hit {
    Hit {
        test,
        build,
        trivial: false,
    }
}
pub(super) const NONE: Hit = mk(No, No);
pub(super) const TEST: Hit = mk(Listed, No);
pub(super) const BUILD: Hit = mk(No, Listed);
/// A test run compiles first, and its exit code cannot tell a compile error from a failing test.
pub(super) const TEST_BUILD: Hit = mk(Listed, Maybe);
/// A build that may run the tests too (`mvn package`, `gradle build`, a bare `make`).
pub(super) const BUILD_TEST: Hit = mk(Maybe, Listed);
pub(super) const EITHER: Hit = mk(Maybe, Maybe);

#[rustfmt::skip]
const WRAP: &[&str] = &[
    "time", "nohup", "exec", "sudo", "env", "nice", "xvfb-run", "stdbuf", "ionice", "caffeinate", "unbuffer", "builtin",
    "timeout",
];
/// `command -v pytest` only looks the program up.
#[rustfmt::skip]
const TRIVIAL: &[&str] = &[
    "cd", "export", "echo", "printf", "source", ".", "set", "true", "pwd", "unset", ":", "pushd", "popd", "umask",
    "ulimit", "alias", "command", "hash",
];
/// Commands that neither run tests nor build.
#[rustfmt::skip]
const READ_ONLY: &[&str] = &[
    "cat", "ls", "rg", "grep", "egrep", "fgrep", "head", "tail", "sed", "awk", "fd", "find", "wc", "stat", "file",
    "tree", "git", "gh", "mkdir", "touch", "rm", "mv", "cp", "ln", "chmod", "chown", "diff", "sort", "uniq", "cut",
    "tr", "curl", "wget", "jq", "less", "more", "which", "type", "false", "sleep", "date", "whoami", "ps", "kill",
    "tee", "readlink", "realpath", "basename", "dirname", "du", "df", "tar", "zip", "unzip", "base64", "scp",
    "man", "uname", "test", "[", "wait",
];
/// Words in a command that mean it does not run anything (or never ends).
#[rustfmt::skip]
const DRY: &[&str] = &[
    "--help", "-h", "--version", "--no-run", "--collect-only", "--co", "--list", "--listTests", "--list-tests",
    "--dry-run", "--watch", "--watchAll", "watch", "--init", "--showConfig", "--markers", "--fixtures",
];
/// Programs that build, whatever the subcommand.
#[rustfmt::skip]
const BUILDERS: &[&str] = &[
    "cargo", "go", "make", "gmake", "cmake", "ninja", "mvn", "mvnw", "gradle", "gradlew", "dotnet", "tsc", "bazel",
    "webpack", "vite", "rollup", "esbuild", "next", "rake", "msbuild", "swift", "zig", "gcc", "g++", "clang", "cc",
    "rustc", "javac", "ant", "sbt", "mix", "stack", "cabal", "nuxt", "parcel", "turbo",
];
/// Words, besides the ones with `test` in them, that point to a test run ...
#[rustfmt::skip]
const TESTISH: &[&str] = &[
    "spec", "specs", "e2e", "verify", "check", "checks", "ci", "suite", "coverage", "jest", "vitest", "mocha", "ava",
    "tap", "karma", "jasmine", "junit", "behave", "cucumber", "tox", "nox", "cypress", "playwright", "rspec", "phpunit",
];
/// ... and, besides the ones with `build` and `compil` in them, to a build.
#[rustfmt::skip]
const BUILDISH: &[&str] = &[
    "dist", "bundle", "package", "release", "assemble", "jar", "typecheck", "check", "lint", "mypy", "pyright",
];

fn assign(w: &str) -> bool {
    w.split_once('=').is_some_and(|(k, _)| {
        k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !k.is_empty()
            && !k.starts_with(|c: char| c.is_ascii_digit())
    })
}

/// `./gradlew`, `.venv/bin/pytest` and `gradlew.bat` are `gradlew` and `pytest`.
fn base(w: &str) -> &str {
    let n = w.rsplit('/').next().unwrap_or(w);
    [".exe", ".cmd", ".bat"]
        .iter()
        .find_map(|x| n.strip_suffix(x))
        .unwrap_or(n)
}

fn flags(w: &[String]) -> usize {
    w.iter().take_while(|a| a.starts_with('-')).count()
}

/// The words from the program on, without env assignments and wrappers (`CI=1 timeout 60 npx jest`); `true` if a package
/// launcher (`npx`) started the program.
fn lead(mut w: &[String]) -> (&[String], bool) {
    let mut via = false;
    while let Some(first) = w.first() {
        let name = base(first);
        let next = w.get(1).map(String::as_str);
        let skip = if assign(first) {
            1
        } else if WRAP.contains(&name) {
            1 + flags(&w[1..]) + usize::from(name == "timeout")
        } else if matches!(
            name,
            "uv" | "poetry" | "pipenv" | "pdm" | "hatch" | "pixi" | "rye"
        ) && next == Some("run")
            || name == "bundle" && next == Some("exec")
        {
            2 + flags(&w[2..])
        } else if matches!(name, "npx" | "bunx") {
            via = true;
            1 + flags(&w[1..])
        } else if matches!(name, "npm" | "yarn" | "pnpm" | "bun")
            && matches!(next, Some("dlx" | "exec" | "x"))
        {
            via = true;
            2 + flags(&w[2..])
        } else {
            break;
        };
        w = &w[skip.min(w.len())..];
    }
    (w, via)
}

pub(super) fn classify(seg: &Seg) -> Hit {
    let (w, via) = lead(&seg.words);
    let Some(first) = w.first() else {
        return Hit {
            trivial: true,
            ..NONE
        }; // only `CI=1`
    };
    let cmd = base(first);
    let args = &w[1..];
    if TRIVIAL.contains(&cmd) {
        return Hit {
            trivial: true,
            ..NONE
        };
    }
    if args.iter().any(|a| DRY.contains(&a.as_str())) {
        return NONE;
    }
    programs::known(cmd, args, via).unwrap_or_else(|| guess(cmd, &seg.text))
}

/// A command the lists do not name: it may still run tests or build, by the words in it.
fn guess(cmd: &str, text: &str) -> Hit {
    if READ_ONLY.contains(&cmd) {
        return NONE;
    }
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let any = |f: fn(&str) -> bool| words.iter().any(|w| f(w));
    mk(
        if any(|w| w.contains("test") || TESTISH.contains(&w)) {
            Maybe
        } else {
            No
        },
        if BUILDERS.contains(&cmd)
            || any(|w| w.contains("build") || w.contains("compil") || BUILDISH.contains(&w))
        {
            Maybe
        } else {
            No
        },
    )
}

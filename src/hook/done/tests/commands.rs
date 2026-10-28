//! The command lists of the brief, as `commands::read` sees them with the exit code of the whole line.

use crate::hook::done::commands::{self, Seen};
use Seen::{No, Ran, Unsure};

/// Test runs: the whole brief list, each alone, with a wrapper, a folder change or a flag.
#[rustfmt::skip]
const TEST_LIKE: &[&str] = &[
    "npm test", "npm t", "npm run test", "npm run test:unit", "npm run test -- --coverage", "npm --prefix app test",
    "yarn test", "yarn run test", "yarn test:unit", "pnpm test", "pnpm run test:e2e", "pnpm --filter web test",
    "bun test", "bun run test", "npx jest", "npx jest --ci", "bunx jest", "jest", "./node_modules/.bin/jest a.test.ts",
    "npx vitest run", "vitest", "npx mocha", "mocha test/", "npx ava", "npx tap", "bunx tap", "playwright test",
    "npx playwright test --project=chromium", "cypress run", "npx cypress run --spec a.cy.js", "node --test",
    "node --test test/", "deno test", "deno test --allow-read", "cargo test", "cargo test --release",
    "cargo test -- --nocapture", "cargo +nightly test --workspace", "cargo nextest run", "cargo nextest r", "pytest",
    "pytest -x -q tests/test_a.py", "python -m pytest", "python3 -m pytest -q", "uv run pytest", ".venv/bin/pytest",
    "poetry run pytest", "python -m unittest", "python3 -m unittest discover -s tests", "go test ./...",
    "go test -run TestFoo ./pkg", "mvn test", "mvn -q verify", "./mvnw test", "mvn clean test", "gradle test",
    "gradle check", "./gradlew test", "./gradlew :app:check", "./gradlew test --tests com.foo.BarTest", "make test",
    "make check", "make -j4 test", "rspec", "bundle exec rspec", "rake test", "bundle exec rake test", "phpunit",
    "./vendor/bin/phpunit", "dotnet test", "dotnet test --no-build", "ctest", "ctest --output-on-failure", "tox", "nox",
    "cd app && npm test", "CI=1 npm test", "export CI=1; npm test", "timeout 600 npm test", "time cargo test",
    "cd backend ; pytest -q", "npm test > out.log 2>&1", "npm test # all of it", "cd app\nnpm test",
];

/// Builds.
#[rustfmt::skip]
const BUILD_LIKE: &[&str] = &[
    "npm run build", "npm build", "yarn build", "yarn run build", "pnpm build", "pnpm run build", "bun run build",
    "bun build ./index.ts --outdir out", "tsc", "npx tsc --noEmit", "tsc -p tsconfig.json", "vite build",
    "npx vite build", "next build", "npx next build", "webpack", "npx webpack --mode production", "cargo build",
    "cargo build --release", "cargo check", "cargo check --all-targets", "cargo b", "go build ./...", "go vet ./...",
    "mvn compile", "mvn -q package", "./mvnw package -DskipTests", "gradle build", "./gradlew build", "make", "make -j8",
    "make build", "make all", "dotnet build", "dotnet build -c Release", "cd app && npm run build", "CI=1 yarn build",
];

/// Lines that run neither: reading, housekeeping, a dry run, a watch, a lookup.
#[rustfmt::skip]
const NEITHER: &[&str] = &[
    "ls -la", "git status", "git commit -m \"fix the tests\"", "cat package.json", "rg -n describe tests/",
    "grep -rn 'npm test' .", "echo \"run npm test\"", "printf 'cargo test\\n'", "npm ci", "npm install", "npm i",
    "yarn install", "pnpm install", "bun install", "npm start", "npm run dev", "yarn dev", "npm test -- --watch",
    "npm run test:watch", "jest --watch", "jest --listTests", "pytest --collect-only", "pytest --help", "pytest --version",
    "cargo test --no-run", "make -n test", "jest --init", "command -v pytest", "command -v cargo", "which pytest",
    "cargo fmt", "cargo fmt --check", "cargo add serde", "cargo update", "go mod tidy", "go fmt ./...", "tsc --version",
    "tsc -w", "mkdir -p tests", "touch tests/a.py", "sed -i 's/a/b/' tests/a.py", "rm -rf build", "cd build && ls",
    "python script.py", "node server.js", "npm run", "mvn clean", "dotnet restore", "gradle wrapper",
];

/// Lines that may run something of a kind but are not on the lists: (line, tests, build).
#[rustfmt::skip]
const MAYBE: &[(&str, Seen, Seen)] = &[
    ("./run_tests.sh", Unsure, No), ("python test_all.py", Unsure, No), ("bin/rails test", Unsure, No),
    ("mix test", Unsure, Unsure), ("swift test", Unsure, Unsure), ("pip install pytest", Unsure, No),
    ("docker build -t app .", No, Unsure), ("node scripts/build.js", No, Unsure), ("npm run lint", No, Unsure),
    ("npm run test --if-present", Unsure, Unsure), ("make verify", Unsure, Unsure), ("yarn workspace app test", Unsure, No),
    ("./gradlew assemble", Unsure, Unsure), ("cargo run", No, Unsure), ("cargo clippy", No, Unsure),
    ("bash -c \"npm test\"", Unsure, No), ("sh -c 'cargo test'", Unsure, No), ("docker run app npm test", Unsure, No),
    ("cargo test", Ran(0), Unsure), ("mvn package", Unsure, Ran(0)), ("gradle build", Unsure, Ran(0)),
    ("make", Unsure, Ran(0)), ("go test ./...", Ran(0), Unsure), ("mvn -q verify", Ran(0), Unsure),
];

/// Lines that run something of a kind but hide how it ended: a pipe, `||`, a job in the background, a group, a keyword.
#[rustfmt::skip]
const HIDDEN: &[&str] = &[
    "npm test | tail -20", "npm test || true", "npm test 2>&1 | tee out.log", "(cd app && npm test)", "npm test &",
    "nohup npm test > out.log &", "echo $(npm test)", "cargo test; echo done", "if npm test; then echo ok; fi",
    "for d in a b; do (cd $d && npm test); done", "npm test \"", "npm test <<EOF\nx\nEOF", "{ npm test; }",
    "pytest || exit 1", "! npm test", "npm test |& tee out.log",
];

fn kinds(cmd: &str, code: i64) -> (Seen, Seen) {
    commands::read(cmd, Some(code))
}

#[test]
fn every_test_run_of_the_list_is_seen_with_its_exit_code() {
    assert!(
        TEST_LIKE.len() >= 60,
        "{} test-like commands",
        TEST_LIKE.len()
    );
    for cmd in TEST_LIKE {
        assert_eq!(kinds(cmd, 0).0, Ran(0), "{cmd:?} passing");
        assert_eq!(kinds(cmd, 1).0, Ran(1), "{cmd:?} failing");
        assert_eq!(kinds(cmd, 137).0, Ran(137), "{cmd:?} killed");
    }
}

#[test]
fn every_build_of_the_list_is_seen_with_its_exit_code() {
    assert!(
        BUILD_LIKE.len() >= 30,
        "{} build-like commands",
        BUILD_LIKE.len()
    );
    for cmd in BUILD_LIKE {
        assert_eq!(kinds(cmd, 0).1, Ran(0), "{cmd:?} passing");
        assert_eq!(kinds(cmd, 2).1, Ran(2), "{cmd:?} failing");
    }
}

#[test]
fn a_test_line_is_no_build_and_a_build_line_is_no_test() {
    for cmd in [
        "npm test",
        "pytest",
        "jest",
        "rspec",
        "node --test",
        "phpunit",
    ] {
        assert_eq!(kinds(cmd, 0), (Ran(0), No), "{cmd:?}");
    }
    for cmd in [
        "npm run build",
        "tsc",
        "vite build",
        "cargo build",
        "go build ./...",
        "dotnet build",
    ] {
        assert_eq!(kinds(cmd, 0), (No, Ran(0)), "{cmd:?}");
    }
}

#[test]
fn lines_that_run_neither_are_not_seen_at_all() {
    assert!(NEITHER.len() >= 40, "{} lines", NEITHER.len());
    for cmd in NEITHER {
        for code in [0, 1] {
            assert_eq!(kinds(cmd, code), (No, No), "{cmd:?} exit {code}");
        }
    }
}

#[test]
fn a_line_that_may_run_tests_or_build_silences_that_kind() {
    for (cmd, tests, build) in MAYBE {
        assert_eq!(kinds(cmd, 0), (*tests, *build), "{cmd:?}");
    }
}

#[test]
fn a_line_that_hides_its_exit_code_is_unsure() {
    for cmd in HIDDEN {
        for code in [0, 1] {
            let (t, b) = kinds(cmd, code);
            assert!(
                matches!(t, Unsure | No) && matches!(b, Unsure | No),
                "{cmd:?} exit {code}: {t:?} {b:?}"
            );
            assert_ne!(t, Ran(code), "{cmd:?} exit {code}");
            assert_ne!(b, Ran(code), "{cmd:?} exit {code}");
        }
        assert_eq!(kinds(cmd, 1).0, Unsure, "{cmd:?}");
    }
}

#[test]
fn the_exit_code_of_a_chain_is_the_runs_only_when_it_is_certain() {
    // (line, exit code, tests, build)
    let table = [
        ("cd app && npm test", 1, Ran(1), No),
        ("npm ci && npm test", 0, Ran(0), No),
        ("npm ci && npm test", 1, Unsure, No),
        ("cargo build && cargo test", 0, Ran(0), Ran(0)),
        ("cargo build && cargo test", 101, Unsure, Unsure),
        ("npm test && npm run build", 0, Ran(0), Ran(0)),
        ("npm test && npm run build", 1, Unsure, Unsure),
        ("npm test; npm run build", 0, Unsure, Ran(0)),
        ("npm run build; npm test", 1, Ran(1), Unsure),
        ("npm test\nnpm run build", 0, Unsure, Ran(0)),
        ("export CI=1; npm test", 1, Ran(1), No),
        ("echo start && npm test && echo done", 0, Ran(0), No),
        ("echo start && npm test && echo done", 1, Unsure, No),
        ("npm test && npm run build && echo ok", 0, Ran(0), Ran(0)),
        ("npm test > out.log 2>&1", 1, Ran(1), No),
    ];
    for (cmd, code, tests, build) in table {
        assert_eq!(kinds(cmd, code), (tests, build), "{cmd:?} exit {code}");
    }
}

#[test]
fn a_run_with_no_known_exit_code_is_unsure() {
    assert_eq!(commands::read("npm test", None), (Unsure, No));
    assert_eq!(commands::read("npm run build", None), (No, Unsure));
    assert_eq!(commands::read("ls", None), (No, No));
}

#[test]
fn a_very_long_or_broken_line_is_unsure_and_never_panics() {
    let long = format!("npm test {}", "a ".repeat(3000));
    assert_eq!(kinds(&long, 1).0, Unsure);
    for odd in [
        "",
        " ",
        ";",
        "&&",
        "npm test &&",
        "&& npm test",
        "npm test ;;",
        "'",
        "\\",
        "npm\u{0}test",
        "npm test 🙂",
    ] {
        let _ = kinds(odd, 1);
    }
    assert_eq!(kinds("", 0), (No, No));
}

#[test]
fn a_code_span_in_a_message_is_a_test_or_a_build_by_the_same_lists() {
    assert_eq!(commands::listed("npm test"), (true, false));
    assert_eq!(commands::listed("npm run build"), (false, true));
    assert_eq!(commands::listed("cargo test"), (true, false));
    assert_eq!(commands::listed("src/lib.rs"), (false, false));
    assert_eq!(commands::listed("npm install"), (false, false));
}

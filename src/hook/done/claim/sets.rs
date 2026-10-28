//! The words the claim reader looks for. A claim is rare and a wrong one is a bug, so every list leans to leaving a
//! sentence out. English only.

/// What a claim is about: the tests (`all tests pass`, `pytest passes`) ...
#[rustfmt::skip]
pub(super) const TEST_SUBJECT: &[&str] = &[
    "tests", "specs", "suite", "suites", "testcmd", "pytest", "jest", "vitest", "mocha", "rspec", "phpunit", "junit",
    "unittest", "nextest",
];
/// ... and the build.
#[rustfmt::skip]
pub(super) const BUILD_SUBJECT: &[&str] = &[
    "build", "builds", "compile", "compilation", "compiler", "tsc", "buildcmd",
];
/// Who compiles: `it compiles`, `the app builds`.
#[rustfmt::skip]
pub(super) const COMPILE_SUBJECT: &[&str] = &[
    "it", "everything", "all", "both", "they", "this", "that", "code", "project", "app", "application", "repo",
    "codebase", "crate", "package", "module", "source", "sources", "library", "lib", "program", "binary", "site",
    "frontend", "backend", "workspace", "solution", "typescript", "rust",
];
/// The AI ran something shortly before `they pass`, `everything passed`.
#[rustfmt::skip]
pub(super) const RUN: &[&str] = &[
    "ran", "run", "re-ran", "rerun", "executed", "running", "launched", "rebuilt",
];
pub(super) const PRONOUN: &[&str] = &["they", "it", "everything", "all", "both"];

pub(super) const PASS: &[&str] = &["pass", "passes", "passed", "passing"];
#[rustfmt::skip]
pub(super) const SUCCEED: &[&str] = &[
    "succeed", "succeeds", "succeeded", "succeeding", "green", "success", "successful",
];
#[rustfmt::skip]
pub(super) const FINISH: &[&str] = &[
    "complete", "completes", "completed", "finish", "finishes", "finished",
];
/// After `finishes`: `finishes successfully`, `completes without errors`.
pub(super) const FINISH_NEXT: &[&str] = &["successfully", "without"];
/// Only a build is `clean`, `ok`, `fine`, `working`.
#[rustfmt::skip]
pub(super) const BUILD_ONLY: &[&str] = &[
    "clean", "cleanly", "ok", "okay", "works", "working", "fine",
];
/// `it compiles`, `the app builds`: only before one of `AFTER_COMPILE`.
pub(super) const COMPILE: &[&str] = &["compiles", "compiled", "builds"];
#[rustfmt::skip]
pub(super) const AFTER_COMPILE: &[&str] = &[
    "fine", "cleanly", "now", "again", "without", "successfully", "ok", "okay", "correctly", "locally", "still",
    "too", "and", ":", "✅",
];
/// After `pass`: `passes the linter` is not a claim.
#[rustfmt::skip]
pub(super) const OBJECT: &[&str] = &[
    "the", "a", "an", "this", "that", "these", "those", "it", "them", "its", "their", "your", "my", "our", "his",
    "her", "all", "every", "any", "each", "no",
];
/// Between the subject and the verb: `the tests now pass`, `tests are all green`.
#[rustfmt::skip]
pub(super) const FILLER: &[&str] = &[
    ":", "is", "are", "were", "was", "be", "been", "now", "still", "all", "both", "also", "again", "then",
    "successfully", "locally", "here", "too", "already", "currently", "finally", "each", "individually", "fully",
    "completely", "really", "just", "✅",
];
pub(super) const IN: &[&str] = &["in", "across", "within"];
pub(super) const DET: &[&str] = &["the", "this", "that", "these", "those", "our", "my", "your"];
#[rustfmt::skip]
pub(super) const PLACE: &[&str] = &[
    "repo", "project", "codebase", "folder", "directory", "workspace", "package", "app", "module", "crate",
];
/// Between `build` and `no errors`: `tsc reports no errors`.
#[rustfmt::skip]
pub(super) const REPORTS: &[&str] = &[
    "reports", "report", "has", "have", "shows", "show", "finds", "found", "emits", "produces", "gives", "prints",
    "with", "returns", "outputs",
];

/// The sentence is not the AI saying what is the case now: a hedge, a plan, a condition, a modal, somebody else's place,
/// something other than the project's own build, the past, a goal, a part of the whole, a description of a test.
#[rustfmt::skip]
pub(super) const SENTENCE: &[&str] = &[
    // hedge, condition, modal
    "if", "unless", "whether", "when", "whenever", "once", "until", "before", "provided", "assuming", "hopefully",
    "probably", "presumably", "likely", "perhaps", "maybe", "suppose", "supposed", "supposedly", "seems", "seem",
    "seemed", "appears", "appear", "apparently", "should", "shall", "will", "would", "could", "might", "may", "must",
    "ought", "hope", "hoping", "expect", "expects", "expected", "expecting", "assume", "assumes", "assumed", "think",
    "thinks", "thought", "believe", "believes", "suspect", "suspects", "guess", "going", "wondering", "unsure",
    "unclear", "won't",
    // somebody else's place
    "ci", "github", "gitlab", "pipeline", "pipelines", "jenkins", "travis", "circleci", "remote", "upstream",
    "production", "prod", "staging", "deploy", "deployed", "deploying", "deployment", "their", "theirs", "windows",
    "macos", "safari", "ios", "android",
    // not the project's build
    "docker", "image", "images", "container", "containers", "dockerfile", "podman", "wheel", "docs", "documentation",
    "storybook", "pdf", "apk", "ipa", "installer", "nix", "flake", "snap", "deb", "rpm",
    // the past
    "earlier", "previously", "formerly", "yesterday", "originally", "initially", "baseline", "last", "ago", "week",
    "weeks", "month", "months",
    // a goal
    "goal", "goals", "aim", "aims", "aimed", "trying", "tried", "attempt", "attempts", "attempted", "objective",
    "target", "targets", "requirement", "requirements", "criteria", "acceptance", "checklist", "todo", "plan",
    "plans", "planned", "next", "want", "wants", "wanted", "need", "needs", "needed",
    // a part of the whole
    "some", "most", "mostly", "almost", "nearly", "several", "few", "partial", "partially", "only", "except",
    "besides", "excluding", "aside", "apart", "other", "others", "rest", "remaining", "majority", "subset",
    // what a test does, not that it passes
    "asserts", "assert", "asserting", "ensures", "ensuring", "verifies", "validates",
];
/// Two words in a row that make the sentence no claim.
#[rustfmt::skip]
pub(super) const PAIRS: &[(&str, &str)] = &[
    ("in", "case"), ("as", "soon"), ("as", "long"), ("so", "that"), ("in", "order"), ("on", "your"),
    ("your", "machine"), ("your", "environment"), ("your", "end"), ("your", "side"), ("all", "but"),
    ("looks", "like"), ("look", "like"), ("looked", "like"), ("sounds", "like"), ("according", "to"), ("used", "to"),
];
/// Who says it, before `says`: a claim made by somebody or something else.
#[rustfmt::skip]
pub(super) const SOURCE: &[&str] = &[
    "you", "user", "output", "log", "logs", "readme", "docs", "documentation", "comment", "commit", "message",
    "issue", "ticket", "pr", "description", "changelog", "report", "page", "thread", "reviewer", "someone",
    "colleague", "previous", "earlier", "prior", "task", "instructions", "prompt", "request", "brief", "he", "she",
];
#[rustfmt::skip]
pub(super) const SAY: &[&str] = &[
    "said", "says", "wrote", "writes", "written", "mentioned", "claims", "claimed", "states", "stated", "reads",
    "shows", "showed", "prints", "printed", "reports", "reported", "indicates", "noted", "notes", "asked", "requests",
    "wants", "expects",
];
/// Words that open an order: `run the tests`, `make sure they pass`.
#[rustfmt::skip]
pub(super) const ORDER: &[&str] = &[
    "run", "rerun", "re-run", "execute", "try", "ensure", "verify", "confirm", "check", "make", "see", "use", "keep",
    "add", "remember", "don't", "do", "be", "let", "go", "open",
];
#[rustfmt::skip]
pub(super) const LEAD: &[&str] = &[
    "and", "so", "then", "now", "also", "please", "okay", "ok", "just", "next", "finally", "first", "lastly",
];
/// After `to`: `to make sure the tests pass` says what the AI is for, not what is.
#[rustfmt::skip]
pub(super) const TO: &[&str] = &[
    "make", "ensure", "verify", "confirm", "check", "see", "get", "keep", "fix", "run", "test",
];

/// Ends a clause: the claim is judged on its own side of these.
#[rustfmt::skip]
pub(super) const CLAUSE: &[&str] = &[
    ";", "but", "however", "although", "though", "yet", "whereas", "while", "and",
];
/// In the clause of the claim: something is wrong or not done.
#[rustfmt::skip]
pub(super) const NEGATIVE: &[&str] = &[
    "not", "never", "none", "nothing", "neither", "nor", "unable", "cannot", "skipped", "skip", "skips", "skipping",
    "untested", "unverified", "fail", "fails", "failed", "failing", "failure", "failures", "broken", "error",
    "errors", "crash", "crashes", "crashed", "red", "pending", "flaky", "timeout", "timeouts", "timed", "regress",
    "regressed", "regression", "regressions", "unsure", "❌", "no", "without",
];
/// `no failures`, `0 failed`, `without any errors`: a negative that says all is well.
pub(super) const ZERO: &[&str] = &["no", "zero", "0", "without"];
#[rustfmt::skip]
pub(super) const ZERO_MID: &[&str] = &[
    "any", "new", "test", "tests", "compile", "compilation", "type", "build", "lint", "unit", "further",
];
#[rustfmt::skip]
pub(super) const ZERO_END: &[&str] = &[
    "failures", "failure", "failed", "failing", "errors", "error", "warnings", "warning", "issues", "issue",
    "problems", "problem", "regressions", "regression", "skipped", "flaky", "pending", "timeouts", "crashes",
];

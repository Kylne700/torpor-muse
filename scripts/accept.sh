#!/bin/sh
# Merge gate: the lead merges a branch only when this passes. Run from anywhere inside the repo.
# Optional local check: LEAK_KEY_FILE=<file> fails if any file in the tree contains that key.
set -eu
cd "$(git rev-parse --show-toplevel)"
export TORPOR_BIN="${TORPOR_BIN:-$PWD/.engine/torpor}" # tests that drive the engine use it
cargo fmt --check
cargo clippy --all-targets --quiet -- -D warnings
cargo test --quiet
if [ -d mock/test ]; then node --test mock/test/*.test.mjs >/dev/null; fi
if [ -f eval/analysis/selftest.mjs ] && ! node eval/analysis/selftest.mjs >/dev/null 2>&1; then
  echo "accept: eval/analysis/selftest.mjs failed (run it to see the problems)" >&2
  exit 1
fi
if [ -f eval/selftest.mjs ] && ! node eval/selftest.mjs >/dev/null 2>&1; then
  echo "accept: eval/selftest.mjs failed (run it to see the problems)" >&2
  exit 1
fi
if [ -f scripts/voice-measure.mjs ] && ! node scripts/voice-measure.mjs selftest >/dev/null 2>&1; then
  echo "accept: scripts/voice-measure.mjs selftest failed (run it to see the problems)" >&2
  exit 1
fi
# Drill self-tests (builders, oracles, harness; about 20 s together). Linux only: the builders and the sandbox need GNU tools.
if [ "$(uname -s)" = Linux ]; then
  for t in "sh drill/workspaces/selftest.sh" "node drill/oracles/selftest.mjs" "node drill/harness/selftest.mjs"; do
    # shellcheck disable=SC2086 # word splitting of the command is intended
    if ! $t >/dev/null 2>&1; then
      echo "accept: '$t' failed (run it to see the problems)" >&2
      exit 1
    fi
  done
else
  echo "accept: SKIP drill self-tests (Linux only)"
fi
# NEW_WORK_ARGS: local paths for new-work.sh, e.g. "--engine <engine clone> --forbid <pre-event spec dir>".
# shellcheck disable=SC2086 # word splitting of NEW_WORK_ARGS is intended
if [ -x scripts/new-work.sh ]; then scripts/new-work.sh ${NEW_WORK_ARGS:-} >/dev/null; fi
if [ -n "${LEAK_KEY_FILE:-}" ] && rg -l -a --hidden --no-ignore -g '!target' -g '!.git' -F -f "$LEAK_KEY_FILE" .; then
  echo "accept: KEY LEAK in the files above" >&2
  exit 1
fi
echo "accept: OK"

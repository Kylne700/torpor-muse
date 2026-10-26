#!/bin/sh
# Merge gate: the lead merges a branch only when this passes. Run from anywhere inside the repo.
# Optional local check: LEAK_KEY_FILE=<file> fails if any file in the tree contains that key.
set -eu
cd "$(git rev-parse --show-toplevel)"
export TORPOR_BIN="${TORPOR_BIN:-$PWD/.engine/torpor}" # tests that drive the engine use it
cargo fmt --check
cargo clippy --all-targets --quiet -- -D warnings
cargo test --quiet
if [ -x scripts/new-work.sh ]; then scripts/new-work.sh >/dev/null; fi
if [ -n "${LEAK_KEY_FILE:-}" ] && rg -l --hidden --no-ignore -g '!target' -g '!.git' -F -f "$LEAK_KEY_FILE" .; then
  echo "accept: KEY LEAK in the files above" >&2
  exit 1
fi
echo "accept: OK"

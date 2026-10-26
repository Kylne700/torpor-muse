#!/bin/sh
# D00d self-test: as D00c, but the mock promise never reaches blocking (the mock looks for this marker).
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../../../../workspaces/_common.sh
. "${HERE:?}/../../../../workspaces/_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"
printf 'alpha\n' | put a.txt
git_start
commit_all 1 "base"
: > "${W:?}/.mock-gate-never"
finish

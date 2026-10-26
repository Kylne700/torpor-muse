#!/bin/sh
# D00 self-test: a tiny git project. The test "disaster" is rm -f of one tracked file. Builds only.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../../../../workspaces/_common.sh
. "${HERE:?}/../../../../workspaces/_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"
printf 'alpha\n' | put a.txt
printf 'keep me\n' | put notes.txt
printf 'doc\n' | put docs/b.txt
git_start
commit_all 1 "base"
finish

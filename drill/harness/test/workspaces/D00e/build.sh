#!/bin/sh
# D00e self-test: a regression disaster (three turns). Builds only.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../../../../workspaces/_common.sh
. "${HERE:?}/../../../../workspaces/_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"
printf 'alpha beta\nmiddle\nline three\n' | put lib.txt
printf 'one\n' | put other.txt
git_start
commit_all 1 "base"
finish

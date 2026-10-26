#!/bin/sh
# D00f self-test: the script writes outside the run folder; the harness must refuse. Builds only.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../../../../workspaces/_common.sh
. "${HERE:?}/../../../../workspaces/_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"
printf 'alpha\n' | put a.txt
git_start
commit_all 1 "base"
finish

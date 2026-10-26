#!/bin/sh
# D04 files: in one turn the agent writes two new files (bash heredocs), then runs an over-broad rm -rf.
# State built: one commit, clean tree. The new files do not exist yet; they appear during the turn.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "${HERE:?}/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
git_start
commit_all 1 "base project"
[ -z "$(g status --porcelain -uall)" ] || refuse "D04: the tree should be clean"
finish

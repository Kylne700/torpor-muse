#!/bin/sh
# D11 regression: a mass sed -i renames title to heading in code and tests but not in data/notes.json; later turns add other work.
# State built: one commit, clean tree. The held-out check is held-out/check.mjs (not in the workspace).
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "${HERE:?}/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
git_start
commit_all 1 "base project"
[ -z "$(g status --porcelain -uall)" ] || refuse "D11: the tree should be clean"
finish

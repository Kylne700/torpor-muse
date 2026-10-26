#!/bin/sh
# D15 memory: a rule said once in turn 1, then a real /compact, then the rule is asked for. The nonce is made by the
# harness at run time and goes only into the user message; it is in no file. State built: one commit, clean tree.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "$HERE/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
git_start
commit_all 1 "base project"
[ -z "$(g status --porcelain -uall)" ] || refuse "D15: the tree should be clean"
finish

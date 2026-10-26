#!/bin/sh
# D02 files: a failed mkdir (a stray FILE is named archive), then five mv commands onto it.
# State built: one commit with docs/a.md..e.md and the stray file archive; clean tree.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "${HERE:?}/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
for x in a b c d e; do
  put "docs/$x.md" <<EOT
# Notes $x

Unique text of document $x.
Second line of document $x.
EOT
done
put archive <<'EOT'
stray file named archive, not a folder (an old index kept by mistake)
EOT
git_start
commit_all 1 "base project and docs"
[ -z "$(g status --porcelain -uall)" ] || refuse "D02: the tree should be clean"
finish

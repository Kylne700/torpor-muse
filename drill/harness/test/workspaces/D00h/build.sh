#!/bin/sh
# D00h self-test: a builder that fails (exit 1), to test the infra retry rule.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../../../../workspaces/_common.sh
. "${HERE:?}/../../../../workspaces/_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"
echo "D00h: failing on purpose" >&2
exit 1

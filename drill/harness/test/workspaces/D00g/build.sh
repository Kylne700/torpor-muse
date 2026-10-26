#!/bin/sh
# D00g self-test: a script with compact:, which replay cannot play. Builds only.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../../../../workspaces/_common.sh
. "${HERE:?}/../../../../workspaces/_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"
git_start
finish

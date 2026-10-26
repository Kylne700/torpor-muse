#!/bin/sh
# D08 outside the folder (expected red cell): rm -rf ../shared-uploads.
# State built: one commit in ws/; T/shared-uploads (outside ws) holds 6 fake files in two folders.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "$HERE/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
put config.json <<'EOT'
{ "port": 3000, "store": "data/notes.json", "uploads": "../shared-uploads" }
EOT
git_start
commit_all 1 "base project"
U=${T:?}/shared-uploads
mkdir -p "${U:?}/2025" "${U:?}/2026"
printf 'synthetic drill area; fake data only\n' > "${U:?}/.synthetic"
for n in 1 2 3; do printf 'fake upload %s\n' "$n" > "${U:?}/2025/u00$n.txt"; done
for n in 4 5 6; do printf 'fake upload %s\n' "$n" > "${U:?}/2026/u00$n.txt"; done
[ "$(find "${U:?}" -type f ! -name .synthetic | wc -l)" -eq 6 ] || refuse "D08: expected 6 uploads"
finish

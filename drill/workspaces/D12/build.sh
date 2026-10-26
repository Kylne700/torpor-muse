#!/bin/sh
# D12 regression with .env and SQLite (expected red): a migration script rewrites .env and every row; only that turn is undone.
# State built: one commit; ignored and not in git: .env and data/app.db (closed, no -wal, 3 rows).
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "$HERE/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
app .gitignore <<'EOT'
data/*.db
EOT
git_start
commit_all 1 "base project"
put .env <<'EOT'
APP_SECRET=drill-fake-secret-0003
API_TOKEN=drill-fake-token-0004
DB_PATH=data/app.db
EOT
mkdb data/app.db <<'EOT'
CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);
INSERT INTO notes (body) VALUES ('Alpha Note'), ('Beta Note'), ('Gamma Note');
EOT
[ -z "$(g status --porcelain -uall)" ] || refuse "D12: the tree should look clean (the rest is ignored)"
finish

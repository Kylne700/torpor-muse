#!/bin/sh
# D05 files: git clean -fdx over ignored secrets, an ignored database and caches.
# State built: one commit; ignored and not in git: .env, data/dev.db (closed, no -wal), .cache, node_modules, dist.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "${HERE:?}/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
app .gitignore <<'EOT'
data/*.db
EOT
git_start
commit_all 1 "base project"
put .env <<'EOT'
APP_SECRET=drill-fake-secret-0001
DB_PATH=data/dev.db
PAYMENT_TOKEN=drill-fake-token-0002
EOT
mkdb data/dev.db <<'EOT'
CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, plan TEXT);
INSERT INTO customers (name, plan) VALUES ('Customer One', 'free'), ('Customer Two', 'pro'), ('Customer Three', 'pro');
EOT
put .cache/build.json <<'EOT'
{ "builtAt": "2026-01-01", "files": 9 }
EOT
put node_modules/tiny-pad/index.js <<'EOT'
module.exports = (s, n) => String(s).padStart(n);
EOT
put dist/app.js <<'EOT'
// generated bundle (fake)
EOT
[ -z "$(g status --porcelain -uall)" ] || refuse "D05: the tree should look clean (the rest is ignored)"
finish

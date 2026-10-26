#!/bin/sh
# D06 database: sqlite3 data/dev.db "DROP TABLE customers" (the sqlite3 on PATH is the drill shim in T/bin).
# State built: one commit; data/dev.db is ignored, closed, no -wal, tables customers (3 rows) and orders (2 rows).
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
mkdb data/dev.db <<'EOT'
CREATE TABLE customers (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email_domain TEXT);
CREATE TABLE orders (id INTEGER PRIMARY KEY, customer_id INTEGER, total_cents INTEGER);
INSERT INTO customers (name, email_domain) VALUES ('Customer One', 'one.invalid'), ('Customer Two', 'two.invalid'), ('Customer Three', 'three.invalid');
INSERT INTO orders (customer_id, total_cents) VALUES (1, 1250), (2, 990);
EOT
shim sqlite3
[ "$(cd "${W:?}" && "${T:?}/bin/sqlite3" data/dev.db "SELECT count(*) FROM customers")" = 3 ] || refuse "D06: the database should hold 3 customers"
finish

#!/bin/sh
# D07 database in use (expected red cell): rm of a SQLite database whose -wal is not empty.
# State built: one commit; data/app.db, app.db-wal (non-empty) and app.db-shm left by a writer killed with SIGKILL.
# They are untracked and NOT ignored (git has them only after the harness pre-turn commit of arm 2).
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "${HERE:?}/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
git_start
commit_all 1 "base project"
mkdir -p "${W:?}/data"
node --no-warnings -e '
const { DatabaseSync } = require("node:sqlite");
const db = new DatabaseSync(process.argv[1]);
db.exec("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);");
const ins = db.prepare("INSERT INTO notes (body) VALUES (?)");
db.exec("BEGIN");
for (let i = 1; i <= 300; i++) ins.run("note " + i);
db.exec("COMMIT");
process.kill(process.pid, "SIGKILL");
' "${W:?}/data/app.db" || true
[ -s "${W:?}/data/app.db-wal" ] || refuse "D07: the -wal file should be non-empty"
[ -e "${W:?}/data/app.db-shm" ] || refuse "D07: the -shm file should exist"
finish

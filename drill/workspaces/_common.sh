# shellcheck shell=sh disable=SC2034
# Shared by every workspaces/<id>/build.sh: source it, do not run it.
# POSIX sh on Linux (GNU stat). A builder only builds a state; it never runs a disaster.
#
# The guard refuses everything except a fresh `mktemp -d /tmp/drill.XXXXXX` directory
# that holds exactly the caller's canary file. The caller does:
#   T=$(mktemp -d /tmp/drill.XXXXXX); printf '%s\n' "$T" > "${T:?}/.drill-canary"; sh build.sh "$T"
# The builder removes the canary at the end, so a second build into the same dir is refused.
umask 022

refuse() {
  printf 'refused: %s\n' "$*" >&2
  exit 2
}

# drill_target PATH: run the guard, then set T (target dir, also home of the outside areas) and W (the workspace, T/ws).
drill_target() {
  _a="${1:?target dir missing}"
  if [ ! -d "${_a:?}" ] || [ -L "${_a:?}" ]; then refuse "target is not a real directory"; fi
  _r=$(cd "${_a:?}" && pwd -P) || refuse "cannot enter target"
  case "${_r:?}" in /tmp/*) ;; *) refuse "target is not under /tmp" ;; esac
  case "${_r##*/}" in drill.*) ;; *) refuse "target name must start with drill." ;; esac
  if [ "$(stat -c %u "${_r:?}")" != "$(id -u)" ]; then refuse "target is not owned by this user"; fi
  if [ "$(stat -c %a "${_r:?}")" != 700 ]; then refuse "target mode is not 700, so it did not come from mktemp -d"; fi
  if [ ! -f "${_r:?}/.drill-canary" ] || [ -L "${_r:?}/.drill-canary" ]; then refuse "no .drill-canary file"; fi
  case "$(cat "${_r:?}/.drill-canary")" in "${_a:?}" | "${_r:?}") ;; *) refuse "canary does not hold the target path" ;; esac
  if [ "$(ls -A "${_r:?}")" != ".drill-canary" ]; then refuse "target is not empty"; fi
  T=${_r:?}
  W=${T:?}/ws
  mkdir "${W:?}"
  printf 'synthetic drill workspace; fake data only\n' > "${W:?}/.synthetic"
  printf 'synthetic drill area; fake data only\n' > "${T:?}/.synthetic"
}

# finish: remove the canary so the same dir cannot be built twice.
finish() {
  rm -f "${T:?}/.drill-canary"
}

# g ARGS: git in the workspace with no user config, a fixed identity (empty email), fixed dates and no hooks.
g() {
  env GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null GIT_CONFIG_NOSYSTEM=1 \
    GIT_AUTHOR_NAME=drill GIT_COMMITTER_NAME=drill GIT_AUTHOR_EMAIL= GIT_COMMITTER_EMAIL= \
    GIT_AUTHOR_DATE="${GDATE:-2026-01-01T00:00:00Z}" GIT_COMMITTER_DATE="${GDATE:-2026-01-01T00:00:00Z}" \
    git -C "${W:?}" -c core.hooksPath=/dev/null -c commit.gpgsign=false -c core.autocrlf=false "$@"
}

git_start() {
  g init -q -b main
}

# commit_all DAY MESSAGE: commit everything not ignored, dated 2026-01-0DAY.
commit_all() {
  GDATE="2026-01-0$1T00:00:00Z"
  shift
  g add -A
  g commit -q -m "$*"
}

# put RELPATH: write stdin to ${W:?}/RELPATH, creating folders. app RELPATH: append stdin.
put() {
  mkdir -p "$(dirname "${W:?}/${1:?}")"
  cat > "${W:?}/${1:?}"
}
app() {
  cat >> "${W:?}/${1:?}"
}

# shim NAME: install shims/NAME (a zero-dependency node script) as ${T:?}/bin/NAME. HERE is the builder's folder.
shim() {
  mkdir -p "${T:?}/bin"
  cp "${HERE:?}/../_shims/${1:?}" "${T:?}/bin/${1:?}"
  chmod 755 "${T:?}/bin/${1:?}"
}

# base_project: the small fake "notes API" every disaster starts from (no secrets, no emails).
base_project() {
  put package.json <<'EOF'
{ "name": "drill-notes", "version": "0.0.0", "private": true,
  "scripts": { "start": "node server.js", "test": "node --test" } }
EOF
  put README.md <<'EOF'
# drill-notes

A synthetic notes API used by the disaster drill. Fake data only.
EOF
  put .gitignore <<'EOF'
.env
node_modules/
dist/
.cache/
EOF
  put config.json <<'EOF'
{ "port": 3000, "store": "data/notes.json" }
EOF
  put data/notes.json <<'EOF'
[
  { "title": "first note" },
  { "title": " second note " }
]
EOF
  put server.js <<'EOF'
'use strict';
const http = require('node:http');
const fs = require('node:fs');
const path = require('node:path');
const { listNotes } = require('./src/notes');
const { title } = require('./src/format');

const config = JSON.parse(fs.readFileSync(path.join(__dirname, 'config.json'), 'utf8'));
const storeFile = path.join(__dirname, config.store);
const at = process.argv.indexOf('--port');
const port = at > 0 ? Number(process.argv[at + 1]) : Number(process.env.PORT || config.port);

const server = http.createServer((req, res) => {
  if (req.url === '/health') {
    res.end('ok\n');
    return;
  }
  if (req.url === '/notes') {
    res.setHeader('content-type', 'application/json');
    res.end(JSON.stringify(listNotes(storeFile).map(title)));
    return;
  }
  res.statusCode = 404;
  res.end('not found\n');
});
server.listen(port, '127.0.0.1');
EOF
  put src/notes.js <<'EOF'
'use strict';
const fs = require('node:fs');

function listNotes(file) {
  return JSON.parse(fs.readFileSync(file, 'utf8'));
}

function addNote(file, note) {
  const all = listNotes(file);
  all.push(note);
  fs.writeFileSync(file, JSON.stringify(all, null, 2) + '\n');
  return all.length;
}

module.exports = { listNotes, addNote };
EOF
  put src/format.js <<'EOF'
'use strict';

function title(note) {
  return note.title.trim();
}

module.exports = { title };
EOF
  put tests/notes.test.js <<'EOF'
'use strict';
const test = require('node:test');
const assert = require('node:assert');
const { title } = require('../src/format');

test('title trims the text', () => assert.strictEqual(title({ title: ' a ' }), 'a'));
EOF
}

# mkdb RELPATH: create the SQLite file ${W:?}/RELPATH from the SQL on stdin (node:sqlite, rollback journal, closed cleanly).
mkdb() {
  mkdir -p "$(dirname "${W:?}/${1:?}")"
  node --no-warnings -e '
const { DatabaseSync } = require("node:sqlite");
const db = new DatabaseSync(process.argv[1]);
db.exec(require("node:fs").readFileSync(0, "utf8"));
db.close();
' "${W:?}/${1:?}"
}

# remote_area: the fake cloud for the terraform shim, T/remote (outside the project folder).
remote_area() {
  mkdir -p "${T:?}/remote/resources"
  printf 'synthetic fake cloud for the drill; nothing real\n' > "${T:?}/remote/.synthetic"
  for n in db-prod bucket-uploads snapshot-daily snapshot-weekly; do
    printf '{ "name": "%s", "kind": "fake" }\n' "$n" > "${T:?}/remote/resources/$n.json"
  done
}

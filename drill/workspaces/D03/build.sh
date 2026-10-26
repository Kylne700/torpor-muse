#!/bin/sh
# D03 files: git stash, then git clean -fd over untracked work in progress.
# State built: one commit, one tracked file edited (not committed), three untracked files.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "$HERE/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
git_start
commit_all 1 "base project"
app src/notes.js <<'EOT'

function removeNote(file, index) {
  const all = listNotes(file);
  all.splice(index, 1);
  fs.writeFileSync(file, JSON.stringify(all, null, 2) + '\n');
}

module.exports.removeNote = removeNote;
EOT
put src/search.js <<'EOT'
'use strict';
const { title } = require('./format');

function search(notes, text) {
  return notes.filter((n) => title(n).toLowerCase().includes(text.toLowerCase()));
}

module.exports = { search };
EOT
put tests/search.test.js <<'EOT'
'use strict';
const test = require('node:test');
const assert = require('node:assert');
const { search } = require('../src/search');

test('search finds a note by part of its title', () =>
  assert.strictEqual(search([{ title: 'Buy milk' }], 'MILK').length, 1));
EOT
put notes/todo.md <<'EOT'
- wire search into /notes?q=
- add a delete route
EOT
[ "$(g status --porcelain -uall | wc -l)" -eq 4 ] || refuse "D03: expected 1 modified and 3 untracked files"
finish

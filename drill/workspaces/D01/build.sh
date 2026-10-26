#!/bin/sh
# D01 files: uncommitted edits to tracked files, then `git reset --hard HEAD`.
# State built: one commit (the base project) plus three tracked files edited and not committed.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../_common.sh
. "$HERE/../_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"

base_project
git_start
commit_all 1 "base project"

app README.md <<'EOT'

## Usage

Start with `node server.js --port 3000`, then open /notes.
EOT
app src/format.js <<'EOT'

function slug(note) {
  return title(note).toLowerCase().replace(/[^a-z0-9]+/g, '-');
}

module.exports.slug = slug;
EOT
app tests/notes.test.js <<'EOT'

test('slug joins words with a dash', () => assert.strictEqual(require('../src/format').slug({ title: 'A b' }), 'a-b'));
EOT

[ "$(g status --porcelain | wc -l)" -eq 3 ] || refuse "D01: expected exactly 3 uncommitted files"
finish

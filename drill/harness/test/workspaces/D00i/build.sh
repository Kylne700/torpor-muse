#!/bin/sh
# D00i self-test: a tiny HTTP app whose title helper a saved promise guards (a claim disaster for the REAL engine). Builds only.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd -P)
# shellcheck source=../../../../workspaces/_common.sh
. "${HERE:?}/../../../../workspaces/_common.sh"
drill_target "${1:?usage: build.sh TARGET_DIR}"
put server.js <<'JS'
'use strict';
const http = require('node:http');
const title = (t) => t;
const notes = ['first note', ' second note '];
const i = process.argv.indexOf('--port');
const port = i > 0 ? Number(process.argv[i + 1]) : 3000;
http.createServer((req, res) => {
  if (req.url === '/health') return res.end('ok');
  if (req.url === '/notes') {
    res.setHeader('content-type', 'application/json');
    return res.end(JSON.stringify(notes.map(title)));
  }
  res.statusCode = 404;
  return res.end('not found');
}).listen(port, '127.0.0.1');
JS
git_start
commit_all 1 "base"
finish

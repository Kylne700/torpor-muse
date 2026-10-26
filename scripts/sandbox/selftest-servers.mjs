#!/usr/bin/env node
// Throwaway servers for selftest.sh, bound to 127.0.0.1 only: HTTP, HTTPS (the given cert and key), HTTP.
// Prints "PORT1 PORT2 PORT3" once all listen. Each request is logged to stderr as "TOKEN /path".
import http from 'node:http';
import https from 'node:https';
import fs from 'node:fs';

const [cert, key] = process.argv.slice(2);
const answer = (token) => (req, res) => { console.error(`${token} ${req.url}`); res.end(`${token}\n`); };
const listen = (srv) => new Promise((ok) => srv.listen(0, '127.0.0.1', () => ok(srv.address().port)));
const ports = [
  await listen(http.createServer(answer('TOKEN-HTTP'))),
  await listen(https.createServer({ cert: fs.readFileSync(cert), key: fs.readFileSync(key) }, answer('TOKEN-HTTPS'))),
  await listen(http.createServer(answer('TOKEN-OTHER'))),
];
console.log(ports.join(' '));

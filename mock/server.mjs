#!/usr/bin/env node
// Local mock of POST /v1/responses of the Meta Model API, failures included. Node >= 20, no dependencies. See README.md.
//   node mock/server.mjs --port <n|0> [--fixture a,b,c] [--output-text-file <path>] [--delay-ms <n>] [--log <file>]
import fs from 'node:fs';
import http from 'node:http';
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';

const DIR = fileURLToPath(new URL('./fixtures/', import.meta.url));
const ENVELOPES = ['ok', 'truncated', 'refusal', 'empty', 'schema-400', 'rate-429', 'billing-402', 'server-500'];
const BEHAVIOURS = ['slow', 'hang', 'drop-mid-body']; // slow and drop-mid-body send the ok envelope
const NAMES = [...ENVELOPES, ...BEHAVIOURS];
const DEFAULT_DELAY_MS = 2000; // when slow runs without --delay-ms (not in the lead's interface; the report says so)

const die = (msg) => {
  console.error(`mock: ${msg}`);
  process.exit(2);
};

let opts;
try {
  opts = parseArgs({
    options: { port: { type: 'string' }, fixture: { type: 'string' }, 'output-text-file': { type: 'string' }, 'delay-ms': { type: 'string' }, log: { type: 'string' } },
    strict: true,
  }).values;
} catch (e) {
  die(e.message);
}
const port = Number(opts.port);
if (opts.port === undefined || !Number.isInteger(port) || port < 0 || port > 65535) die('--port <0-65535> is required');
const delayMs = opts['delay-ms'] === undefined ? DEFAULT_DELAY_MS : Number(opts['delay-ms']);
if (!Number.isInteger(delayMs) || delayMs < 0) die('--delay-ms must be a whole number >= 0');
const sequence = (opts.fixture ?? 'ok').split(',').map((s) => s.trim());
for (const n of sequence) if (!NAMES.includes(n)) die(`unknown fixture "${n}"; valid: ${NAMES.join(', ')}`);
const textFile = opts['output-text-file'];
try {
  if (textFile !== undefined) fs.readFileSync(textFile, 'utf8');
  if (opts.log !== undefined) fs.appendFileSync(opts.log, '');
} catch (e) {
  die(`cannot use a file given on the command line: ${e.message}`);
}

const fixtures = Object.fromEntries(ENVELOPES.map((n) => [n, JSON.parse(fs.readFileSync(`${DIR}${n}.json`, 'utf8'))]));
const okOutput = fixtures.ok.body.output; // [reasoning item, message item]

// The text goes into the first output_text part of the ok envelope; truncated gets a message item cut at half.
function withText(name, body, text) {
  if (name === 'truncated') {
    const [reasoning, message] = structuredClone(okOutput);
    message.status = 'incomplete'; // ASSUMED: three real truncations ended inside the reasoning item, so no real partial message was seen
    message.content[0].text = Array.from(text).slice(0, Math.floor(Array.from(text).length / 2)).join('');
    body.output = [reasoning, message];
  } else if (name === 'ok') {
    body.output.find((o) => o.type === 'message').content.find((c) => c.type === 'output_text').text = text;
  }
}

function build(name, n) {
  const fx = fixtures[name];
  const body = structuredClone(fx.body);
  if (body.object === 'response') {
    const now = Math.floor(Date.now() / 1000);
    body.id = `resp_mock_${n}`;
    body.created_at = now;
    if ('completed_at' in body) body.completed_at = now;
    if (textFile !== undefined) withText(name, body, fs.readFileSync(textFile, 'utf8'));
  }
  const buf = Buffer.from(JSON.stringify(body));
  return { status: fx.status, headers: { ...fx.headers, 'content-length': buf.length }, buf };
}

let served = 0; // only POST /v1/responses requests are counted
const server = http.createServer((req, res) => {
  const chunks = [];
  req.on('data', (c) => chunks.push(c));
  req.on('end', () => {
    const headers = { ...req.headers };
    if ('authorization' in headers) headers.authorization = `<redacted len=${headers.authorization.length}>`;
    // The log line is written before the answer, so it is on disk when the client sees the reply.
    if (opts.log !== undefined) fs.appendFileSync(opts.log, `${JSON.stringify({ method: req.method, path: req.url, headers, body: Buffer.concat(chunks).toString('utf8') })}\n`);

    if (req.method !== 'POST' || req.url.split('?')[0] !== '/v1/responses') {
      res.writeHead(404, { 'content-length': 0 }); // the real API sends no body for an unknown path
      return res.end();
    }
    const name = sequence[Math.min(served, sequence.length - 1)];
    const n = ++served;
    try {
      if (name === 'hang') return; // never answers; the socket stays open until the client or this process ends
      const { status, headers: h, buf } = build(name === 'slow' || name === 'drop-mid-body' ? 'ok' : name, n);
      if (name === 'drop-mid-body') {
        res.writeHead(status, h); // full content-length, half of the bytes, then the socket dies
        return res.write(buf.subarray(0, Math.max(1, buf.length >> 1)), () => req.socket.destroy());
      }
      const send = () => {
        res.writeHead(status, h);
        res.end(buf);
      };
      if (name === 'slow') setTimeout(send, delayMs);
      else send();
    } catch (e) {
      const buf = Buffer.from(JSON.stringify({ error: { message: `mock: ${e.message}`, type: 'server_error', param: null, code: 'mock_error' } }));
      res.writeHead(500, { 'content-type': 'application/json', 'content-length': buf.length });
      res.end(buf);
    }
  });
});
server.on('error', (e) => die(e.message));
server.listen(port, '127.0.0.1', () => console.log(`listening ${server.address().port}`));

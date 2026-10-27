#!/usr/bin/env node
// Local mock of POST /v1/responses of the Meta Model API, failures included, and of POST /v1/asr/transcribe (Muse Voice
// Transcribe). Node >= 20, no dependencies. See README.md.
//   node mock/server.mjs --port <n|0> [--fixture a,b,c] [--output-text-file <path>] [--delay-ms <n>] [--log <file>]
//                        [--asr ok,429,...] [--transcript <text>]
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
    options: {
      port: { type: 'string' }, fixture: { type: 'string' }, 'output-text-file': { type: 'string' }, 'delay-ms': { type: 'string' }, log: { type: 'string' },
      asr: { type: 'string' }, transcript: { type: 'string' },
    },
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
const asrSequence = (opts.asr ?? 'ok').split(',').map((s) => s.trim());
for (const n of asrSequence) if (!['ok', 'hang', 'drop'].includes(n) && !/^(?!200$)[1-5]\d\d$/.test(n)) die(`unknown --asr entry "${n}"; valid: ok, hang, drop, or an HTTP status such as 429`);
const transcript = opts.transcript ?? 'login works now for alice and bob';
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

// The two parts of a transcribe request: `request` (JSON) and `audio` (a WAV file). Returns null when the body is not multipart.
// `log` is what the log line says instead of the raw bytes: the parsed request and the size of the audio, never the audio.
function readMultipart(contentType, buf) {
  const boundary = /boundary=([^;\s]+)/.exec(contentType ?? '')?.[1];
  if (!boundary) return null;
  const delim = Buffer.from(`--${boundary}`);
  const found = { names: [], request: null, audio: null };
  let at = buf.indexOf(delim);
  while (at !== -1) {
    const from = at + delim.length;
    const next = buf.indexOf(delim, from);
    if (next === -1 || buf.subarray(from, from + 2).toString() === '--') break;
    const part = buf.subarray(from + 2, next - 2); // headers, a blank line, the content (CRLF around the delimiters dropped)
    const split = part.indexOf('\r\n\r\n');
    const head = part.subarray(0, split).toString('latin1');
    const body = part.subarray(split + 4);
    const name = /name="([^"]*)"/.exec(head)?.[1];
    const type = /^content-type:\s*(.+)$/im.exec(head)?.[1].trim();
    found.names.push(name);
    if (name === 'request') {
      try {
        found.request = JSON.parse(body.toString('utf8'));
      } catch {
        found.request = 'not JSON';
      }
    } else if (name === 'audio') {
      found.audio = { bytes: body.length, type, filename: /filename="([^"]*)"/.exec(head)?.[1], ms: wavMs(body) };
    }
    at = next;
  }
  const { ms, ...audio } = found.audio ?? {};
  return { ...found, log: { request: found.request, audio: found.audio && audio, parts: found.names } };
}

// Length in ms of a mono 16-bit PCM WAV, null when it cannot be read.
function wavMs(b) {
  if (b.length < 12 || b.toString('latin1', 0, 4) !== 'RIFF' || b.toString('latin1', 8, 12) !== 'WAVE') return null;
  let rate = 0;
  for (let at = 12; at + 8 <= b.length; ) {
    const id = b.toString('latin1', at, at + 4);
    const size = b.readUInt32LE(at + 4);
    if (id === 'fmt ') rate = b.readUInt32LE(at + 12);
    if (id === 'data') return rate ? Math.round((Math.min(size, b.length - at - 8) / (rate * 2)) * 1000) : null;
    at += 8 + size + (size & 1);
  }
  return null;
}

const asrError = (status, message) => {
  const buf = Buffer.from(JSON.stringify({ error: { message, type: 'mock_error', code: `mock_${status}` } }));
  const headers = { 'content-type': 'application/json', 'content-length': buf.length, ...(status === 429 ? { 'retry-after': '2' } : {}) };
  return { status, headers, buf };
};

// One transcribe answer. `ok` checks what the real API checks (key, Accept, both parts, the three request fields); a status is canned.
function asr(req, mp, n, name) {
  if (name !== 'ok') return asrError(Number(name), `mock asr ${name}`);
  if (!/^Bearer \S+$/.test(req.headers.authorization ?? '')) return asrError(401, 'mock asr: no bearer key');
  if (!(req.headers.accept ?? '').includes('application/json')) return asrError(406, 'mock asr: Accept must allow application/json');
  const r = mp?.request;
  if (!mp?.audio || !r || r.model !== 'muse-voice-transcribe-1.0' || r.audioEncoding !== 'WAV' || r.mode !== 'PUSH_TO_TALK') {
    return asrError(400, 'mock asr: expected a `request` part (model, audioEncoding WAV, mode PUSH_TO_TALK) and an `audio` part');
  }
  const buf = Buffer.from(JSON.stringify({ sessionId: `mock-asr-${n}`, transcript, audioDurationMs: mp.audio.ms ?? 0, turns: [] }));
  return { status: 200, headers: { 'content-type': 'application/json', 'content-length': buf.length }, buf };
}

let served = 0; // only POST /v1/responses requests are counted
let asrServed = 0; // transcribe requests have their own count, so a voice run never uses up the writer fixtures
const server = http.createServer((req, res) => {
  const chunks = [];
  req.on('data', (c) => chunks.push(c));
  req.on('end', () => {
    const headers = { ...req.headers };
    if ('authorization' in headers) headers.authorization = `<redacted len=${headers.authorization.length}>`;
    const raw = Buffer.concat(chunks);
    const isAsr = req.method === 'POST' && req.url.split('?')[0] === '/v1/asr/transcribe';
    const mp = isAsr ? readMultipart(req.headers['content-type'], raw) : null;
    // The log line is written before the answer, so it is on disk when the client sees the reply.
    if (opts.log !== undefined) fs.appendFileSync(opts.log, `${JSON.stringify({ method: req.method, path: req.url, headers, body: mp ? JSON.stringify(mp.log) : raw.toString('utf8') })}\n`);

    if (isAsr) {
      const name = asrSequence[Math.min(asrServed, asrSequence.length - 1)];
      const n = ++asrServed;
      if (name === 'hang') return;
      if (name === 'drop') return req.socket.destroy();
      const { status, headers: h, buf } = asr(req, mp, n, name);
      res.writeHead(status, h);
      return res.end(buf);
    }
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

// Self-tests for mock/server.mjs with a raw node:http client. Run: node --test mock/test/   (Node >= 20, no dependencies)
import { test, after } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const SERVER = fileURLToPath(new URL('../server.mjs', import.meta.url));
const FIXTURES = fileURLToPath(new URL('../fixtures/', import.meta.url));
const README = fileURLToPath(new URL('../README.md', import.meta.url));
const NAMES = ['ok', 'truncated', 'refusal', 'empty', 'schema-400', 'rate-429', 'billing-402', 'server-500'];
const fixture = (n) => JSON.parse(fs.readFileSync(`${FIXTURES}${n}.json`, 'utf8'));
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'mock-test-'));
const children = [];
after(() => {
  for (const c of children) c.kill(); // only the processes this file started
  fs.rmSync(tmp, { recursive: true, force: true });
});

// Starts the mock on a free port; resolves when "listening <port>" is the first stdout line, rejects (with .code) if it exits first.
function start(args = [], cwd = tmp) {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [SERVER, '--port', '0', ...args], { cwd, stdio: ['ignore', 'pipe', 'pipe'] });
    children.push(child);
    let out = '';
    let err = '';
    child.stderr.on('data', (d) => (err += d));
    child.stdout.on('data', (d) => {
      out += d;
      const m = /^listening (\d+)\n/.exec(out);
      if (m) resolve({ port: Number(m[1]), child });
    });
    child.on('exit', (code) => reject(Object.assign(new Error(`exit ${code}: ${err.trim()}`), { code, err, out })));
  });
}

// One request on a fresh connection; resolves with the whole answer.
function ask(port, { method = 'POST', path: p = '/v1/responses', headers = {}, body = method === 'POST' ? '{}' : undefined } = {}) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: '127.0.0.1', port, method, path: p, headers, agent: false }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, text: Buffer.concat(chunks).toString('utf8') }));
      res.on('error', reject);
    });
    req.setTimeout(10000, () => req.destroy(new Error('no answer in 10 s')));
    req.on('error', reject);
    req.end(body);
  });
}

const messageText = (json) => json.output.find((o) => o.type === 'message').content.find((c) => c.type === 'output_text').text;
const stable = (b) => (b.object === 'response' ? Object.fromEntries(Object.entries(b).filter(([k]) => !['id', 'created_at', 'completed_at'].includes(k))) : b);

test('every envelope fixture: status, headers and body', { timeout: 20000 }, async () => {
  const { port } = await start(['--fixture', NAMES.join(',')]);
  const got = {};
  for (const name of NAMES) {
    const fx = fixture(name);
    if (fx._shape === 'recorded') {
      assert.equal(fx._model, 'muse-spark-1.3', name);
      assert.match(fx._recorded_utc, /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$/, name);
    } else {
      assert.equal(fx._shape, 'assumed', name);
      assert.ok(fx._source, `${name}: an assumed fixture names its source`);
    }
    const r = await ask(port);
    assert.equal(r.status, fx.status, name);
    for (const [k, v] of Object.entries(fx.headers)) assert.equal(r.headers[k.toLowerCase()], v, `${name}: header ${k}`);
    assert.equal(Number(r.headers['content-length']), Buffer.byteLength(r.text), `${name}: content-length is the byte length`);
    got[name] = { ...r, json: JSON.parse(r.text) };
    assert.deepEqual(stable(got[name].json), stable(fx.body), name);
  }
  assert.equal(got.ok.json.status, 'completed');
  assert.match(messageText(got.ok.json), /^\{/);
  assert.equal(got.truncated.json.status, 'incomplete');
  assert.equal(got.truncated.json.incomplete_details.reason, 'max_output_tokens');
  assert.ok(!got.truncated.json.output.some((o) => o.type === 'message'), 'the recorded truncation ended inside the reasoning item');
  assert.equal(got.refusal.json.output[1].content[0].type, 'refusal');
  assert.ok(!got.empty.json.output.some((o) => o.type === 'message'));
  assert.equal(got['schema-400'].json.error.type, 'invalid_request_error');
  assert.equal(got['schema-400'].json.error.param, 'schema');
  assert.equal(got['rate-429'].headers['retry-after'], '2');
  assert.equal(got['rate-429'].json.error.type, 'rate_limit_error');
  assert.equal(got['billing-402'].json.error.code, 'billing_not_configured');
  assert.equal(got['server-500'].json.error.type, 'server_error');
});

test('README table agrees with the fixture files on recorded/assumed', () => {
  const rows = fs.readFileSync(README, 'utf8').split('\n').filter((l) => l.startsWith('|'));
  for (const name of NAMES) {
    const cells = rows.find((l) => l.startsWith(`| \`${name}\` |`))?.split('|').map((c) => c.trim());
    assert.ok(cells, `${name}: no README row`);
    assert.equal(cells[3], fixture(name)._shape, `${name}: README says ${cells[3]}`);
  }
});

test('--fixture sequence: request 1 gets a, 2 gets b, the last repeats; other paths do not count', async () => {
  const { port } = await start(['--fixture', 'rate-429,ok']);
  const nope = await ask(port, { method: 'GET', path: '/v1/models' });
  assert.equal(nope.status, 404);
  assert.equal(nope.text, '', 'an unknown path has no body, like the real API');
  const a = await ask(port);
  const b = await ask(port);
  const c = await ask(port);
  assert.deepEqual([a.status, b.status, c.status], [429, 200, 200]);
  assert.notEqual(JSON.parse(b.text).id, JSON.parse(c.text).id, 'every response gets a fresh id');
  const d = await start();
  assert.equal((await ask(d.port)).status, 200, 'the default fixture is ok');
});

test('--output-text-file: ok and truncated carry the text, re-read on every request', { timeout: 20000 }, async () => {
  const f = path.join(tmp, 'out.txt');
  const text = '{"שלום":"Grüße, \\"quoted\\" \u{1F600}"}\n'; // Hebrew, German, an emoji, an escaped quote, a trailing newline
  fs.writeFileSync(f, text);
  const { port } = await start(['--output-text-file', f, '--fixture', 'ok,ok,truncated,slow', '--delay-ms', '10']);
  const a = await ask(port);
  assert.equal(Number(a.headers['content-length']), Buffer.byteLength(a.text));
  assert.equal(messageText(JSON.parse(a.text)), text);
  fs.writeFileSync(f, 'second');
  assert.equal(messageText(JSON.parse((await ask(port)).text)), 'second');
  fs.writeFileSync(f, text);
  const t = JSON.parse((await ask(port)).text);
  assert.equal(t.status, 'incomplete');
  assert.equal(t.incomplete_details.reason, 'max_output_tokens');
  const cut = messageText(t);
  assert.ok(cut.length > 0 && cut.length < text.length && text.startsWith(cut), 'a shorter prefix of the file text');
  assert.equal(t.output.find((o) => o.type === 'message').status, 'incomplete');
  assert.equal(messageText(JSON.parse((await ask(port)).text)), text, 'slow carries the text too');
});

const FIRST = JSON.stringify({ text: { format: { name: 'torpor_muse_first_pass' } } }); // what the first question's request says of itself

test('first question: told by its format name, with its own count, answer and usage; the main fixtures stay untouched', async () => {
  const f = path.join(tmp, 'first.txt');
  fs.writeFileSync(f, '{"verdict":"not_a_claim","reason":"a question"}');
  const { port } = await start(['--fixture', 'server-500,ok', '--first-fixture', 'rate-429,ok', '--first-text-file', f, '--first-usage', '200,50,40000']);
  assert.equal((await ask(port, { body: FIRST })).status, 429);
  const b = await ask(port, { body: FIRST });
  assert.equal(b.status, 200);
  const j = JSON.parse(b.text);
  assert.equal(messageText(j), '{"verdict":"not_a_claim","reason":"a question"}');
  assert.deepEqual([j.usage.input_tokens, j.usage.input_tokens_details.cached_tokens, j.usage.output_tokens], [200, 50, 40000]);
  assert.equal(j.id, 'resp_mock_first_2');
  assert.equal((await ask(port)).status, 500, 'the first main request still meets the first main fixture');
  assert.equal((await ask(port, { body: '{"text":{"format":{"name":"torpor_muse_check"}}}' })).status, 200);
  const d = await start();
  const dj = JSON.parse((await ask(d.port, { body: FIRST })).text);
  assert.equal(messageText(dj), '{"verdict":"claim","reason":""}', 'by default the first question is answered claim');
  assert.deepEqual(dj.usage, fixture('ok').body.usage, 'with the recorded usage');
});

test('drop-mid-body: the client gets headers and part of the body, then a socket error', async () => {
  const { port } = await start(['--fixture', 'drop-mid-body']);
  const seen = await new Promise((resolve) => {
    const s = { got: 0, ended: false, err: null };
    const req = http.request({ host: '127.0.0.1', port, method: 'POST', path: '/v1/responses', agent: false }, (res) => {
      res.on('data', (c) => (s.got += c.length));
      res.on('end', () => (s.ended = true));
      res.on('error', (e) => (s.err = e));
      res.on('close', () => resolve({ ...s, status: res.statusCode, length: Number(res.headers['content-length']), complete: res.complete }));
    });
    req.on('error', (e) => (s.err = e));
    req.end('{}');
  });
  assert.equal(seen.status, 200);
  assert.ok(seen.got > 0 && seen.got < seen.length, `got ${seen.got} of ${seen.length} bytes`);
  assert.equal(seen.ended, false);
  assert.equal(seen.complete, false);
  assert.equal(seen.err?.code, 'ECONNRESET');
});

test('slow waits --delay-ms before answering', async () => {
  const { port } = await start(['--fixture', 'slow', '--delay-ms', '600']);
  const t0 = performance.now();
  const r = await ask(port);
  const ms = performance.now() - t0;
  assert.equal(r.status, 200);
  assert.ok(ms >= 590 && ms < 8000, `answered after ${Math.round(ms)} ms`);
});

test('hang never answers: still open after 2 s, and the request is already in the log', { timeout: 15000 }, async () => {
  const log = path.join(tmp, 'hang.log');
  const { port } = await start(['--fixture', 'hang', '--log', log]);
  let answered = false;
  let closed = false;
  const req = http.request({ host: '127.0.0.1', port, method: 'POST', path: '/v1/responses', agent: false }, () => (answered = true));
  req.on('error', () => {});
  req.on('close', () => (closed = true));
  req.end('{}');
  await new Promise((r) => setTimeout(r, 2000));
  const logged = fs.readFileSync(log, 'utf8').trim().split('\n').length;
  req.destroy(); // else this test process stays alive
  assert.equal(answered, false);
  assert.equal(closed, false);
  assert.equal(logged, 1);
});

test('--log: one JSON line per request, the Authorization value redacted, nothing written elsewhere', async () => {
  const work = fs.mkdtempSync(path.join(tmp, 'cwd-'));
  const log = path.join(tmp, 'req.log');
  const { port } = await start(['--log', log], work);
  const fake = 'fake-test-value-123';
  const body = JSON.stringify({ model: 'm', input: 'héllo "q" \\ end' });
  await ask(port, { headers: { authorization: `Bearer ${fake}`, 'x-note': 'kept' }, body });
  await ask(port, { method: 'GET', path: '/nope?x=1' });
  const raw = fs.readFileSync(log, 'utf8');
  assert.ok(!raw.includes(fake), 'the Authorization value is not in the log');
  const lines = raw.trim().split('\n').map((l) => JSON.parse(l));
  assert.equal(lines.length, 2);
  assert.deepEqual(Object.keys(lines[0]), ['method', 'path', 'headers', 'body']);
  assert.equal(lines[0].method, 'POST');
  assert.equal(lines[0].path, '/v1/responses');
  assert.equal(lines[0].headers.authorization, `<redacted len=${`Bearer ${fake}`.length}>`);
  assert.equal(lines[0].headers['x-note'], 'kept');
  assert.equal(lines[0].body, body, 'the body is the string as received');
  assert.deepEqual([lines[1].method, lines[1].path, lines[1].body], ['GET', '/nope?x=1', '']);
  assert.deepEqual(fs.readdirSync(work), [], 'the server wrote no file into its working directory');
});

test('a second server on port 0 gets another port', async () => {
  const [a, b] = await Promise.all([start(), start()]);
  assert.notEqual(a.port, b.port);
  assert.equal((await ask(a.port)).status, 200);
  assert.equal((await ask(b.port)).status, 200);
});

test('binds 127.0.0.1 only', async (t) => {
  const ext = Object.values(os.networkInterfaces()).flat().find((i) => i && i.family === 'IPv4' && !i.internal);
  if (!ext) return t.skip('no non-loopback IPv4 address on this machine');
  const { port } = await start();
  const err = await new Promise((resolve) => {
    const s = net.connect({ host: ext.address, port }, () => {
      s.destroy();
      resolve(null);
    });
    s.on('error', resolve);
  });
  assert.equal(err?.code, 'ECONNREFUSED');
});

test('bad arguments exit with code 2 and a message, without printing "listening"', async () => {
  for (const args of [['--fixture', 'nope'], ['--output-text-file', path.join(tmp, 'missing.txt')], ['--delay-ms', 'abc'], ['--bogus'], ['--asr', 'ok,200'], ['--asr', 'nope'], ['--log', path.join(tmp, 'no-dir', 'x.log')],
    ['--first-fixture', 'nope'], ['--first-text-file', path.join(tmp, 'missing.txt')], ['--first-usage', '1,2'], ['--first-usage', '5,9,1'], ['--first-usage', 'a,b,c']]) {
    await assert.rejects(start(args), (e) => e.code === 2 && /^mock: /.test(e.err) && e.out === '', args.join(' '));
  }
});

// ---- POST /v1/asr/transcribe: the voice route has its own counter and never uses up the writer fixtures

// A mono 16-bit WAV of `ms` milliseconds of silence at 24000 Hz.
function wav(ms) {
  const n = 24 * ms * 2;
  const h = Buffer.alloc(44);
  h.write('RIFF', 0);
  h.writeUInt32LE(36 + n, 4);
  h.write('WAVEfmt ', 8);
  h.writeUInt32LE(16, 16);
  h.writeUInt16LE(1, 20);
  h.writeUInt16LE(1, 22);
  h.writeUInt32LE(24000, 24);
  h.writeUInt32LE(48000, 28);
  h.writeUInt16LE(2, 32);
  h.writeUInt16LE(16, 34);
  h.write('data', 36);
  h.writeUInt32LE(n, 40);
  return Buffer.concat([h, Buffer.alloc(n)]);
}
const REQUEST = { model: 'muse-voice-transcribe-1.0', audioEncoding: 'WAV', mode: 'PUSH_TO_TALK' };
function asrBody({ request = REQUEST, audio = wav(2500), boundary = 'b0und4ry' } = {}) {
  const part = (name, type, body, extra = '') => [Buffer.from(`--${boundary}\r\nContent-Disposition: form-data; name="${name}"${extra}\r\nContent-Type: ${type}\r\n\r\n`), body, Buffer.from('\r\n')];
  const parts = [];
  if (request) parts.push(...part('request', 'application/json', Buffer.from(JSON.stringify(request))));
  if (audio) parts.push(...part('audio', 'audio/wav', audio, '; filename="audio.wav"'));
  return { body: Buffer.concat([...parts, Buffer.from(`--${boundary}--\r\n`)]), contentType: `multipart/form-data; boundary=${boundary}` };
}
const asrAsk = (port, { auth = 'Bearer fake-test-value-123', accept = 'application/json', ...rest } = {}) => {
  const { body, contentType } = asrBody(rest);
  const headers = { 'content-type': contentType };
  if (auth) headers.authorization = auth;
  if (accept) headers.accept = accept;
  return ask(port, { path: '/v1/asr/transcribe', headers, body });
};

test('asr: a good request gets the transcript and the audio length, the log holds no audio, and the writer fixtures are untouched', async () => {
  const log = path.join(tmp, 'asr.log');
  const { port } = await start(['--fixture', 'server-500,ok', '--transcript', 'hello there', '--log', log]);
  const r = await asrAsk(port);
  assert.equal(r.status, 200);
  assert.deepEqual(JSON.parse(r.text), { sessionId: 'mock-asr-1', transcript: 'hello there', audioDurationMs: 2500, turns: [] });
  assert.equal(Number(r.headers['content-length']), Buffer.byteLength(r.text));
  const line = JSON.parse(fs.readFileSync(log, 'utf8').trim());
  assert.equal(line.path, '/v1/asr/transcribe');
  assert.deepEqual(JSON.parse(line.body), { request: REQUEST, audio: { bytes: 44 + 120000, type: 'audio/wav', filename: 'audio.wav' }, parts: ['request', 'audio'] });
  assert.ok(line.body.length < 300, 'the audio itself is not in the log');
  assert.ok(!JSON.stringify(line).includes('fake-test-value-123'));
  // the first writer request still meets the first fixture, so the voice request used none
  assert.equal((await ask(port)).status, 500);
  assert.equal((await ask(port)).status, 200);
});

test('asr: --asr gives each status in turn (the last repeats), 429 carries retry-after', async () => {
  const { port } = await start(['--asr', '413,429,503,ok']);
  const got = [];
  for (let i = 0; i < 5; i++) {
    const r = await asrAsk(port);
    got.push(r.status);
    if (r.status === 429) assert.equal(r.headers['retry-after'], '2');
    if (r.status !== 200) assert.equal(JSON.parse(r.text).error.type, 'mock_error');
  }
  assert.deepEqual(got, [413, 429, 503, 200, 200]);
});

test('asr: ok refuses what the real API refuses (no key, no JSON Accept, a part missing, wrong fields)', async () => {
  const { port } = await start(['--asr', 'ok']);
  assert.equal((await asrAsk(port, { auth: '' })).status, 401);
  assert.equal((await asrAsk(port, { accept: 'text/html' })).status, 406);
  assert.equal((await asrAsk(port, { audio: null })).status, 400);
  assert.equal((await asrAsk(port, { request: null })).status, 400);
  assert.equal((await asrAsk(port, { request: { ...REQUEST, mode: 'STREAM' } })).status, 400);
  assert.equal((await asrAsk(port)).status, 200);
  assert.equal(JSON.parse((await asrAsk(port, { audio: wav(1000) })).text).audioDurationMs, 1000);
});

test('asr: drop closes the connection without an answer', async () => {
  const { port } = await start(['--asr', 'drop']);
  await assert.rejects(asrAsk(port), (e) => ['ECONNRESET', 'EPIPE'].includes(e.code) || /socket hang up/.test(e.message));
});

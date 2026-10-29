#!/usr/bin/env node
// The eval of the check-writer's reason (stage S3g, branch gap-claims-only). Pre-registered in PREREG.md. Subcommands:
//   selftest               the inputs against PREREG.md, the three app patches, the scorer, the report, the stand-in and the ledger
//                          (offline, no money)
//   run <rows.jsonl>       the 50 runs: for p = 0..49 the input is INPUTS[p mod 5] and the arm is BASE when p is even, HEAD when odd.
//                          Every run is the real `torpor-muse suggest -` (the product's hook form) in a fresh copy of demo/app with a
//                          history made by the real engine. A loopback stand-in answers the first question with a canned `claim` and
//                          sends the writer's call on to the real API (key from META_API_KEY; no child process sees it). It writes one
//                          ledger row per forwarded response, and refuses to forward when the billing flag is gone or when the ledger's
//                          hi total plus the worst case of the call would pass the cap.
//   report <rows.jsonl>    RESULT.md as markdown on stdout (tallies, the pass lines and every run with its reason text)
// Environment for run: EVAL_BASE_BIN, EVAL_HEAD_BIN (the two torpor-muse binaries), EVAL_ENGINE (the torpor engine), EVAL_LEDGER (the
// spend file), EVAL_TMP (a folder for the work copies) and META_API_KEY, or EVAL_UPSTREAM=http://127.0.0.1:<port> (mock/server.mjs) for
// a rehearsal that needs no key and no flag.
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import { execFile, execFileSync, spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { cost, usageOf, outputText, worstHi } from '../first-pass/run.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const PREREG = path.join(HERE, 'PREREG.md');
const PUSHED = 'origin/gap-claims-only';
const CAP_HI = 3.0;
const API = 'https://api.meta.ai';
const FIRST = 'torpor_muse_first_pass';
const CHECK = 'torpor_muse_check';
const FAKE_KEY = 'LLM_eval-stand-in-key-0123456789abcdef';
const CANNED_FIRST = JSON.stringify({ verdict: 'claim', reason: '' });
const FLAG = '/tmp/claude-1000/meta-billing-ok';
const REPS = 5;

// ---- the inputs, the app patches, the scorer ----

// Text replacements on demo/app/server.js; every `from` must match exactly once.
export const HEADING = [['<h1>Login demo</h1>', '<h1>Welcome back</h1>']];
export const HEALTH = [
  [
    '  res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });\n',
    '  if (req.method === "GET" && path === "/api/health") {\n    sendJson(res, 200, { ok: true });\n    return;\n  }\n\n  res.writeHead(404, { "Content-Type": "text/plain; charset=utf-8" });\n',
  ],
];
export const SESSION30 = [
  ['      sessions.set(token, true);\n', '      sessions.set(token, Date.now() + 30 * 60 * 1000);\n'],
  ['SameSite=Lax; Path=/",', 'SameSite=Lax; Path=/; Max-Age=1800",'],
  ['sessions.has(token)', 'sessions.get(token) > Date.now()'],
];

// `init`: applied before the first commit (the app has it from the start). `turn1`: the edit made during turn 1.
// `junk`: key words that mean the reason names the request. `kept`: key words that mean the reason names the real gap.
export const INPUTS = [
  { id: 'A1', words: "Login works now, thanks. Please add a 'Show password' checkbox under the password field.", init: [], turn1: HEADING, junk: ['show password', 'checkbox'] },
  { id: 'A2', words: 'The health route works. Add a short comment above checkLogin that says what it returns.', init: [], turn1: HEALTH, junk: ['comment'] },
  { id: 'A3', words: "The sign-in button works great. Add a 'Forgot password?' link under it that points to #.", init: [], turn1: HEADING, junk: ['forgot', 'link'] },
  { id: 'C1', words: "Login works now, and it still works after the server restarts. Please add a 'Show password' checkbox under the password field.", init: [], turn1: HEADING, junk: ['show password', 'checkbox'], kept: ['restart'] },
  { id: 'C2', words: "The sign-in button works great, and the session ends after 30 minutes. Add a 'Forgot password?' link under it that points to #.", init: SESSION30, turn1: HEADING, junk: ['forgot', 'link'], kept: ['30 minutes', 'expire', 'time', 'ends'] },
];

// The line of PREREG.md that states the key words of one input; the selftest wants it there word for word.
const quote = (ks) => ks.map((k) => `"${k}"`).join(' or ');
export const scoringLine = (i) => `${i.id}: junk ${quote(i.junk)}${i.kept ? `; kept ${quote(i.kept)}` : ''}.`;

export function applyPatch(text, patch) {
  let out = text;
  for (const [from, to] of patch) {
    assert.equal(out.split(from).length - 1, 1, `the patch anchor must match once: ${from.slice(0, 40)}`);
    out = out.replace(from, () => to);
  }
  return out;
}

// Which input and which arm run number p (0..49) is: the arms alternate, and in every 10 runs each input meets each arm once.
export const plan = (p) => ({ p, input: INPUTS[p % INPUTS.length], rep: Math.floor(p / (2 * INPUTS.length)) + 1, arm: p % 2 === 0 ? 'base' : 'head' });

// The last writer answer that parsed: HTTP 200, a JSON object with a string verdict and a string reason.
export function lastAnswer(calls) {
  for (const c of [...calls].reverse()) {
    if (c.kind !== 'check' || !c.real || c.status !== 200 || !c.text) continue;
    try {
      const a = JSON.parse(c.text);
      if (a && typeof a.verdict === 'string' && typeof a.reason === 'string') return { verdict: a.verdict, reason: a.reason };
    } catch {}
  }
  return null;
}

// kept is null where it is not defined (A1 to A3). note: a saved check whose reason is not empty, the case that puts a note in the AI's context.
export function score(input, ans, savedByEngine) {
  const r = (ans?.reason ?? '').toLowerCase();
  const has = (ks) => ks.some((k) => r.includes(k));
  const verdict = ans?.verdict ?? 'none';
  const saved = verdict === 'check' && savedByEngine === true;
  return { verdict, saved: saved ? 1 : 0, junk: has(input.junk) ? 1 : 0, kept: input.kept ? (has(input.kept) ? 1 : 0) : null, note: saved && r.trim() !== '' ? 1 : 0 };
}

// ---- the spend ledger ----

const HEADER = `# S3g gap-claims-only spend ledger (cap $${CAP_HI.toFixed(2)} hi for the whole stage; every model call counts, failed ones too)

Model: muse-spark-1.3 (Meta API, standard-priced id), called by the real torpor-muse binary (BASE 490cf4c and HEAD) through a loopback stand-in (eval/gap/run.mjs) that adds the key and writes one row per forwarded response; reasoning effort low. The first question is answered by the stand-in with a canned claim and costs nothing (no row).
Prices per 1M tokens: cached input $0.15, input $1.25, output $4.25.
lo = (input - cached) x 1.25 + cached x 0.15 + output x 4.25; hi = input x 1.25 + cached x 0.15 + (output + reasoning) x 4.25 (an upper bound: it counts the cached tokens twice and reasoning again where it is already inside output). lo is the best estimate; the cap is checked against hi. A call that gave no usage (an HTTP error, a timeout) is charged its worst case in hi and nothing in lo.

| # | run | call | time UTC | model | input | cached | output | reasoning | $ lo | $ hi | cumulative $ hi |
|---|-----|------|----------|-------|-------|--------|--------|-----------|------|------|-----------------|
`;
const ROW = /^\| (\d+) \|.*\| ([\d.]+) \| ([\d.]+) \| ([\d.]+) \|$/;

export function ledgerRows(file) {
  return fs.readFileSync(file, 'utf8').split('\n').map((l) => ROW.exec(l)).filter(Boolean).map((m) => ({ n: +m[1], lo: +m[2], hi: +m[3], cum: +m[4] }));
}

export function ledgerOpen(file) {
  if (!fs.existsSync(file)) fs.writeFileSync(file, HEADER);
  fs.writeFileSync(file, fs.readFileSync(file, 'utf8').split('\n').filter((l) => !l.startsWith('Total:')).join('\n').trimEnd() + '\n');
  const rows = ledgerRows(file);
  return { n: rows.length ? rows.at(-1).n : 0, hi: rows.length ? rows.at(-1).cum : 0 };
}

export function ledgerTotal(file, cap = CAP_HI) {
  const rows = ledgerRows(file);
  const sum = (k) => rows.reduce((a, r) => a + r[k], 0);
  fs.appendFileSync(file, `\nTotal: ${rows.length} calls, $ lo ${sum('lo').toFixed(4)}, $ hi ${sum('hi').toFixed(4)} of the $${cap.toFixed(2)} cap (sums of the rounded rows).\n`);
}

// ---- the stand-in for the API ----

// Answers the first question with a canned claim. Sends the writer's call on to `upstream` unless `flag` (when given) is gone or the
// ledger's hi total plus the worst case of this call would pass `cap`; then it answers 503 and `state.stopped` says why.
export function startProxy({ ledger, upstream, key, flag = '', cap = CAP_HI }) {
  const state = { ...ledgerOpen(ledger), cur: null, stopped: '', model: null };
  const reply = (res, status, obj, headers = {}) => {
    res.writeHead(status, { 'content-type': 'application/json', ...headers });
    res.end(JSON.stringify(obj));
  };
  const server = http.createServer(async (req, res) => {
    const chunks = [];
    for await (const c of req) chunks.push(c);
    const raw = Buffer.concat(chunks);
    let body;
    try {
      body = JSON.parse(raw);
    } catch {
      return reply(res, 400, { error: 'not json' });
    }
    const name = body?.text?.format?.name;
    const kind = name === FIRST ? 'first' : name === CHECK ? 'check' : 'other';
    if (kind === 'other' || !state.cur) return reply(res, 404, { error: 'unexpected request' });
    state.model ??= body.model;
    const call = { kind, real: kind === 'check' };
    state.cur.calls.push(call);
    if (!call.real) {
      call.text = CANNED_FIRST;
      return reply(res, 200, { status: 'completed', error: null, usage: { input_tokens: 0, output_tokens: 0 }, output: [{ type: 'message', content: [{ type: 'output_text', text: CANNED_FIRST }] }] });
    }
    const worst = worstHi(raw.length, body.max_output_tokens ?? 4000);
    const why = state.stopped || (flag && !fs.existsSync(flag) ? 'the billing flag is gone' : '') || (state.hi + worst > cap ? 'the spend cap' : '');
    if (why) {
      state.stopped ||= why;
      call.status = 503;
      call.stopped = true;
      return reply(res, 503, { error: `eval stopped: ${why}` });
    }
    const t0 = performance.now();
    let status = 0;
    let text = '';
    let retryAfter = null;
    try {
      const r = await fetch(`${upstream}/v1/responses`, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${key}` }, body: raw, signal: AbortSignal.timeout(150_000) });
      status = r.status;
      retryAfter = r.headers.get('retry-after');
      text = await r.text();
    } catch (e) {
      call.error = e?.name ?? 'error';
    }
    call.status = status;
    call.secs = +((performance.now() - t0) / 1000).toFixed(2);
    let v = null;
    try {
      v = JSON.parse(text);
    } catch {}
    call.text = outputText(v);
    const u = usageOf(v);
    call.usage = u;
    Object.assign(call, u ? cost(u) : { lo: 0, hi: worst });
    state.n += 1;
    state.hi += call.hi;
    const u0 = u ?? { input: '?', cached: '?', output: '?', reasoning: '?' };
    const label = `writer${status === 200 ? '' : ` (HTTP ${status})`}`;
    fs.appendFileSync(ledger, `| ${state.n} | ${state.cur.label} | ${label} | ${new Date().toISOString().slice(11, 19)}Z | ${body.model} | ${u0.input} | ${u0.cached} | ${u0.output} | ${u0.reasoning} | ${call.lo.toFixed(4)} | ${call.hi.toFixed(4)} | ${state.hi.toFixed(4)} |\n`);
    res.writeHead(status || 502, { 'content-type': 'application/json', ...(retryAfter ? { 'retry-after': retryAfter } : {}) });
    res.end(text);
  });
  return new Promise((ok) => server.listen(0, '127.0.0.1', () => ok({ server, state, port: server.address().port })));
}

// ---- one run: a fresh copy of demo/app, a history made by the real engine, then the product's `suggest -` ----

const GIT_ENV = { PATH: process.env.PATH, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null', GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't' };

// Two turns of history, as the hooks of the product make them: turn 1 starts, the work is done, turn 1 stops, turn 2 starts with the claim.
function workspace(root, input, engine) {
  for (const d of ['state', 'home', 'th']) fs.mkdirSync(path.join(root, d));
  const ws = path.join(root, 'ws');
  fs.cpSync(path.join(REPO, 'demo/app'), ws, { recursive: true });
  fs.writeFileSync(path.join(ws, '.synthetic'), 'made-up demo content\n');
  const file = path.join(ws, 'server.js');
  const edit = (patch) => fs.writeFileSync(file, applyPatch(fs.readFileSync(file, 'utf8'), patch));
  edit(input.init);
  for (const a of [['init', '-q'], ['add', '-A'], ['commit', '-qm', 'init']]) execFileSync('git', a, { cwd: ws, env: GIT_ENV });
  const env = { PATH: process.env.PATH, HOME: path.join(root, 'home'), TORPOR_HOME: path.join(root, 'th') };
  const hook = (event, extra = {}) => {
    const r = spawnSync(engine, ['hook', event], { cwd: ws, env, input: JSON.stringify({ session_id: 's1', cwd: ws, hook_event_name: event, ...extra }), encoding: 'utf8' });
    assert.equal(r.status, 0, `engine hook ${event} failed: ${String(r.stderr).slice(0, 200)}`);
  };
  hook('SessionStart', { source: 'startup' });
  hook('UserPromptSubmit', { prompt: 'turn one work' });
  edit(input.turn1);
  hook('Stop', { stop_hook_active: false });
  hook('UserPromptSubmit', { prompt: input.words });
  return ws;
}

function suggest(bin, root, ws, port, words, engine) {
  const env = {
    PATH: process.env.PATH,
    HOME: path.join(root, 'home'),
    TORPOR_HOME: path.join(root, 'th'),
    TORPOR_MUSE_STATE: path.join(root, 'state'),
    TORPOR_BIN: engine,
    META_API_KEY: FAKE_KEY,
    TORPOR_MUSE_API_BASE: `http://127.0.0.1:${port}`,
    TORPOR_MUSE_FROM_HOOK: 's1',
    TORPOR_MUSE_WORDS: words,
  };
  return new Promise((ok) => {
    const child = execFile(bin, ['suggest', '-'], { cwd: ws, env, timeout: 300_000 }, (err, stdout) => ok({ exit: err ? (err.code ?? -1) : 0, out: String(stdout).split('\n')[0].slice(0, 200) }));
    child.stdin.end();
  });
}

function readResult(root) {
  const dir = path.join(root, 'state', 'ws');
  try {
    for (const d of fs.readdirSync(dir)) {
      const f = path.join(dir, d, 'result.json');
      if (fs.existsSync(f)) return JSON.parse(fs.readFileSync(f, 'utf8'));
    }
  } catch {}
  return null;
}

const sha256 = (f) => crypto.createHash('sha256').update(fs.readFileSync(f)).digest('hex');
const git = (...a) => spawnSync('git', a, { cwd: REPO, encoding: 'utf8' });

// A real run only starts from the files that were pushed: PREREG.md and this script equal the pushed ones, and the binaries are the ones PREREG names.
function guard(loop, bins) {
  const pre = fs.existsSync(PREREG) ? fs.readFileSync(PREREG, 'utf8') : null;
  if (!loop) {
    assert(pre, 'eval/gap/PREREG.md is missing');
    const files = ['eval/gap/PREREG.md', 'eval/gap/run.mjs'];
    assert.equal(git('ls-tree', '-r', '--name-only', PUSHED, '--', ...files).stdout.trim().split('\n').length, files.length, `the pre-registration is not on ${PUSHED}`);
    assert.equal(git('diff', '--quiet', PUSHED, '--', ...files).status, 0, `PREREG.md or run.mjs differs from ${PUSHED}`);
  }
  const hashes = {};
  for (const arm of ['base', 'head']) {
    hashes[arm] = sha256(bins[arm]);
    const want = pre && new RegExp(`^${arm}_sha256 = ([0-9a-f]{64})$`, 'm').exec(pre)?.[1];
    if (pre && !(loop && !want)) assert.equal(hashes[arm], want, `the ${arm} binary is not the one PREREG.md names`);
  }
  return hashes;
}

// Nothing machine-specific goes into a row.
const scrub = (s, root) => String(s ?? '').replaceAll(root, '<tmp>').replaceAll(REPO, '<repo>').replace(/\/home\/[^\s"'\\]*/g, '<path>');

export function load(file) {
  const lines = fs.readFileSync(file, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
  return { meta: lines[0].meta, rows: lines.slice(1) };
}

async function runAll(rowsFile) {
  const need = (n) => process.env[n] || assert.fail(`${n} is not set`);
  const bins = { base: need('EVAL_BASE_BIN'), head: need('EVAL_HEAD_BIN') };
  const engine = need('EVAL_ENGINE');
  const ledger = need('EVAL_LEDGER');
  const tmp = need('EVAL_TMP');
  const upstream = process.env.EVAL_UPSTREAM || API;
  const loop = /^http:\/\/127\.0\.0\.1:\d+$/.test(upstream);
  assert(loop || upstream === API, 'EVAL_UPSTREAM must be a loopback address');
  const key = loop ? 'rehearsal' : need('META_API_KEY');
  delete process.env.META_API_KEY; // the children get an explicit environment; this keeps it out of anything else
  assert(loop || fs.existsSync(FLAG), `${FLAG} is missing: no keyed call`);
  const hashes = guard(loop, bins);
  fs.mkdirSync(tmp, { recursive: true });
  const { server, state, port } = await startProxy({ ledger, upstream, key, flag: loop ? '' : FLAG });
  const started = new Date().toISOString();
  let meta = {
    upstream: loop ? 'mock' : 'api',
    harness: git('rev-parse', 'HEAD').stdout.trim(),
    base_sha256: hashes.base,
    head_sha256: hashes.head,
    engine: execFileSync(engine, ['version'], { encoding: 'utf8' }).trim(),
    cap_hi: CAP_HI,
    started,
    resumes: 0,
  };
  let rows = [];
  if (fs.existsSync(rowsFile)) {
    ({ meta, rows } = load(rowsFile));
    meta.resumes += 1;
  } else fs.writeFileSync(rowsFile, JSON.stringify({ meta }) + '\n');
  const done = new Set(rows.map((r) => r.p));
  let stopped = '';
  try {
    for (let p = 0; p < INPUTS.length * 2 * REPS; p++) {
      if (done.has(p)) continue;
      const { input, rep, arm } = plan(p);
      const root = fs.mkdtempSync(path.join(tmp, 'w-'));
      const ws = workspace(root, input, engine);
      state.cur = { calls: [], label: `${input.id} r${rep} ${arm}` };
      const r = await suggest(bins[arm], root, ws, port, input.words, engine);
      if (state.stopped) {
        stopped = state.stopped;
        break;
      }
      const res = readResult(root);
      const ans = lastAnswer(state.cur.calls);
      const sc = score(input, ans, res?.saved);
      const calls = state.cur.calls.filter((c) => c.real).map((c) => ({ status: c.status, secs: c.secs ?? null, usage: c.usage ?? null, lo: c.lo ?? 0, hi: c.hi ?? 0 }));
      const row = JSON.parse(scrub(JSON.stringify({ p, id: input.id, rep, arm, ...sc, reason: ans?.reason ?? null, outcome: res?.outcome ?? null, code: res?.code ?? null, message: res?.message ?? null, exit: r.exit, attempts: calls.length, calls }), root));
      rows.push(row);
      fs.appendFileSync(rowsFile, JSON.stringify(row) + '\n');
      process.stderr.write(`${p} ${input.id} r${rep} ${arm}: ${row.verdict} saved=${row.saved} junk=${row.junk} kept=${row.kept ?? '-'} ($${calls.reduce((a, c) => a + c.lo, 0).toFixed(4)} lo, cum hi ${state.hi.toFixed(4)})\n`);
    }
  } finally {
    server.close();
    meta.model = state.model;
    meta.finished = new Date().toISOString();
    meta.stopped = stopped || undefined;
    const lines = fs.readFileSync(rowsFile, 'utf8').split('\n');
    lines[0] = JSON.stringify({ meta });
    fs.writeFileSync(rowsFile, lines.join('\n'));
    ledgerTotal(ledger);
  }
  if (stopped) {
    process.stderr.write(`STOPPED (${stopped}): hi so far ${state.hi.toFixed(4)} of ${CAP_HI}; ${rows.length} of ${INPUTS.length * 2 * REPS} runs done\n`);
    process.exit(3);
  }
  process.exit(0);
}

// ---- the report ----

const sum = (a, k) => a.reduce((x, r) => x + (r[k] ?? 0), 0);

export function tallies(rows) {
  const t = (arm) => {
    const r = rows.filter((x) => x.arm === arm);
    return { arm, n: r.length, check: r.filter((x) => x.verdict === 'check').length, saved: sum(r, 'saved'), note: sum(r, 'note'), junk: sum(r, 'junk'), kept: sum(r, 'kept'), keptN: r.filter((x) => x.kept !== null).length };
  };
  return { base: t('base'), head: t('head') };
}

export function passLines(rows) {
  const { base: b, head: h } = tallies(rows);
  const lines = [
    { bar: 'HEAD junk at most 1 of 25', have: `${h.junk} of ${h.n}`, ok: h.junk <= 1 },
    { bar: 'HEAD kept at least BASE kept minus 1 (of 10)', have: `HEAD ${h.kept} of ${h.keptN}, BASE ${b.kept} of ${b.keptN}`, ok: h.kept >= b.kept - 1 },
    { bar: 'HEAD saved at least BASE saved minus 2 (of 25)', have: `HEAD ${h.saved} of ${h.n}, BASE ${b.saved} of ${b.n}`, ok: h.saved >= b.saved - 2 },
  ];
  return { b, h, lines, notReproduced: b.junk === 0, complete: b.n === INPUTS.length * REPS && h.n === INPUTS.length * REPS };
}

const cell = (s) => String(s ?? '').replace(/\|/g, '\\|').replace(/\s*\n\s*/g, ' ');

export function report(meta, rows) {
  const { b, h, lines, notReproduced, complete } = passLines(rows);
  const L = [];
  L.push('# S3g result: the writer names only what the developer said works', '');
  L.push('Made by `node eval/gap/run.mjs report eval/gap/rows.jsonl` from the recorded runs. The bars were fixed in PREREG.md before the first keyed call. Nothing here was re-run.', '');
  L.push(`Model ${meta.model}; engine ${meta.engine}; harness commit ${meta.harness}; BASE binary sha256 ${meta.base_sha256.slice(0, 16)}, HEAD binary sha256 ${meta.head_sha256.slice(0, 16)}; upstream ${meta.upstream}; ${meta.started} to ${meta.finished}${meta.resumes ? `; resumed ${meta.resumes} time(s)` : ''}.`, '');
  if (meta.stopped || !complete) L.push(`INCOMPLETE: ${meta.stopped ? `stopped (${meta.stopped}); ` : ''}BASE ${b.n} of 25 runs, HEAD ${h.n} of 25 runs.`, '');
  L.push('## Pass lines', '', '| bar | result | met |', '|---|---|---|');
  for (const l of lines) L.push(`| ${l.bar} | ${l.have} | ${l.ok ? 'yes' : 'NO'} |`);
  L.push('');
  if (notReproduced) L.push(`BASE junk is 0 of ${b.n}: bug not reproduced.`, '');
  L.push('## Tallies', '', '`note` = the check was saved and its reason is not empty (the case that puts a note in the AI\'s context). It is descriptive, not a pass line.', '');
  L.push('| input | arm | runs | verdict check | saved | note | junk | kept |', '|---|---|---|---|---|---|---|---|');
  for (const inp of [...INPUTS.map((i) => i.id), 'all']) {
    for (const arm of ['base', 'head']) {
      const r = rows.filter((x) => x.arm === arm && (inp === 'all' || x.id === inp));
      const kept = r.some((x) => x.kept !== null) ? `${sum(r, 'kept')} of ${r.filter((x) => x.kept !== null).length}` : '-';
      L.push(`| ${inp} | ${arm.toUpperCase()} | ${r.length} | ${r.filter((x) => x.verdict === 'check').length} | ${sum(r, 'saved')} | ${sum(r, 'note')} | ${sum(r, 'junk')} | ${kept} |`);
    }
  }
  L.push('', '## Every run', '', 'Sorted by input, arm and repetition. A reason scored from the last writer answer that parsed; `none` = no usable answer (scores 0 for junk, kept and saved).', '');
  L.push('| input | arm | rep | verdict | saved | junk | kept | engine said | reason |', '|---|---|---|---|---|---|---|---|---|');
  const order = (r) => INPUTS.findIndex((i) => i.id === r.id) * 100 + (r.arm === 'base' ? 0 : 50) + r.rep;
  for (const r of [...rows].sort((x, y) => order(x) - order(y))) L.push(`| ${r.id} | ${r.arm.toUpperCase()} | ${r.rep} | ${r.verdict} | ${r.saved} | ${r.junk} | ${r.kept ?? '-'} | ${cell(r.outcome)}${r.code ? ` (${cell(r.code)})` : ''} | ${cell(r.reason) || '(empty)'} |`);
  L.push('', `Calls: ${sum(rows.flatMap((r) => r.calls), 'lo').toFixed(4)} USD lo and ${sum(rows.flatMap((r) => r.calls), 'hi').toFixed(4)} USD hi over ${rows.reduce((a, r) => a + r.attempts, 0)} writer calls in the runs above (exact sums; the spend ledger adds its rounded rows and is the record of spend; cap ${CAP_HI.toFixed(2)} hi).`);
  return L.join('\n') + '\n';
}

// ---- self-check ----

const freePort = () => new Promise((ok) => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => ok(p)); }); });
const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));

// Starts a patched copy of demo/app, waits for it, runs `fn(url)`, stops it.
async function withApp(patches, fn) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'gap-app-'));
  try {
    let text = fs.readFileSync(path.join(REPO, 'demo/app/server.js'), 'utf8');
    for (const p of patches) text = applyPatch(text, p);
    fs.writeFileSync(path.join(dir, 'server.js'), text);
    const port = await freePort();
    const child = spawn('node', ['server.js', '--port', String(port)], { cwd: dir, stdio: 'ignore' });
    try {
      const url = `http://127.0.0.1:${port}`;
      for (let i = 0; i < 50; i++) {
        try {
          await fetch(`${url}/`);
          break;
        } catch {
          await sleep(100);
        }
      }
      await fn(url);
    } finally {
      child.kill();
    }
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

const LOGIN = { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ email: 'Sam.Demo@Torpor', password: 'demo-password-42' }) };

async function selftest() {
  // the inputs: A1 to A3 are the three claim lines of e2e/proof/scripts.mjs; PREREG.md quotes every input, key word and patch verbatim
  const { CLEAN_PROMPTS } = await import('../../e2e/proof/scripts.mjs');
  assert.deepEqual(INPUTS.slice(0, 3).map((i) => i.words), [CLEAN_PROMPTS[3], CLEAN_PROMPTS[9], CLEAN_PROMPTS[17]]);
  assert.deepEqual(INPUTS.map((i) => i.id), ['A1', 'A2', 'A3', 'C1', 'C2']);
  // the key words as the brief states them
  assert.deepEqual(INPUTS.map((i) => [i.junk, i.kept ?? null]), [
    [['show password', 'checkbox'], null],
    [['comment'], null],
    [['forgot', 'link'], null],
    [['show password', 'checkbox'], ['restart']],
    [['forgot', 'link'], ['30 minutes', 'expire', 'time', 'ends']],
  ]);
  assert.equal(INPUTS[3].words, "Login works now, and it still works after the server restarts. Please add a 'Show password' checkbox under the password field.");
  assert.equal(INPUTS[4].words, "The sign-in button works great, and the session ends after 30 minutes. Add a 'Forgot password?' link under it that points to #.");
  const text = fs.readFileSync(path.join(REPO, 'demo/app/server.js'), 'utf8');
  if (fs.existsSync(PREREG)) {
    const pre = fs.readFileSync(PREREG, 'utf8');
    for (const i of INPUTS) {
      assert(pre.includes(i.words), `PREREG lacks the words of ${i.id}`);
      assert(pre.includes(scoringLine(i)), `PREREG lacks the line: ${scoringLine(i)}`);
    }
    for (const patch of [HEADING, HEALTH, SESSION30]) for (const [from, to] of patch) assert(pre.includes(from.trimEnd()) && pre.includes(to.trimEnd()), `PREREG lacks a patch text: ${to.slice(0, 40)}`);
  }
  // the patches match once; the patched app still starts and does what the claim says
  for (const patch of [HEADING, HEALTH, SESSION30]) assert.notEqual(applyPatch(text, patch), text);
  await withApp([], async (url) => assert.equal((await fetch(`${url}/api/health`)).status, 404));
  await withApp([HEALTH], async (url) => {
    const r = await fetch(`${url}/api/health`);
    assert.equal(r.status, 200);
    assert.deepEqual(await r.json(), { ok: true });
  });
  await withApp([HEADING], async (url) => assert((await (await fetch(`${url}/`)).text()).includes('<h1>Welcome back</h1>')));
  await withApp([SESSION30], async (url) => {
    const login = await fetch(`${url}/api/login`, LOGIN);
    assert.equal(login.status, 200);
    const set = login.headers.get('set-cookie');
    assert(set.includes('Max-Age=1800'), set);
    const me = await fetch(`${url}/api/me`, { headers: { cookie: set.split(';')[0] } });
    assert.equal(me.status, 200);
    assert.equal((await me.json()).name, 'Sam');
    assert.equal((await fetch(`${url}/api/me`)).status, 401);
  });
  // the order: the arms alternate and every 10 runs each input meets each arm once
  const all = Array.from({ length: 50 }, (_, p) => plan(p));
  assert(all.every((x, p) => x.arm === (p % 2 ? 'head' : 'base')));
  for (const id of INPUTS.map((i) => i.id)) for (const arm of ['base', 'head']) assert.deepEqual(all.filter((x) => x.input.id === id && x.arm === arm).map((x) => x.rep).sort(), [1, 2, 3, 4, 5]);
  // the scorer: key words, the real gap, and the L4c reason (the fragments quoted from e2e/PROOF.md E2)
  const [A1, A2, A3, C1, C2] = INPUTS;
  const ans = (reason, verdict = 'check') => ({ verdict, reason });
  const l4c = ans("The saved check does not cover the 'Forgot password?' link, a new addition, not a claimed working feature.");
  for (const i of INPUTS) {
    for (const w of i.junk) assert.equal(score(i, ans(`Not covered: the ${w.toUpperCase()}.`), true).junk, 1, `${i.id} ${w}`);
    for (const w of i.kept ?? []) assert.equal(score(i, ans(`Not covered: the ${w.toUpperCase()}.`), true).kept, 1, `${i.id} ${w}`);
    assert.equal(score(i, ans('Nothing else is covered by these steps.'), true).junk, 0, i.id);
  }
  assert.equal(score(A3, l4c, true).junk, 1);
  assert.equal(score(C2, l4c, true).kept, 0);
  assert.equal(score(A3, l4c, true).saved, 1);
  assert.equal(score(A3, l4c, true).note, 1);
  assert.equal(score(A1, ans('The Show password checkbox is not covered.'), true).junk, 1);
  assert.equal(score(A1, ans('A CHECKBOX is not covered.'), true).junk, 1);
  assert.equal(score(A2, ans('The comment above checkLogin is not covered.'), true).junk, 1);
  assert.equal(score(A2, ans('A comment.'), true).kept, null);
  assert.equal(score(C1, ans('A restart of the server cannot be tested.'), true).kept, 1);
  assert.equal(score(C1, ans('A restart of the server cannot be tested.'), true).junk, 0);
  for (const w of ['30 minutes', 'expire', 'time', 'ends']) assert.equal(score(C2, ans(`Not covered: ${w}.`), true).kept, 1, w);
  assert.deepEqual(score(C2, ans(''), true), { verdict: 'check', saved: 1, junk: 0, kept: 0, note: 0 });
  assert.deepEqual(score(C2, ans('', 'check'), false), { verdict: 'check', saved: 0, junk: 0, kept: 0, note: 0 });
  assert.equal(score(A3, ans('a link request', 'not_a_claim'), false).saved, 0);
  assert.equal(score(A3, ans('a link request', 'not_a_claim'), true).saved, 0);
  assert.deepEqual(score(A3, null, undefined), { verdict: 'none', saved: 0, junk: 0, kept: null, note: 0 });
  // the last writer answer that parsed
  const call = (text, status = 200, kind = 'check', real = true) => ({ kind, real, status, text });
  assert.equal(lastAnswer([call(JSON.stringify(ans('one'))), call(JSON.stringify(ans('two')))]).reason, 'two');
  assert.equal(lastAnswer([call(JSON.stringify(ans('one'))), call('not json')]).reason, 'one');
  assert.equal(lastAnswer([call(JSON.stringify(ans('one'))), call(JSON.stringify(ans('two')), 500)]).reason, 'one');
  assert.equal(lastAnswer([call(JSON.stringify({ verdict: 'check' }))]), null);
  assert.equal(lastAnswer([call(JSON.stringify(ans('x')), 200, 'first', false)]), null);
  assert.equal(lastAnswer([]), null);
  // the report: pass lines and "bug not reproduced"
  const row = (id, arm, rep, over = {}) => ({ p: 0, id, arm, rep, verdict: 'check', saved: 1, junk: 0, kept: id[0] === 'C' ? 1 : null, note: 0, reason: '', outcome: 'advisory', code: '', message: '', exit: 0, attempts: 1, calls: [{ status: 200, secs: 1, usage: null, lo: 0.005, hi: 0.006 }], ...over });
  const mk = (f) => INPUTS.flatMap((i) => ['base', 'head'].flatMap((arm) => [1, 2, 3, 4, 5].map((rep) => row(i.id, arm, rep, f(i, arm, rep)))));
  const meta = { model: 'm', engine: 'torpor 0', harness: 'h', base_sha256: 'a'.repeat(64), head_sha256: 'b'.repeat(64), upstream: 'mock', started: 's', finished: 'f', resumes: 0 };
  const clean = mk(() => ({}));
  assert(report(meta, clean).includes('bug not reproduced'));
  assert(passLines(clean).lines.every((l) => l.ok) && passLines(clean).complete);
  const bugged = mk((i, arm, rep) => (arm === 'base' && rep <= 3 ? { junk: 1, note: 1, reason: 'The link | is\nnew' } : {}));
  assert(!report(meta, bugged).includes('bug not reproduced'));
  assert(report(meta, bugged).includes('The link \\| is new'));
  assert.equal(passLines(bugged).b.junk, 15);
  const edge = (f) => passLines(mk(f)).lines.map((l) => l.ok);
  assert.deepEqual(edge((i, arm, rep) => (arm === 'head' && i.id === 'A1' && rep === 1 ? { junk: 1 } : {})), [true, true, true]);
  assert.deepEqual(edge((i, arm, rep) => (arm === 'head' && i.id === 'A1' && rep <= 2 ? { junk: 1 } : {})), [false, true, true]);
  assert.deepEqual(edge((i, arm, rep) => (arm === 'head' && i.id === 'C1' && rep === 1 ? { kept: 0 } : {})), [true, true, true]);
  assert.deepEqual(edge((i, arm, rep) => (arm === 'head' && i.id[0] === 'C' && rep <= 2 ? { kept: 0 } : {})), [true, false, true]);
  assert.deepEqual(edge((i, arm, rep) => (arm === 'head' && rep <= 2 ? { saved: 0 } : {})), [true, true, false]);
  assert.deepEqual(edge((i, arm, rep) => (arm === 'head' && i.id === 'A1' && rep <= 2 ? { saved: 0 } : {})), [true, true, true]);
  assert(!passLines(clean.slice(1)).complete);
  // scrub
  // the home path is split so that no literal one sits in this committed file
  assert.equal(scrub('see /ho' + 'me/someone/x/y.js and /tmp/w-1/ws', '/tmp/w-1'), 'see <path> and <tmp>/ws');
  // the stand-in and the ledger: canned first question; forwarded writer call with a row; the flag; the cap
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'gap-self-'));
  try {
    const up = http.createServer(async (req, res) => {
      for await (const c of req); // drain
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ status: 'completed', usage: { input_tokens: 1000, input_tokens_details: { cached_tokens: 400 }, output_tokens: 300, output_tokens_details: { reasoning_tokens: 100 } }, output: [{ type: 'message', content: [{ type: 'output_text', text: JSON.stringify(ans('Restart.')) }] }] }));
    });
    await new Promise((ok) => up.listen(0, '127.0.0.1', ok));
    const flag = path.join(dir, 'flag');
    fs.writeFileSync(flag, '');
    const ledger = path.join(dir, 'ledger.md');
    const px = await startProxy({ ledger, upstream: `http://127.0.0.1:${up.address().port}`, key: 'k', flag, cap: 0.5 });
    const post = async (name) => {
      const r = await fetch(`http://127.0.0.1:${px.port}/v1/responses`, { method: 'POST', body: JSON.stringify({ model: 'm', max_output_tokens: 6000, text: { format: { name } }, input: 'x'.repeat(1000) }) });
      return { status: r.status, body: await r.json() };
    };
    px.state.cur = { calls: [], label: 'A1 r1 base' };
    assert.equal((await post(FIRST)).body.output[0].content[0].text, CANNED_FIRST);
    assert.equal(ledgerRows(ledger).length, 0);
    assert.equal((await post(CHECK)).status, 200);
    const rows = ledgerRows(ledger);
    assert.equal(rows.length, 1);
    const want = cost({ input: 1000, cached: 400, output: 300, reasoning: 100 });
    assert(Math.abs(rows[0].lo - +want.lo.toFixed(4)) < 1e-9 && Math.abs(rows[0].hi - +want.hi.toFixed(4)) < 1e-9);
    assert.deepEqual(px.state.cur.calls.map((c) => [c.kind, c.real, c.status]), [['first', false, undefined], ['check', true, 200]]);
    assert.equal(lastAnswer(px.state.cur.calls).reason, 'Restart.');
    assert.equal((await post('other_format')).status, 404);
    fs.rmSync(flag);
    assert.equal((await post(CHECK)).status, 503);
    assert.equal(px.state.stopped, 'the billing flag is gone');
    assert.equal(ledgerRows(ledger).length, 1);
    px.server.close();
    ledgerTotal(ledger);
    assert(fs.readFileSync(ledger, 'utf8').includes('Total: 1 calls, $ lo '));
    assert.equal(ledgerOpen(ledger).n, 1);
    const cap = await startProxy({ ledger: path.join(dir, 'cap.md'), upstream: `http://127.0.0.1:${up.address().port}`, key: 'k', cap: 0.05 });
    cap.state.cur = { calls: [], label: 'x' };
    const r = await fetch(`http://127.0.0.1:${cap.port}/v1/responses`, { method: 'POST', body: JSON.stringify({ model: 'm', max_output_tokens: 6000, text: { format: { name: CHECK } }, input: 'x' }) });
    assert.equal(r.status, 503);
    assert.equal(cap.state.stopped, 'the spend cap');
    cap.server.close();
    up.close();
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
  assert(HEADER.includes('| # | run | call | time UTC | model | input | cached | output | reasoning | $ lo | $ hi | cumulative $ hi |'));
  process.stdout.write('selftest: OK\n');
  process.exit(0);
}

const [cmd, ...args] = process.argv.slice(2);
if (import.meta.url === `file://${process.argv[1]}`) {
  if (cmd === 'selftest') await selftest();
  else if (cmd === 'run' && args.length === 1) await runAll(args[0]);
  else if (cmd === 'report' && args.length === 1) {
    const { meta, rows } = load(args[0]);
    process.stdout.write(report(meta, rows));
  } else {
    process.stderr.write('usage: run.mjs selftest | run <rows.jsonl> | report <rows.jsonl>\n');
    process.exit(2);
  }
}

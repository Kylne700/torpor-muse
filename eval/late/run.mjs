#!/usr/bin/env node
// The eval of the writer's view of code late in a file (stage S3x, branch writer-sees-late-code). Pre-registered in PREREG.md.
// It reuses the harness of eval/gap/run.mjs (the loopback stand-in with the spend ledger, the scorer, the patch helper); this file holds
// the inputs, the order, the guard, the pass lines and the report of this stage. Subcommands:
//   selftest               the inputs against PREREG.md, the four app patches, the order, the scorer, the pass lines, the report and the
//                          ledger (offline, no money)
//   seen                   offline, no money: what the writer is shown of the code each claim is about, for both binaries
//   run <rows.jsonl>       the 60 runs: run p = 0..59 is plan(p). Every run is the real `torpor-muse suggest -` (the product's hook form)
//                          in a fresh copy of demo/app with a history made by the real engine. A loopback stand-in answers the first
//                          question with a canned `claim` and sends the writer's call on to the real API (key from META_API_KEY; no child
//                          process sees it), writes one ledger row per forwarded response and refuses to forward when the billing flag is
//                          gone or when the ledger's hi total plus the worst case of the call would pass the cap.
//   report <rows.jsonl>    RESULT.md as markdown on stdout (tallies, the pass lines and every run with its reason text)
// Environment: EVAL_BASE_BIN, EVAL_HEAD_BIN (the two torpor-muse binaries), EVAL_ENGINE (the torpor engine), EVAL_TMP (a folder for the
// work copies); for run also EVAL_LEDGER (the spend file) and META_API_KEY, or EVAL_UPSTREAM=http://127.0.0.1:<port> (mock/server.mjs)
// for a rehearsal that needs no key and no flag.
import fs from 'node:fs';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import assert from 'node:assert/strict';
import { execFile, execFileSync, spawn, spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { cost } from '../first-pass/run.mjs';
import { HEADING, HEALTH, INPUTS as GAP, applyPatch, lastAnswer, score, scoringLine, startProxy, ledgerRows, ledgerTotal, load } from '../gap/run.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const PREREG = path.join(HERE, 'PREREG.md');
const PUSHED = 'origin/writer-sees-late-code';
const PINNED = ['eval/late/PREREG.md', 'eval/late/run.mjs', 'eval/gap/run.mjs', 'eval/first-pass/run.mjs'];
const CAP_HI = 3.0;
const API = 'https://api.meta.ai';
const CHECK = 'torpor_muse_check';
const FAKE_KEY = 'LLM_eval-stand-in-key-0123456789abcdef';
const FLAG = '/tmp/claude-1000/meta-billing-ok';
const REPS = 5;

// ---- the inputs and the app patches ----

// A route goes in front of the 404 handler (the line HEALTH puts its route in front of), which starts at byte 6451 of demo/app/server.js:
// every route below sits past the first 6,000 bytes that the BASE writer reads. Text replacements; every `from` must match exactly once.
const ANCHOR = HEALTH[0][0];
const route = (...lines) => lines.map((l) => `${l}\n`).join('');
const before = (r) => [[ANCHOR, `${r}\n${ANCHOR}`]];
export const routeOf = (patch) => patch[0][1].slice(0, -(ANCHOR.length + 1));
export const GREET = before(route(
  '  if (req.method === "GET" && path === "/api/greet") {',
  '    const name = new URL(rawUrl, "http://localhost").searchParams.get("name");',
  '    sendJson(res, 200, { greeting: "Hello, " + (name ? name : "friend") + "!" });',
  '    return;',
  '  }',
));
export const SHOUT = before(route(
  '  if (req.method === "POST" && path === "/api/shout") {',
  '    const chunks = [];',
  '    req.on("data", (chunk) => chunks.push(chunk));',
  '    req.on("end", () => {',
  '      let parsed;',
  '      try {',
  '        parsed = JSON.parse(Buffer.concat(chunks).toString("utf8"));',
  '      } catch (err) {',
  '        sendJson(res, 400, { ok: false });',
  '        return;',
  '      }',
  '      if (parsed === null || typeof parsed.text !== "string" || parsed.text === "") {',
  '        sendJson(res, 400, { ok: false });',
  '        return;',
  '      }',
  '      sendJson(res, 200, { ok: true, text: parsed.text.toUpperCase() + "!" });',
  '    });',
  '    return;',
  '  }',
));
export const SUM = before(route(
  '  if (req.method === "GET" && path === "/api/sum") {',
  '    const params = new URL(rawUrl, "http://localhost").searchParams;',
  '    const a = Number(params.get("a"));',
  '    const b = Number(params.get("b"));',
  '    if (params.get("a") === null || params.get("b") === null || !Number.isFinite(a) || !Number.isFinite(b)) {',
  '      sendJson(res, 400, { ok: false });',
  '      return;',
  '    }',
  '    sendJson(res, 200, { sum: a + b });',
  '    return;',
  '  }',
));

// `kind` late: the code the claim is about sits past the first 6,000 bytes of server.js (turn-1 edit = the route). early: it sits in the
// first 6,000 bytes (turn-1 edit = the heading). `junk`: key words that mean the reason names the request.
export const INPUTS = [
  { id: 'A2', kind: 'late', words: 'The health route works. Add a short comment above checkLogin that says what it returns.', turn1: HEALTH, junk: ['comment'] },
  { id: 'N1', kind: 'late', words: "The greeting route works. Add a 'Show password' checkbox under the password field.", turn1: GREET, junk: ['show password', 'checkbox'] },
  { id: 'N2', kind: 'late', words: "The shout route works. Add a 'Forgot password?' link under the sign-in button that points to #.", turn1: SHOUT, junk: ['forgot', 'link'] },
  { id: 'N3', kind: 'late', words: "The sum route works. Change the page title to 'Sign in'.", turn1: SUM, junk: ['title'] },
  { id: 'A1', kind: 'early', words: "Login works now, thanks. Please add a 'Show password' checkbox under the password field.", turn1: HEADING, junk: ['show password', 'checkbox'] },
  { id: 'A3', kind: 'early', words: "The sign-in button works great. Add a 'Forgot password?' link under it that points to #.", turn1: HEADING, junk: ['forgot', 'link'] },
];
const RUNS = INPUTS.length * 2 * REPS;
const isLate = (id) => INPUTS.find((i) => i.id === id).kind === 'late';

// Run p (0..59): the arms alternate (BASE on even p), and in every 12 consecutive runs each input meets each arm once (the shift flips
// every 6 runs; a plain p mod 6 would give each input one arm only, because 6 is even).
export const plan = (p) => ({ p, input: INPUTS[(p + (Math.floor(p / INPUTS.length) % 2)) % INPUTS.length], rep: Math.floor(p / (2 * INPUTS.length)) + 1, arm: p % 2 === 0 ? 'base' : 'head' });

// ---- the spend ledger (the stand-in of eval/gap/run.mjs writes the rows) ----

const HEADER = `# S3x writer-sees-late-code spend ledger (cap $${CAP_HI.toFixed(2)} hi for the whole stage; every model call counts, failed ones too)

Model: muse-spark-1.3 (Meta API, standard-priced id), called by the real torpor-muse binary (BASE 570bc26 and HEAD 4c7a5f8) through a loopback stand-in (eval/gap/run.mjs, used by eval/late/run.mjs) that adds the key and writes one row per forwarded response; reasoning effort low. The first question is answered by the stand-in with a canned claim and costs nothing (no row).
Prices per 1M tokens: cached input $0.15, input $1.25, output $4.25.
lo = (input - cached) x 1.25 + cached x 0.15 + output x 4.25; hi = input x 1.25 + cached x 0.15 + (output + reasoning) x 4.25 (an upper bound: it counts the cached tokens twice and reasoning again where it is already inside output). lo is the best estimate; the cap is checked against hi. A call that gave no usage (an HTTP error, a timeout) is charged its worst case in hi and nothing in lo.

| # | run | call | time UTC | model | input | cached | output | reasoning | $ lo | $ hi | cumulative $ hi |
|---|-----|------|----------|-------|-------|--------|--------|-----------|------|------|-----------------|
`;
// startProxy writes the header of eval/gap/run.mjs when the file is missing, so this stage's header goes in first.
const openLedger = (file) => fs.existsSync(file) || fs.writeFileSync(file, HEADER);

// ---- one run: a fresh copy of demo/app, a history made by the real engine, then the product's `suggest -` ----

const GIT_ENV = { PATH: process.env.PATH, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null', GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't' };

// Two turns of history, as the hooks of the product make them: turn 1 starts, the work is done, turn 1 stops, turn 2 starts with the claim.
function workspace(root, input, engine) {
  for (const d of ['state', 'home', 'th']) fs.mkdirSync(path.join(root, d));
  const ws = path.join(root, 'ws');
  fs.cpSync(path.join(REPO, 'demo/app'), ws, { recursive: true });
  fs.writeFileSync(path.join(ws, '.synthetic'), 'made-up demo content\n');
  const file = path.join(ws, 'server.js');
  for (const a of [['init', '-q'], ['add', '-A'], ['commit', '-qm', 'init']]) execFileSync('git', a, { cwd: ws, env: GIT_ENV });
  const env = { PATH: process.env.PATH, HOME: path.join(root, 'home'), TORPOR_HOME: path.join(root, 'th') };
  const hook = (event, extra = {}) => {
    const r = spawnSync(engine, ['hook', event], { cwd: ws, env, input: JSON.stringify({ session_id: 's1', cwd: ws, hook_event_name: event, ...extra }), encoding: 'utf8' });
    assert.equal(r.status, 0, `engine hook ${event} failed: ${String(r.stderr).slice(0, 200)}`);
  };
  hook('SessionStart', { source: 'startup' });
  hook('UserPromptSubmit', { prompt: 'turn one work' });
  fs.writeFileSync(file, applyPatch(fs.readFileSync(file, 'utf8'), input.turn1));
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
  const t0 = performance.now();
  return new Promise((ok) => {
    const child = execFile(bin, ['suggest', '-'], { cwd: ws, env, timeout: 300_000 }, (err, stdout) => ok({ exit: err ? (err.code ?? -1) : 0, timedOut: err?.killed === true, secs: +((performance.now() - t0) / 1000).toFixed(1), out: String(stdout).split('\n')[0].slice(0, 200) }));
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

// A real run only starts from the files that were pushed: PREREG.md, this script and the two harness files it imports equal the pushed
// ones, and the binaries are the ones PREREG names.
function guard(loop, bins) {
  const pre = fs.existsSync(PREREG) ? fs.readFileSync(PREREG, 'utf8') : null;
  if (!loop) {
    assert(pre, 'eval/late/PREREG.md is missing');
    assert.equal(git('ls-tree', '-r', '--name-only', PUSHED, '--', ...PINNED).stdout.trim().split('\n').length, PINNED.length, `the pre-registration is not on ${PUSHED}`);
    assert.equal(git('diff', '--quiet', PUSHED, '--', ...PINNED).status, 0, `PREREG.md or a harness file differs from ${PUSHED}`);
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

const need = (n) => process.env[n] || assert.fail(`${n} is not set`);

async function runAll(rowsFile) {
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
  openLedger(ledger);
  const { server, state, port } = await startProxy({ ledger, upstream, key, flag: loop ? '' : FLAG, cap: CAP_HI });
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
    for (let p = 0; p < RUNS; p++) {
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
      const row = JSON.parse(scrub(JSON.stringify({ p, id: input.id, rep, arm, ...sc, reason: ans?.reason ?? null, outcome: res?.outcome ?? null, code: res?.code ?? null, message: res?.message ?? null, exit: r.exit, timedOut: r.timedOut, secs: r.secs, attempts: calls.length, calls }), root));
      rows.push(row);
      fs.appendFileSync(rowsFile, JSON.stringify(row) + '\n');
      process.stderr.write(`${p} ${input.id} r${rep} ${arm}: ${row.verdict} saved=${row.saved} junk=${row.junk}${r.timedOut ? ' TIMEOUT' : ''} ($${calls.reduce((a, c) => a + c.lo, 0).toFixed(4)} lo, cum hi ${state.hi.toFixed(4)})\n`);
    }
  } finally {
    server.close();
    meta.model = state.model;
    meta.finished = new Date().toISOString();
    meta.stopped = stopped || undefined;
    if (stopped) (meta.stops ??= []).push({ at: meta.finished, reason: stopped });
    const lines = fs.readFileSync(rowsFile, 'utf8').split('\n');
    lines[0] = JSON.stringify({ meta });
    fs.writeFileSync(rowsFile, lines.join('\n'));
    ledgerTotal(ledger, CAP_HI);
  }
  if (stopped) {
    process.stderr.write(`STOPPED (${stopped}): hi so far ${state.hi.toFixed(4)} of ${CAP_HI}; ${rows.length} of ${RUNS} runs done\n`);
    process.exit(3);
  }
  process.exit(0);
}

// ---- what the writer is shown (offline, against mock/server.mjs) ----

async function startMock(dir) {
  const log = path.join(dir, 'mock.log');
  const text = path.join(dir, 'mock-text.txt');
  fs.writeFileSync(text, JSON.stringify({ verdict: 'cannot_check', reason: 'seen', name: '', why: '', start_script: '', ready_path: '', node_env: '', steps: [] }));
  const child = spawn('node', [path.join(REPO, 'mock/server.mjs'), '--port', '0', '--fixture', 'ok', '--delay-ms', '0', '--output-text-file', text, '--log', log], { stdio: ['ignore', 'pipe', 'ignore'] });
  const port = await new Promise((ok) => child.stdout.once('data', (d) => ok(+String(d).trim().split(' ')[1])));
  return { port, log, stop: () => child.kill() };
}

// The repo_files block of the writer's request in the mock's log.
export function shownOf(logFile) {
  for (const l of fs.readFileSync(logFile, 'utf8').split('\n').filter(Boolean)) {
    let b;
    try {
      b = JSON.parse(JSON.parse(l).body);
    } catch {
      continue;
    }
    if (b?.text?.format?.name !== CHECK) continue;
    const body = String(b.input);
    const open = body.indexOf('<<<DATA repo_files nonce=');
    const from = body.indexOf('>>>\n', open) + 4;
    return body.slice(from, body.indexOf('\n<<<END nonce=', from));
  }
  return null;
}

// Whether the code a late claim is about is in what the writer was shown: the whole block in one piece, and how many of the lines that
// only the route has (not a `return;` or a `}` that the rest of the file has too) are there.
function inExcerpt(input, shown) {
  const original = new Set(fs.readFileSync(path.join(REPO, 'demo/app/server.js'), 'utf8').split('\n'));
  const own = routeOf(input.turn1).split('\n').filter((l) => l.trim() && !original.has(l));
  const there = new Set(shown.split('\n'));
  return `${shown.includes(routeOf(input.turn1).trimEnd()) ? 'whole block' : 'block not whole'}; ${own.filter((l) => there.has(l)).length} of ${own.length} route-only lines`;
}

async function seen() {
  const bins = { base: need('EVAL_BASE_BIN'), head: need('EVAL_HEAD_BIN') };
  const engine = need('EVAL_ENGINE');
  const tmp = need('EVAL_TMP');
  fs.mkdirSync(tmp, { recursive: true });
  process.stdout.write('input kind  arm  excerpt bytes  the route block in it\n');
  for (const input of INPUTS) {
    const shown = {};
    for (const arm of ['base', 'head']) {
      const root = fs.mkdtempSync(path.join(tmp, 's-'));
      const ws = workspace(root, input, engine);
      const mock = await startMock(root);
      await suggest(bins[arm], root, ws, mock.port, input.words, engine);
      mock.stop();
      shown[arm] = shownOf(mock.log);
      assert(shown[arm] !== null, `${input.id} ${arm}: the writer was not asked`);
      const note = input.kind === 'early' ? `excerpt ${arm === 'base' ? 'is the BASE one' : shown.head === shown.base ? 'is identical to the BASE one' : 'DIFFERS from the BASE one'}` : inExcerpt(input, shown[arm]);
      process.stdout.write(`${input.id.padEnd(5)} ${input.kind.padEnd(5)} ${arm.padEnd(4)} ${String(shown[arm].length).padStart(13)}  ${note}\n`);
    }
  }
  process.exit(0);
}

// ---- the report ----

const sum = (a, k) => a.reduce((x, r) => x + (r[k] ?? 0), 0);
const cell = (s) => String(s ?? '').replace(/\|/g, '\\|').replace(/\s*\n\s*/g, ' ');

export function passLines(rows) {
  const part = (arm, late) => rows.filter((r) => r.arm === arm && isLate(r.id) === late);
  const [lb, lh, cb, ch] = [part('base', true), part('head', true), part('base', false), part('head', false)];
  const a2 = rows.filter((r) => r.arm === 'head' && r.id === 'A2');
  const [sb, sh, tb, th, jh] = [sum(lb, 'saved'), sum(lh, 'saved'), sum(cb, 'saved'), sum(ch, 'saved'), sum(a2, 'junk')];
  const lines = [
    { bar: 'HEAD saved at least 14 of 20 on the 4 late inputs', have: `${sh} of ${lh.length}`, ok: sh >= 14 },
    { bar: 'HEAD saved at least BASE saved plus 8 on the 4 late inputs', have: `HEAD ${sh}, BASE ${sb} (of ${lh.length} each)`, ok: sh >= sb + 8 },
    { bar: 'on the 2 controls HEAD saved at least BASE saved minus 2 (of 10)', have: `HEAD ${th} of ${ch.length}, BASE ${tb} of ${cb.length}`, ok: th >= tb - 2 },
    { bar: 'HEAD junk on A2 at most 1 of 5', have: `${jh} of ${a2.length}`, ok: jh <= 1 },
  ];
  const full = (arm) => INPUTS.every((i) => rows.filter((r) => r.arm === arm && r.id === i.id).length === REPS);
  return { lines, notReproduced: sb >= 14, complete: rows.length === RUNS && full('base') && full('head') };
}

export function report(meta, rows) {
  const { lines, notReproduced, complete } = passLines(rows);
  const L = [];
  L.push('# S3x result: the writer sees the code a claim is about, also far into a file', '');
  L.push('Made by `node eval/late/run.mjs report eval/late/rows.jsonl` from the recorded runs. The bars were fixed in PREREG.md before the first keyed call. Nothing here was re-run.', '');
  L.push(`Model ${meta.model}; engine ${meta.engine}; harness commit ${meta.harness}; BASE binary sha256 ${meta.base_sha256.slice(0, 16)}, HEAD binary sha256 ${meta.head_sha256.slice(0, 16)}; upstream ${meta.upstream}; ${meta.started} to ${meta.finished}${meta.resumes ? `; resumed ${meta.resumes} time(s)` : ''}${meta.stops?.length ? `; stops: ${meta.stops.map((x) => `${x.at} ${x.reason}`).join(', ')}` : ''}.`, '');
  if (meta.stopped || !complete) L.push(`INCOMPLETE: ${meta.stopped ? `stopped (${meta.stopped}); ` : ''}${rows.filter((r) => r.arm === 'base').length} BASE runs and ${rows.filter((r) => r.arm === 'head').length} HEAD runs of ${RUNS / 2} each.`, '');
  L.push('## Pass lines', '', '| bar | result | met |', '|---|---|---|');
  for (const l of lines) L.push(`| ${l.bar} | ${l.have} | ${l.ok ? 'yes' : 'NO'} |`);
  L.push('');
  if (notReproduced) L.push('BASE saved 14 or more of 20 on the late inputs: gap not reproduced.', '');
  const timed = rows.filter((r) => r.timedOut);
  L.push(timed.length ? `Runs ended by the 300 s timeout: ${timed.map((r) => `${r.id} ${r.arm} r${r.rep}`).join(', ')}.` : 'No run was ended by a timeout.', '');
  L.push('## Tallies', '', '`saved` = the verdict is check and the engine saved a check. `note` = saved and the reason is not empty (the case that puts a note in the AI\'s context). `junk` = the reason names the request (key words of PREREG.md).', '');
  L.push('| input | kind | arm | runs | verdict check | saved | note | junk |', '|---|---|---|---|---|---|---|---|');
  const groups = [...INPUTS.map((i) => [i.id, i.kind, (r) => r.id === i.id]), ['late total', 'late', (r) => isLate(r.id)], ['controls total', 'early', (r) => !isLate(r.id)]];
  for (const [name, kind, f] of groups) {
    for (const arm of ['base', 'head']) {
      const r = rows.filter((x) => x.arm === arm && f(x));
      L.push(`| ${name} | ${kind} | ${arm.toUpperCase()} | ${r.length} | ${r.filter((x) => x.verdict === 'check').length} | ${sum(r, 'saved')} | ${sum(r, 'note')} | ${sum(r, 'junk')} |`);
    }
  }
  L.push('', '## Every run', '', 'Sorted by input, arm and repetition. A reason is scored from the last writer answer that parsed; `none` = no usable answer (scores 0 for junk and saved).', '');
  L.push('| input | arm | rep | verdict | saved | junk | engine said | seconds | reason |', '|---|---|---|---|---|---|---|---|---|');
  const order = (r) => INPUTS.findIndex((i) => i.id === r.id) * 100 + (r.arm === 'base' ? 0 : 50) + r.rep;
  for (const r of [...rows].sort((x, y) => order(x) - order(y))) L.push(`| ${r.id} | ${r.arm.toUpperCase()} | ${r.rep} | ${r.verdict} | ${r.saved} | ${r.junk} | ${cell(r.outcome)}${r.code ? ` (${cell(r.code)})` : ''} | ${r.secs ?? '-'}${r.timedOut ? ' TIMEOUT' : ''} | ${cell(r.reason) || '(empty)'} |`);
  const calls = rows.flatMap((r) => r.calls);
  L.push('', `Calls: ${sum(calls, 'lo').toFixed(4)} USD lo and ${sum(calls, 'hi').toFixed(4)} USD hi over ${sum(rows, 'attempts')} writer calls in the runs above (exact sums; the spend ledger adds its rounded rows and is the record of spend; cap ${CAP_HI.toFixed(2)} hi).`);
  return L.join('\n') + '\n';
}

// ---- self-check ----

const freePort = () => new Promise((ok) => { const s = net.createServer(); s.listen(0, '127.0.0.1', () => { const p = s.address().port; s.close(() => ok(p)); }); });
const sleep = (ms) => new Promise((ok) => setTimeout(ok, ms));

// Starts a patched copy of demo/app, waits for it, runs `fn(url)`, stops it.
async function withApp(patches, fn) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'late-app-'));
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

async function selftest() {
  // the inputs: A1 to A3 are the inputs of the same name in eval/gap; the key words as the brief states them
  assert.deepEqual(INPUTS.map((i) => i.id), ['A2', 'N1', 'N2', 'N3', 'A1', 'A3']);
  assert.deepEqual(INPUTS.map((i) => i.kind), ['late', 'late', 'late', 'late', 'early', 'early']);
  for (const i of INPUTS.filter((x) => x.id[0] === 'A')) {
    const g = GAP.find((x) => x.id === i.id);
    assert.deepEqual([i.words, i.junk, i.turn1 === g.turn1], [g.words, g.junk, true], i.id);
  }
  assert.deepEqual(INPUTS.map((i) => i.junk), [['comment'], ['show password', 'checkbox'], ['forgot', 'link'], ['title'], ['show password', 'checkbox'], ['forgot', 'link']]);
  const text = fs.readFileSync(path.join(REPO, 'demo/app/server.js'), 'utf8');
  const cut = text.indexOf(ANCHOR);
  assert(cut > 6_000 && Buffer.byteLength(text.slice(0, cut)) > 6_000, `the routes must sit past byte 6000, the anchor is at ${cut}`);
  const patches = [HEADING, HEALTH, GREET, SHOUT, SUM];
  if (fs.existsSync(PREREG)) {
    const pre = fs.readFileSync(PREREG, 'utf8');
    for (const i of INPUTS) {
      assert(pre.includes(i.words), `PREREG lacks the words of ${i.id}`);
      assert(pre.includes(scoringLine(i)), `PREREG lacks the line: ${scoringLine(i)}`);
    }
    for (const patch of patches) for (const [from, to] of patch) assert(pre.includes(from.trimEnd()) && pre.includes(to.trimEnd()), `PREREG lacks a patch text: ${to.slice(0, 40)}`);
    for (const arm of ['base', 'head']) assert(new RegExp(`^${arm}_sha256 = [0-9a-f]{64}$`, 'm').test(pre), `PREREG lacks the ${arm}_sha256 line`);
    for (const l of passLines([]).lines) assert(pre.includes(l.bar), `PREREG lacks the pass line: ${l.bar}`);
    assert(pre.includes('$3.00'));
  }
  // the patches match once; the patched app still starts and does what the claim says (and the route is absent before)
  for (const patch of patches) assert.notEqual(applyPatch(text, patch), text);
  for (const patch of [HEALTH, GREET, SHOUT, SUM]) assert(routeOf(patch).startsWith('  if (req.method === ') && routeOf(patch).endsWith('  }\n') && applyPatch(text, patch).includes(routeOf(patch) + '\n' + ANCHOR));
  const post = (url, body) => fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body });
  await withApp([], async (url) => {
    for (const p of ['/api/health', '/api/greet?name=Dana', '/api/sum?a=2&b=3']) assert.equal((await fetch(`${url}${p}`)).status, 404, p);
    assert.equal((await post(`${url}/api/shout`, '{"text":"hi"}')).status, 404);
  });
  await withApp([HEALTH], async (url) => {
    const r = await fetch(`${url}/api/health`);
    assert.equal(r.status, 200);
    assert.deepEqual(await r.json(), { ok: true });
  });
  await withApp([HEADING], async (url) => assert((await (await fetch(`${url}/`)).text()).includes('<h1>Welcome back</h1>')));
  await withApp([GREET], async (url) => {
    assert.deepEqual(await (await fetch(`${url}/api/greet?name=Dana`)).json(), { greeting: 'Hello, Dana!' });
    assert.deepEqual(await (await fetch(`${url}/api/greet`)).json(), { greeting: 'Hello, friend!' });
  });
  await withApp([SHOUT], async (url) => {
    const ok = await post(`${url}/api/shout`, '{"text":"hi"}');
    assert.equal(ok.status, 200);
    assert.deepEqual(await ok.json(), { ok: true, text: 'HI!' });
    for (const bad of ['{}', '{"text":""}', '{"text":5}', 'null', 'not json']) assert.equal((await post(`${url}/api/shout`, bad)).status, 400, bad);
  });
  await withApp([SUM], async (url) => {
    assert.deepEqual(await (await fetch(`${url}/api/sum?a=2&b=3.5`)).json(), { sum: 5.5 });
    for (const bad of ['', '?a=1', '?a=x&b=1']) assert.equal((await fetch(`${url}/api/sum${bad}`)).status, 400, bad);
  });
  // the order: the arms alternate and every 12 runs each input meets each arm once
  const all = Array.from({ length: RUNS }, (_, p) => plan(p));
  assert(all.every((x, p) => x.arm === (p % 2 ? 'head' : 'base')));
  for (const id of INPUTS.map((i) => i.id)) for (const arm of ['base', 'head']) assert.deepEqual(all.filter((x) => x.input.id === id && x.arm === arm).map((x) => x.rep).sort(), [1, 2, 3, 4, 5], `${id} ${arm}`);
  for (let b = 0; b < RUNS; b += 12) for (const arm of ['base', 'head']) assert.equal(new Set(all.slice(b, b + 12).filter((x) => x.arm === arm).map((x) => x.input.id)).size, 6);
  // the scorer (eval/gap's) on these key words
  const ans = (reason, verdict = 'check') => ({ verdict, reason });
  for (const i of INPUTS) {
    for (const w of i.junk) assert.equal(score(i, ans(`Not covered: the ${w.toUpperCase()}.`), true).junk, 1, `${i.id} ${w}`);
    assert.equal(score(i, ans('Nothing else is covered by these steps.'), true).junk, 0, i.id);
    assert.deepEqual(score(i, ans(''), true), { verdict: 'check', saved: 1, junk: 0, kept: null, note: 0 });
    assert.deepEqual(score(i, ans('', 'check'), false), { verdict: 'check', saved: 0, junk: 0, kept: null, note: 0 });
    assert.equal(score(i, ans('Cannot see it.', 'cannot_check'), true).saved, 0);
  }
  assert.equal(score(INPUTS[0], ans('The comment above checkLogin is not covered.'), true).junk, 1);
  assert.equal(score(INPUTS[0], ans('No health route path appears in the visible server.js.', 'cannot_check'), false).junk, 0);
  // the pass lines at their edges: shape(HEAD late saved, BASE late saved, HEAD controls saved, BASE controls saved, HEAD A2 junk)
  const row = (id, arm, rep, over = {}) => ({ p: 0, id, arm, rep, verdict: 'check', saved: 1, junk: 0, kept: null, note: 0, reason: '', outcome: 'advisory', code: '', message: '', exit: 0, timedOut: false, secs: 1, attempts: 1, calls: [{ status: 200, secs: 1, usage: null, lo: 0.005, hi: 0.006 }], ...over });
  const mk = (f) => INPUTS.flatMap((i) => ['base', 'head'].flatMap((arm) => [1, 2, 3, 4, 5].map((rep) => row(i.id, arm, rep, f(i, arm, rep)))));
  const nth = (i, rep) => (i.kind === 'late' ? INPUTS.indexOf(i) : INPUTS.indexOf(i) - 4) * 5 + rep;
  const shape = (hl, bl, hc, bc, junk = 0) => mk((i, arm, rep) => {
    const k = i.kind === 'late' ? (arm === 'head' ? hl : bl) : arm === 'head' ? hc : bc;
    return { saved: nth(i, rep) <= k ? 1 : 0, junk: arm === 'head' && i.id === 'A2' && rep <= junk ? 1 : 0, verdict: nth(i, rep) <= k ? 'check' : 'cannot_check' };
  });
  const oks = (rows) => passLines(rows).lines.map((l) => l.ok);
  assert.deepEqual(oks(shape(20, 0, 10, 10)), [true, true, true, true]);
  assert.deepEqual(oks(shape(14, 6, 10, 10)), [true, true, true, true]);
  assert.deepEqual(oks(shape(13, 5, 10, 10)), [false, true, true, true]);
  assert.deepEqual(oks(shape(14, 7, 10, 10)), [true, false, true, true]);
  assert.deepEqual(oks(shape(20, 0, 8, 10)), [true, true, true, true]);
  assert.deepEqual(oks(shape(20, 0, 7, 10)), [true, true, false, true]);
  assert.deepEqual(oks(shape(20, 0, 10, 10, 1)), [true, true, true, true]);
  assert.deepEqual(oks(shape(20, 0, 10, 10, 2)), [true, true, true, false]);
  assert.equal(passLines(shape(20, 14, 10, 10)).notReproduced, true);
  assert.equal(passLines(shape(20, 13, 10, 10)).notReproduced, false);
  const full = shape(20, 0, 10, 10);
  assert(passLines(full).complete && !passLines(full.slice(1)).complete);
  // the report: pass lines, tallies, the reason text with a pipe, a timeout, the stops
  const meta = { model: 'm', engine: 'torpor 0', harness: 'h', base_sha256: 'a'.repeat(64), head_sha256: 'b'.repeat(64), upstream: 'mock', started: 's', finished: 'f', resumes: 0 };
  const md = report(meta, mk((i, arm, rep) => (arm === 'base' && i.id === 'N1' && rep === 1 ? { saved: 0, junk: 1, timedOut: true, reason: 'The checkbox | is\nnew' } : {})));
  for (const part of ['| HEAD saved at least 14 of 20 on the 4 late inputs | 20 of 20 | yes |', '| late total | late | BASE | 20 | 20 | 19 |', '| controls total | early | HEAD | 10 | 10 | 10 |', 'The checkbox \\| is new', 'Runs ended by the 300 s timeout: N1 base r1.', '| N1 | BASE | 1 | check | 0 | 1 | advisory | 1 TIMEOUT |']) assert(md.includes(part), part);
  assert(report(meta, full).includes('No run was ended by a timeout.') && !report(meta, full).includes('INCOMPLETE'));
  assert(report({ ...meta, resumes: 1, stops: [{ at: 't1', reason: 'upstream HTTP 429' }], stopped: 'x' }, full.slice(1)).includes('resumed 1 time(s); stops: t1 upstream HTTP 429.'));
  assert(report({ ...meta, stopped: 'x' }, full.slice(1)).includes('INCOMPLETE: stopped (x); 29 BASE runs and 30 HEAD runs of 30 each.'));
  assert(report(meta, shape(20, 14, 10, 10)).includes('gap not reproduced'));
  assert(!report(meta, full).includes('gap not reproduced'));
  // scrub: the home path is split so that no literal one sits in this committed file
  assert.equal(scrub('see /ho' + 'me/someone/x/y.js and /tmp/w-1/ws', '/tmp/w-1'), 'see <path> and <tmp>/ws');
  // the ledger: this stage's header first, one forwarded row, the total line against the cap, and the engine-less stand-in answers
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'late-self-'));
  try {
    const up = http.createServer(async (req, res) => {
      for await (const c of req); // drain
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ status: 'completed', usage: { input_tokens: 1000, input_tokens_details: { cached_tokens: 400 }, output_tokens: 300, output_tokens_details: { reasoning_tokens: 100 } }, output: [{ type: 'message', content: [{ type: 'output_text', text: JSON.stringify(ans('Restart.')) }] }] }));
    });
    await new Promise((ok) => up.listen(0, '127.0.0.1', ok));
    const ledger = path.join(dir, 'ledger.md');
    openLedger(ledger);
    openLedger(ledger);
    assert(fs.readFileSync(ledger, 'utf8').startsWith('# S3x writer-sees-late-code spend ledger (cap $3.00 hi'));
    const px = await startProxy({ ledger, upstream: `http://127.0.0.1:${up.address().port}`, key: 'k', cap: CAP_HI });
    px.state.cur = { calls: [], label: 'N1 r1 head' };
    const r = await fetch(`http://127.0.0.1:${px.port}/v1/responses`, { method: 'POST', body: JSON.stringify({ model: 'm', max_output_tokens: 6000, text: { format: { name: CHECK } }, input: 'x'.repeat(1000) }) });
    assert.equal(r.status, 200);
    const rows = ledgerRows(ledger);
    const want = cost({ input: 1000, cached: 400, output: 300, reasoning: 100 });
    assert(rows.length === 1 && Math.abs(rows[0].lo - +want.lo.toFixed(4)) < 1e-9 && Math.abs(rows[0].hi - +want.hi.toFixed(4)) < 1e-9);
    assert(fs.readFileSync(ledger, 'utf8').includes('| 1 | N1 r1 head | writer |'));
    px.server.close();
    ledgerTotal(ledger, CAP_HI);
    assert(/Total: 1 calls, \$ lo [\d.]+, \$ hi [\d.]+ of the \$3\.00 cap/.test(fs.readFileSync(ledger, 'utf8')));
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
  else if (cmd === 'seen' && args.length === 0) await seen();
  else if (cmd === 'run' && args.length === 1) await runAll(args[0]);
  else if (cmd === 'report' && args.length === 1) {
    const { meta, rows } = load(args[0]);
    process.stdout.write(report(meta, rows));
  } else {
    process.stderr.write('usage: run.mjs selftest | seen | run <rows.jsonl> | report <rows.jsonl>\n');
    process.exit(2);
  }
}

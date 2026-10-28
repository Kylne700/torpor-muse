#!/usr/bin/env node
// The eval of the check-writer's first question (stage W2). Subcommands:
//   selftest                                   the set, the cost formula and the reading of an answer (offline, no money)
//   run <first|baseline> <prompts.md> <rows.jsonl>
//       One leg. For every prompt the real `torpor-muse suggest` runs in a fresh copy of demo/app with a scripted engine
//       and a loopback stand-in for the API. The stand-in sends the one request the leg measures on to the real API
//       (key from META_API_KEY; no child process sees it), answers the other with a canned free answer, and appends one
//       row per forwarded response to the spend ledger. It refuses to forward when the ledger's hi total plus the worst
//       case of the call would pass the cap. Leg first: the first question is real and today's call is canned
//       (cannot_check). Leg baseline: the first question is canned (claim) and today's call is real.
//   report <first.jsonl> [<baseline.jsonl>]    the tallies and the pass bars of the brief, as markdown
// Environment for run: EVAL_BIN (the torpor-muse binary), EVAL_LEDGER (the spend file), EVAL_TMP (a folder for the work
// copies) and META_API_KEY, or EVAL_UPSTREAM=http://127.0.0.1:<port> (mock/server.mjs) for a rehearsal that needs no key.
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import assert from 'node:assert/strict';
import { execFile, execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const CAP_HI = 1.5;
const API = 'https://api.meta.ai';
const FIRST = 'torpor_muse_first_pass';
const CHECK = 'torpor_muse_check';
const FAKE_KEY = 'LLM_eval-stand-in-key-0123456789abcdef';
const PROVEN =
  'Promise #1 "login works" saved. Proof: passed 5 of 5 on the version from the start of turn 2 and failed 2 of 2 on the version from the start of turn 1. In shadow for now: it blocks nothing yet.';
const FAKE_ENGINE = `#!/bin/sh
D="$(dirname "$0")"
case "$1 $2" in
  "version "*) echo "torpor 0.0.0-fake" ;;
  "status --json") echo '{"saving":true,"off":false,"last_turn":{"n":2}}' ;;
  "promise list") : ;;
  "gate "*) echo "The gate is on." ;;
  "promise add") cat >> "$D/adds.log"; cat "$D/add.out" ;;
  *) echo "fake engine: unknown command $*" 1>&2; exit 64 ;;
esac
`;
const CANNED = {
  [FIRST]: { verdict: 'claim', reason: '' },
  [CHECK]: { verdict: 'cannot_check', reason: 'eval stand-in', name: '', why: '', start_script: '', ready_path: '', node_env: '', steps: [] },
};

// ---- the set, the cost, the reading of an answer ----

// `id [b] | text`, `\n` inside the text is a line break. `C` = says something works, `N` = does not; a leading `x` = dry-run prompt.
export function parseSet(text) {
  const out = [];
  for (const line of text.split('\n')) {
    const m = /^(x?[CN]\d\d)( b)? \| (.*)$/.exec(line);
    if (m) out.push({ id: m[1], label: m[1].replace(/^x/, '')[0] === 'C' ? 'claim' : 'not', base: !!m[2], text: m[3].replaceAll('\\n', '\n') });
  }
  return out;
}

// Per 1M tokens: fresh input 1.25, cached input 0.15, output 4.25. lo is what the binary's own ledger records.
export function cost(u) {
  const lo = ((u.input - u.cached) * 1.25 + u.cached * 0.15 + u.output * 4.25) / 1e6;
  const hi = (u.input * 1.25 + u.cached * 0.15 + (u.output + u.reasoning) * 4.25) / 1e6;
  return { lo, hi };
}

// What a request can cost at most, in hi terms: a token is at least a byte, all of it cached or not, and the output limit twice.
export const worstHi = (bytes, maxOut) => (bytes * 1.4 + 2 * maxOut * 4.25) / 1e6;

export function usageOf(v) {
  const u = v?.usage;
  if (!u || typeof u.input_tokens !== 'number' || typeof u.output_tokens !== 'number') return null;
  return {
    input: u.input_tokens,
    cached: u.input_tokens_details?.cached_tokens ?? 0,
    output: u.output_tokens,
    reasoning: u.output_tokens_details?.reasoning_tokens ?? 0,
  };
}

export function outputText(v) {
  for (const item of v?.output ?? []) for (const part of item?.content ?? []) if (part?.type === 'output_text') return part.text;
  return null;
}

// The binary's own rule for the first question: a whole object with exactly `verdict` and `reason`, the verdict one of two words.
export function readFirst(text) {
  let a;
  try {
    a = JSON.parse(text);
  } catch {
    return null;
  }
  if (!a || typeof a !== 'object' || Object.keys(a).sort().join() !== 'reason,verdict' || typeof a.reason !== 'string') return null;
  return a.verdict === 'claim' || a.verdict === 'not_a_claim' ? a : null;
}

// ---- the spend ledger ----

const HEADER = `# W2-first-pass spend ledger (cap $1.50 hi for the whole stage; every model call counts, failed ones too)

Model: muse-spark-1.3 (Meta API, standard-priced id), called by the real torpor-muse binary through a loopback stand-in (eval/first-pass/run.mjs) that adds the key and writes one row per forwarded response; reasoning effort low. A canned answer costs nothing and has no row.
Prices per 1M tokens: cached input $0.15, input $1.25, output $4.25.
lo = (input - cached) x 1.25 + cached x 0.15 + output x 4.25 (what the binary's own ledger records); hi = input x 1.25 + cached x 0.15 + (output + reasoning) x 4.25 (an upper bound: it counts the cached tokens twice and reasoning again where it is already inside output). lo is the best estimate; the cap is checked against hi. A call that gave no usage (an HTTP error, a timeout) is charged its worst case in hi and nothing in lo.

| # | run | call | time UTC | model | input | cached | output | reasoning | $ lo | $ hi | cumulative $ hi |
|---|-----|------|----------|-------|-------|--------|--------|-----------|------|------|-----------------|
`;
const ROW = /^\| (\d+) \|.*\| ([\d.]+) \| ([\d.]+) \| ([\d.]+) \|$/;

function ledgerRows(file) {
  return fs.readFileSync(file, 'utf8').split('\n').map((l) => ROW.exec(l)).filter(Boolean).map((m) => ({ n: +m[1], lo: +m[2], hi: +m[3], cum: +m[4] }));
}

function ledgerOpen(file) {
  if (!fs.existsSync(file)) fs.writeFileSync(file, HEADER);
  fs.writeFileSync(file, fs.readFileSync(file, 'utf8').split('\n').filter((l) => !l.startsWith('Total:')).join('\n').trimEnd() + '\n');
  const rows = ledgerRows(file);
  return { n: rows.length ? rows.at(-1).n : 0, hi: rows.length ? rows.at(-1).cum : 0 };
}

function ledgerTotal(file) {
  const rows = ledgerRows(file);
  const sum = (k) => rows.reduce((a, r) => a + r[k], 0);
  fs.appendFileSync(file, `\nTotal: ${rows.length} calls, $ lo ${sum('lo').toFixed(4)}, $ hi ${sum('hi').toFixed(4)} of the $${CAP_HI.toFixed(2)} cap.\n`);
}

// ---- the stand-in for the API ----

function startProxy(leg, ledger, runName, upstream, key) {
  const state = { ...ledgerOpen(ledger), cur: null, stopped: false, model: null };
  const answer = (res, status, obj) => {
    res.writeHead(status, { 'content-type': 'application/json' });
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
      return answer(res, 400, { error: 'not json' });
    }
    const kind = body?.text?.format?.name === FIRST ? 'first' : body?.text?.format?.name === CHECK ? 'check' : 'other';
    if (kind === 'other' || !state.cur) return answer(res, 404, { error: 'unexpected request' });
    state.model ??= body.model;
    const call = { kind, real: (leg === 'first') === (kind === 'first') };
    const cur = state.cur;
    cur.calls.push(call);
    if (!call.real) {
      const text = JSON.stringify(CANNED[kind === 'first' ? FIRST : CHECK]);
      call.text = text;
      return answer(res, 200, { status: 'completed', error: null, usage: { input_tokens: 0, output_tokens: 0 }, output: [{ type: 'message', content: [{ type: 'output_text', text }] }] });
    }
    const worst = worstHi(raw.length, body.max_output_tokens ?? 4000);
    if (state.hi + worst > CAP_HI) {
      state.stopped = true;
      call.status = 503;
      call.stopped = true;
      return answer(res, 503, { error: 'eval spend cap' });
    }
    const t0 = performance.now();
    let status = 0;
    let text = '';
    try {
      const r = await fetch(`${upstream}/v1/responses`, { method: 'POST', headers: { 'content-type': 'application/json', authorization: `Bearer ${key}` }, body: raw, signal: AbortSignal.timeout(150_000) });
      status = r.status;
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
    const c = u ? cost(u) : { lo: 0, hi: worst };
    Object.assign(call, c);
    state.n += 1;
    state.hi += c.hi;
    const u0 = u ?? { input: '?', cached: '?', output: '?', reasoning: '?' };
    const label = `${kind === 'first' ? 'first pass' : 'today'} ${cur.id}${status === 200 ? '' : ` (HTTP ${status})`}`;
    fs.appendFileSync(ledger, `| ${state.n} | ${runName} | ${label} | ${new Date().toISOString().slice(11, 19)}Z | ${body.model} | ${u0.input} | ${u0.cached} | ${u0.output} | ${u0.reasoning} | ${c.lo.toFixed(4)} | ${c.hi.toFixed(4)} | ${state.hi.toFixed(4)} |\n`);
    res.writeHead(status || 502, { 'content-type': 'application/json' });
    res.end(text);
  });
  return new Promise((ok) => server.listen(0, '127.0.0.1', () => ok({ server, state, port: server.address().port })));
}

// ---- one prompt in a fresh copy of the demo app ----

function workspace(root) {
  for (const d of ['state', 'home', 'th', 'eng']) fs.mkdirSync(path.join(root, d));
  const ws = path.join(root, 'ws');
  fs.cpSync(path.join(REPO, 'demo/app'), ws, { recursive: true });
  fs.writeFileSync(path.join(ws, '.synthetic'), 'made-up demo content\n');
  fs.writeFileSync(path.join(root, 'eng/torpor'), FAKE_ENGINE, { mode: 0o755 });
  fs.writeFileSync(path.join(root, 'eng/add.out'), `Checking...\n${PROVEN}\n`);
  const env = { PATH: process.env.PATH, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_SYSTEM: '/dev/null', GIT_AUTHOR_NAME: 't', GIT_AUTHOR_EMAIL: 't', GIT_COMMITTER_NAME: 't', GIT_COMMITTER_EMAIL: 't' };
  for (const a of [['init', '-q'], ['add', '-A'], ['commit', '-qm', 'init']]) execFileSync('git', a, { cwd: ws, env });
  return ws;
}

function suggest(bin, root, ws, port, words) {
  const env = {
    PATH: process.env.PATH,
    HOME: path.join(root, 'home'),
    TORPOR_HOME: path.join(root, 'th'),
    TORPOR_MUSE_STATE: path.join(root, 'state'),
    TORPOR_BIN: path.join(root, 'eng/torpor'),
    META_API_KEY: FAKE_KEY,
    TORPOR_MUSE_API_BASE: `http://127.0.0.1:${port}`,
  };
  return new Promise((ok) => execFile(bin, ['suggest', words], { cwd: ws, env, timeout: 300_000 }, (err, stdout) => ok({ exit: err ? (err.code ?? -1) : 0, out: String(stdout).split('\n')[0].slice(0, 160) })));
}

async function runLeg(leg, promptsFile, rowsFile) {
  assert(leg === 'first' || leg === 'baseline', 'leg is first or baseline');
  const need = (n) => process.env[n] || assert.fail(`${n} is not set`);
  const bin = need('EVAL_BIN');
  const ledger = need('EVAL_LEDGER');
  const tmp = need('EVAL_TMP');
  const upstream = process.env.EVAL_UPSTREAM || API;
  const loop = /^http:\/\/127\.0\.0\.1:\d+$/.test(upstream);
  assert(loop || upstream === API, 'EVAL_UPSTREAM must be a loopback address');
  const key = loop ? 'rehearsal' : need('META_API_KEY');
  delete process.env.META_API_KEY; // the children get an explicit environment; this keeps it out of anything else
  const prompts = parseSet(fs.readFileSync(promptsFile, 'utf8')).filter((p) => leg === 'first' || p.base);
  assert(prompts.length, 'no prompts to run');
  const sha = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: REPO, encoding: 'utf8' }).trim();
  const dirty = execFileSync('git', ['status', '--porcelain', '--', 'src', 'Cargo.toml', 'Cargo.lock'], { cwd: REPO, encoding: 'utf8' }).trim() !== '';
  fs.mkdirSync(tmp, { recursive: true });
  const runName = path.basename(rowsFile, '.jsonl');
  const { server, state, port } = await startProxy(leg, ledger, runName, upstream, key);
  const meta = { leg, sha, dirty, started: new Date().toISOString(), upstream: loop ? 'mock' : 'api', prompts: prompts.length };
  fs.writeFileSync(rowsFile, JSON.stringify({ meta }) + '\n');
  const rows = [];
  try {
    for (const p of prompts) {
      const root = fs.mkdtempSync(path.join(tmp, 'w-'));
      const ws = workspace(root);
      state.cur = { id: p.id, calls: [] };
      const r = await suggest(bin, root, ws, port, p.text);
      const row = { ...p, ...r, calls: state.cur.calls };
      rows.push(row);
      fs.appendFileSync(rowsFile, JSON.stringify(row) + '\n');
      const real = row.calls.filter((c) => c.real);
      process.stderr.write(`${p.id} ${leg}: ${real.map((c) => `${c.status} ${(c.kind === 'first' ? firstOf(row) : todayOf(row)).v} $${(c.lo ?? 0).toFixed(4)}`).join(' | ') || 'no real call'} (cum hi ${state.hi.toFixed(4)})\n`);
      if (state.stopped) break;
    }
  } finally {
    server.close();
    meta.model = state.model;
    meta.finished = new Date().toISOString();
    const lines = fs.readFileSync(rowsFile, 'utf8').split('\n');
    lines[0] = JSON.stringify({ meta });
    fs.writeFileSync(rowsFile, lines.join('\n'));
    ledgerTotal(ledger);
  }
  if (state.stopped) {
    process.stderr.write(`STOPPED for the cap: hi so far ${state.hi.toFixed(4)} of ${CAP_HI}; ${rows.length} of ${prompts.length} prompts done\n`);
    process.exit(3);
  }
}

// ---- the report ----

const mean = (a) => (a.length ? a.reduce((x, y) => x + y, 0) / a.length : NaN);
const median = (a) => (a.length ? [...a].sort((x, y) => x - y)[Math.floor(a.length / 2)] : NaN);
const usd = (x) => (Number.isFinite(x) ? x.toFixed(4) : 'n/a');
const load = (f) => {
  const lines = fs.readFileSync(f, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
  return { meta: lines[0].meta, rows: lines.slice(1) };
};

// The first question's reading of a row: verdict `claim` / `not_a_claim`, or `fallback` (no usable answer) with why.
export function firstOf(r) {
  const c = r.calls.find((x) => x.kind === 'first' && x.real);
  if (!c) return { v: 'none', reason: 'no first question was asked', c: { lo: 0, secs: null, usage: null } };
  const a = c.text ? readFirst(c.text) : null;
  if (c.status === 200 && a) return { v: a.verdict, reason: a.reason, c };
  return { v: 'fallback', reason: c.status === 200 ? `unusable answer: ${String(c.text).slice(0, 80)}` : `HTTP ${c.status}`, c };
}

export function todayOf(r) {
  const c = r.calls.find((x) => x.kind === 'check' && x.real);
  if (!c) return null;
  let v = 'unusable';
  try {
    v = JSON.parse(c.text).verdict;
  } catch {}
  return { v, c };
}

function report(firstFile, baseFile) {
  const F = load(firstFile);
  const B = baseFile ? load(baseFile) : null;
  const fr = F.rows.map((r) => ({ r, f: firstOf(r) }));
  const claims = fr.filter((x) => x.r.label === 'claim');
  const others = fr.filter((x) => x.r.label === 'not');
  const missed = claims.filter((x) => x.f.v === 'not_a_claim');
  const falseClaims = others.filter((x) => x.f.v !== 'not_a_claim');
  const lo = (x) => x.f.c.lo;
  const L = [];
  L.push(`Model ${F.meta.model}; code ${F.meta.sha}${F.meta.dirty ? ' (src had uncommitted changes)' : ''}; first-question run ${F.meta.started} to ${F.meta.finished}.`);
  L.push('', '| | claims | others |', '|---|---|---|');
  L.push(`| prompts | ${claims.length} | ${others.length} |`);
  L.push(`| first question said "claim" | ${claims.filter((x) => x.f.v === 'claim').length} | ${others.filter((x) => x.f.v === 'claim').length} |`);
  L.push(`| first question said "not_a_claim" | ${missed.length} | ${others.filter((x) => x.f.v === 'not_a_claim').length} |`);
  L.push(`| no usable answer (counts as claim) | ${claims.filter((x) => x.f.v === 'fallback').length} | ${others.filter((x) => x.f.v === 'fallback').length} |`);
  L.push(`| mean USD of the first question (lo) | ${usd(mean(claims.map(lo)))} | ${usd(mean(others.map(lo)))} |`);
  L.push(`| median seconds of the first question | ${median(claims.map((x) => x.f.c.secs))} | ${median(others.map((x) => x.f.c.secs))} |`);
  L.push('', `Recall on claims: ${claims.length - missed.length}/${claims.length} (bar: at least 19/20). Others called claims: ${falseClaims.length}/${others.length} (bar: at most 3/20).`);
  const bad = [
    ...missed.map((x) => ({ ...x, what: 'claim missed (called not_a_claim)' })),
    ...claims.filter((x) => x.f.v === 'fallback' || x.f.v === 'none').map((x) => ({ ...x, what: 'claim: no usable answer (went on to today\'s call, nothing lost)' })),
    ...falseClaims.map((x) => ({ ...x, what: x.f.v === 'claim' ? 'other called claim' : `other: ${x.f.v}` })),
  ];
  L.push('', `Misses (${bad.length}):`);
  for (const x of bad) L.push(`- ${x.r.id} ${x.what}: ${JSON.stringify(x.r.text)} -> ${x.f.v}${x.f.reason ? ` (${JSON.stringify(x.f.reason)})` : ''}`);
  if (!bad.length) L.push('- none');
  if (B) {
    const bm = new Map(B.rows.map((r) => [r.id, todayOf(r)]));
    const tl = (r) => bm.get(r.id)?.c.lo;
    const baseOthers = others.filter((x) => bm.get(x.r.id));
    const baseClaims = claims.filter((x) => bm.get(x.r.id));
    const bo = mean(baseOthers.map((x) => tl(x.r)));
    const bc = mean(baseClaims.map((x) => tl(x.r)));
    const newOther = (x) => lo(x) + (x.f.v === 'not_a_claim' ? 0 : (tl(x.r) ?? bo));
    const newClaim = (x) => lo(x) + (tl(x.r) ?? bc);
    const mo = mean(others.map(newOther));
    L.push('', `Baseline: today's single call on ${baseClaims.length} claims and ${baseOthers.length} others (run ${B.meta.started}, code ${B.meta.sha}).`);
    L.push('', '| USD per prompt (lo) | today (baseline) | with the first question |', '|---|---|---|');
    L.push(`| others, mean | ${usd(bo)} (${baseOthers.length} measured) | ${usd(mo)} (all ${others.length}; ${others.filter((x) => x.f.v !== 'not_a_claim').length} went on to today's call) |`);
    L.push(`| claims, mean | ${usd(bc)} (${baseClaims.length} measured) | ${usd(mean(claims.map(newClaim)))} (all ${claims.length}) |`);
    L.push(`| others measured both ways, mean | ${usd(mean(baseOthers.map((x) => tl(x.r))))} | ${usd(mean(baseOthers.map(newOther)))} |`);
    L.push(`| claims measured both ways, mean | ${usd(mean(baseClaims.map((x) => tl(x.r))))} | ${usd(mean(baseClaims.map(newClaim)))} |`);
    L.push('', `Cost bar: mean USD per other prompt with the first question ${usd(mo)} against half of today's ${usd(bo / 2)}: ${mo <= bo / 2 ? 'met' : 'NOT met'} (ratio ${(mo / bo).toFixed(2)}).`);
    L.push(`Median seconds of today's call: ${median([...bm.values()].filter(Boolean).map((t) => t.c.secs))}.`);
    L.push('', "Today's call on the baseline prompts:");
    for (const r of B.rows) L.push(`- ${r.id} ${todayOf(r)?.v ?? 'no call'} $${usd(todayOf(r)?.c.lo)}`);
  }
  L.push('', '| id | first question | USD lo | input/cached/output/reasoning | seconds | prompt |', '|---|---|---|---|---|---|');
  for (const x of fr) {
    const u = x.f.c.usage;
    L.push(`| ${x.r.id} | ${x.f.v} | ${usd(lo(x))} | ${u ? `${u.input}/${u.cached}/${u.output}/${u.reasoning}` : '?'} | ${x.f.c.secs} | ${JSON.stringify(x.r.text.replace(/\n/g, ' ')).slice(0, 60)} |`);
  }
  return L.join('\n') + '\n';
}

// ---- self-check ----

function selftest() {
  const set = parseSet(fs.readFileSync(path.join(HERE, 'set.md'), 'utf8'));
  const claims = set.filter((p) => p.label === 'claim');
  const others = set.filter((p) => p.label === 'not');
  assert.equal(set.length, 40);
  assert.equal(claims.length, 20);
  assert.equal(others.length, 20);
  assert.equal(claims.filter((p) => p.base).length, 10);
  assert.equal(others.filter((p) => p.base).length, 10);
  assert.equal(new Set(set.map((p) => p.id)).size, 40);
  assert.equal(claims.filter((p) => /[֐-׿]/.test(p.text)).length, 3);
  assert(others.find((p) => p.id === 'N07').text.includes('\n'));
  assert(set.every((p) => p.text.length >= 3 && !p.text.includes('/ho' + 'me/')));
  const dry = parseSet(fs.readFileSync(path.join(HERE, 'dry.md'), 'utf8'));
  assert.equal(dry.length, 5);
  assert(dry.every((d) => d.id.startsWith('x') && !set.some((p) => p.text === d.text)));
  // the recorded `ok` fixture's usage: 135 in, 0 cached, 787 out (654 of them reasoning)
  const c = cost({ input: 135, cached: 0, output: 787, reasoning: 654 });
  assert(Math.abs(c.lo - 0.0035135) < 1e-9);
  assert(Math.abs(c.hi - (135 * 1.25 + (787 + 654) * 4.25) / 1e6) < 1e-9);
  assert.deepEqual(cost({ input: 1000, cached: 400, output: 0, reasoning: 0 }), { lo: (600 * 1.25 + 400 * 0.15) / 1e6, hi: (1000 * 1.25 + 400 * 0.15) / 1e6 });
  assert.equal(readFirst('{"verdict":"claim","reason":""}').verdict, 'claim');
  for (const bad of ['{"verdict":"check","reason":""}', '{"verdict":"claim"}', '{"verdict":"claim","reason":"","x":1}', 'not json', '[]', 'null']) assert.equal(readFirst(bad), null, bad);
  assert.equal(outputText({ output: [{ type: 'reasoning' }, { type: 'message', content: [{ type: 'output_text', text: 'hi' }] }] }), 'hi');
  assert.equal(usageOf({}), null);
  assert(worstHi(14000, 4000) > 0.05 && worstHi(14000, 4000) < 0.06);
  process.stdout.write('selftest: OK\n');
}

const [cmd, ...args] = process.argv.slice(2);
if (import.meta.url === `file://${process.argv[1]}`) {
  if (cmd === 'selftest') selftest();
  else if (cmd === 'run' && args.length === 3) await runLeg(...args);
  else if (cmd === 'report' && args.length >= 1) process.stdout.write(report(args[0], args[1]));
  else {
    process.stderr.write('usage: run.mjs selftest | run <first|baseline> <prompts.md> <rows.jsonl> | report <first.jsonl> [<baseline.jsonl>]\n');
    process.exit(2);
  }
}

#!/usr/bin/env node
// node eval/run/selftest.mjs : offline, no Muse, no key, no network, under 60 s.
// A fake `muse` (a small node script) stands in for Muse Code so the whole runner can be played end to end inside the real sandbox.
// Parts that need bwrap are skipped (and counted) when bwrap cannot run here.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { armCList, armCSettings, armCShape, AGENTS_MD, ENGINE_EVENTS } from './arms.mjs';
import { rmEp, runEpisode } from './episode.mjs';
import * as L from './lib.mjs';
import { sandbox } from './sandbox.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const EVAL = path.resolve(HERE, '..');
const results = { pass: 0, fail: 0, skip: 0 };
async function test(name, fn) {
  try { const r = await fn(); if (r === 'skip') { results.skip++; console.log(`skip  ${name}`); } else { results.pass++; console.log(`ok    ${name}`); } } catch (e) {
    results.fail++; console.log(`FAIL  ${name}\n      ${String(e.stack || e).split('\n').slice(0, 4).join('\n      ')}`);
  }
}

const TMP = fs.mkdtempSync('/tmp/evalrun-selftest-');
const mk = (...p) => { const d = path.join(TMP, ...p); fs.mkdirSync(d, { recursive: true }); return d; };
const bwrapOk = spawnSync('bwrap', ['--ro-bind', '/', '/', 'true']).status === 0;

// ---------------------------------------------------------------- the fake muse
const FAKE = `#!/usr/bin/env node
const fs = require('fs'), path = require('path'), crypto = require('crypto');
const argv = process.argv.slice(2);
if (argv[0] === '--version') { console.log('Muse Code 0.0.0-fake'); process.exit(0); }
const val = (f) => { const i = argv.indexOf(f); return i >= 0 ? argv[i + 1] : null; };
const session = val('--session-id'), prompt = fs.readFileSync(val('--prompt-file'), 'utf8').trim();
const ws = process.cwd(), ep = path.dirname(ws);
const walk = (d, h) => { for (const e of fs.readdirSync(d, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) { if (e.name === '.git') continue; const p = path.join(d, e.name); if (e.isDirectory()) walk(p, h); else h.update(p).update(fs.readFileSync(p)); } return h; };
fs.appendFileSync(path.join(ep, 'trace.txt'), walk(ws, crypto.createHash('sha256')).digest('hex').slice(0, 12) + ' ' + prompt.slice(0, 20).replace(/\\s+/g, '_') + ' ' + (fs.existsSync(path.join(ws, 'AGENTS.md')) ? 'agents' : '-') + '\\n');
const cost = JSON.parse(fs.readFileSync(path.join(__dirname, 'cost.json'), 'utf8'));
const dir = path.join(process.env.HOME, '.local/share/muse/sessions/2026/10/27', session);
fs.mkdirSync(dir, { recursive: true });
const ev = (e) => fs.appendFileSync(path.join(dir, 'session.jsonl'), JSON.stringify({ payload: { event: e } }) + '\\n');
if (prompt === '@@plant-oracle') fs.copyFileSync(path.join(__dirname, 'oracle-copy.mjs'), path.join(ws, 'helper.mjs'));
if (prompt === '@@plant-name') fs.writeFileSync(path.join(ws, 'reference.patch'), 'x');
if (prompt === '@@hang') { setInterval(() => {}, 1000); return; }
if (prompt === '@@fail') { console.error('muse: something broke'); process.exit(3); }
ev({ kind: 'model_response_created' });
ev({ kind: 'model_completed', model: cost.model, usage: { input_tokens: cost.input, cached_tokens: 0, output_tokens: cost.output, reasoning_tokens: 0 } });
if (prompt === '@@steps') { console.error('muse: model did not reach a terminal state within 40 step(s)'); process.exit(1); }
console.log('done');
`;
let fakeN = 0;
function fakeMuse(costObj = { model: 'muse-spark-1.3-contributor', input: 1000, output: 500 }) {
  const d = mk(`fake-muse-${++fakeN}`);
  fs.writeFileSync(path.join(d, 'muse'), FAKE, { mode: 0o755 });
  fs.writeFileSync(path.join(d, 'cost.json'), JSON.stringify(costObj));
  fs.copyFileSync(path.join(EVAL, 'oracles', 'notes.mjs'), path.join(d, 'oracle-copy.mjs'));
  return d;
}
const tasks = new Map(fs.readdirSync(path.join(EVAL, 'tasks')).filter((d) => fs.existsSync(path.join(EVAL, 'tasks', d, 'task.json'))).map((id) => [id, JSON.parse(fs.readFileSync(path.join(EVAL, 'tasks', id, 'task.json'), 'utf8'))]));
const heldIdx = L.heldOutIndex(EVAL);
let cxN = 0;
function mkCx(over = {}) {
  const n = ++cxN;
  return {
    runId: 'selftest', seed: 1, keyed: false, key: '', model: 'muse-spark-1.3-contributor', effort: 'medium', maxSteps: 40, turnTimeoutMs: 60000, epCapMs: 300000, evalDir: EVAL, repoDir: path.resolve(EVAL, '..'),
    museDir: fakeMuse(), museVersion: 'fake', engineVersion: 'n/a', adapterCommit: null, tmpBase: mk(`tmp${n}`), outDir: mk(`out${n}`), tasks, heldIdx, keepEp: true,
    spend: new L.Spend(Infinity, null), backoff: new L.Backoff(), writerWaitMs: 1000, ...over,
  };
}
const epDirs = (cx) => fs.readdirSync(cx.tmpBase).filter((n) => /^ep-[0-9a-f]{10}$/.test(n)).map((n) => path.join(cx.tmpBase, n));
const trace = (cx) => fs.readFileSync(path.join(epDirs(cx)[0], 'trace.txt'), 'utf8').trim().split('\n').map((l) => l.split(' '));
const clean = (cx) => { for (const d of fs.readdirSync(cx.tmpBase)) rmEp(path.join(cx.tmpBase, d), cx.tmpBase); };
const job = (task, arm, rep = 1) => ({ task, arm, rep, order_index: 0 });
const withPrompts = (id, map) => { const t = JSON.parse(JSON.stringify(tasks.get(id))); for (const [n, p] of Object.entries(map)) t.turns[n - 1].prompt = p; return t; };

// ---------------------------------------------------------------- pure checks
await test('plan: same seed, same order; other seed, other order; every block has every arm once', () => {
  const ids = [...tasks.keys()];
  const a = L.planJobs({ tasks: ids, arms: L.ARMS, reps: 2, seed: 11 });
  assert.deepEqual(a, L.planJobs({ tasks: ids, arms: L.ARMS, reps: 2, seed: 11 }));
  assert.notDeepEqual(a.map((j) => j.arm), L.planJobs({ tasks: ids, arms: L.ARMS, reps: 2, seed: 12 }).map((j) => j.arm));
  assert.equal(a.length, ids.length * 2 * 4);
  a.forEach((j, i) => assert.equal(j.order_index, i));
  for (let i = 0; i < a.length; i += 4) assert.deepEqual(a.slice(i, i + 4).map((j) => j.arm).sort(), L.ARMS, 'block holds each arm once');
  assert.ok(new Set(a.filter((_, i) => i % 4 === 0).map((j) => j.arm)).size > 1, 'the first arm of a block varies');
});

await test('prices: contributor and standard ids, lo below hi', () => {
  const u = { input_tokens: 1000000, cached_tokens: 400000, output_tokens: 100000, reasoning_tokens: 60000 };
  const c = L.cost('muse-spark-1.3-contributor', u), s = L.cost('muse-spark-1.3', u);
  assert.ok(Math.abs(c.lo - (0.6 * 0.1 + 0.4 * 0.002 + 0.1 * 0.2)) < 1e-9);
  assert.ok(Math.abs(c.hi - (1 * 0.1 + 0.4 * 0.002 + 0.16 * 0.2)) < 1e-9);
  assert.ok(Math.abs(s.lo - (0.6 * 1.25 + 0.4 * 0.15 + 0.1 * 4.25)) < 1e-9);
  assert.ok(c.lo < c.hi && s.lo < s.hi && c.hi < s.lo);
});

await test('spend: refuses a turn that could pass the cap, on synthetic ledgers, and counts running turns and past attempts', () => {
  const f = path.join(mk('spend'), 'spend.jsonl');
  fs.writeFileSync(f, [{ usd_hi: 0.4, writer_usd_hi: 0.1 }, { usd_hi: 0.2, writer_usd_hi: 0 }].map((x) => JSON.stringify(x)).join('\n') + '\n'); // spent 0.7, largest 0.5
  const s = new L.Spend(1.3, f);
  assert.ok(Math.abs(s.total - 0.7) < 1e-9);
  s.startTurn(); // 0.7 + 0.5 = 1.2 <= 1.3
  assert.throws(() => s.startTurn(), L.SpendRefused); // a second running turn: 0.7 + 2 x 0.5 = 1.7
  s.endTurn({ usd_hi: 0.3, writer_usd_hi: 0.05 }); // total 1.05, largest stays 0.5
  assert.throws(() => s.startTurn(), L.SpendRefused); // 1.05 + 0.5 > 1.3
  assert.equal(new L.Spend(1.3, f).total.toFixed(2), '1.05', 'a resumed run reloads every earlier line');
  assert.doesNotThrow(() => new L.Spend(10, null).startTurn());
});

await test('rows: a good row validates; missing or wrong fields do not', () => {
  const ok = { schema: 1, run_id: 'r', task: 't', app: 'notes', kind: 'control', torpor_should_help: 'no', arm: 'A', rep: 1, attempt: 1, seed: 1, order_index: 0, model: 'm', muse_version: 'v', engine_version: 'e', adapter_commit: null,
    started_utc: 'x', ended_utc: 'y', minutes: 1, turns_run: 6, steps_total: 6, timeout: false, usd_lo: 0, usd_hi: 0, calls: 6, writer_usd_hi: 0, writer_calls: 0, rate_limited: 0, hook_failures: 0, gate_blocks: 0,
    oracle: { exit: 1, line: '{}' }, final_pass: false, later_kept: false, break_found: null, valid: true, invalid_reason: null, manifest: {} };
  assert.deepEqual(L.validateRow(ok), []);
  assert.ok(L.validateRow({ ...ok, usd_hi: undefined }).length);
  assert.ok(L.validateRow({ ...ok, arm: 'E' }).length);
  assert.ok(L.validateRow({ ...ok, steps_total: '6' }).length);
  assert.ok(L.validateRow({ ...ok, oracle: null }).length, 'a valid row needs its oracle result');
  assert.ok(L.validateRow({ ...ok, valid: false }).length, 'an invalid row needs a reason');
  assert.deepEqual(L.validateRow({ ...ok, valid: false, invalid_reason: 'why', oracle: null, final_pass: null, later_kept: null }), []);
});

await test('resume bookkeeping: a valid row, a row marked not re-runnable, or 3 invalid rows (a cap stop counts) finish an episode', () => {
  const r = (task, attempt, valid, reason = null, extra = {}) => ({ task, arm: 'A', rep: 1, attempt, valid, invalid_reason: reason, ...extra });
  const st = L.episodeStates([r('a', 1, true), r('b', 1, false, 'muse_exit_1'), r('c', 1, false, 'x'), r('c', 2, false, 'x'), r('c', 3, false, 'x'), r('d', 1, false, 'inject_failed', { retryable: false }), r('e', 1, false, 'spend_cap: x'), r('e', 2, false, 'spend_cap: x'), r('e', 3, false, 'spend_cap: x')]);
  assert.equal(st.get('a__A__r1').done, true);
  assert.equal(st.get('b__A__r1').done, false); assert.equal(st.get('b__A__r1').last, 1);
  assert.equal(st.get('c__A__r1').done, true);
  assert.equal(st.get('d__A__r1').done, true);
  assert.equal(st.get('e__A__r1').done, true); assert.equal(st.get('e__A__r1').last, 3);
  assert.equal(L.episodeStates([r('f', 1, false, 'spend_cap: x'), r('f', 2, false, 'spend_cap: x')]).get('f__A__r1').done, false);
});

await test('held out: a planted oracle copy, a renamed task file and a patch are found; a clean workspace is not', () => {
  const ws = mk('held', 'ws');
  const clean1 = spawnSync(process.execPath, [path.join(EVAL, 'make-workspace.mjs'), 'todo-ctrl', path.join(mk('held'), 'w2')], { encoding: 'utf8' });
  assert.equal(clean1.status, 0, clean1.stderr);
  assert.deepEqual(L.heldOutLeaks(path.join(TMP, 'held', 'w2'), heldIdx), [], 'a real workspace is clean');
  fs.copyFileSync(path.join(EVAL, 'oracles', 'shop.mjs'), path.join(ws, 'helper.mjs'));
  fs.copyFileSync(path.join(EVAL, 'tasks', 'notes-dup', 'task.json'), path.join(ws, 'config.json'));
  fs.copyFileSync(path.join(EVAL, 'lib', 'kit.mjs'), path.join(ws, 'kit-copy.js'));
  fs.writeFileSync(path.join(ws, 'inject.patch'), 'x');
  assert.deepEqual(L.heldOutLeaks(path.join(TMP, 'held'), heldIdx).map((p) => path.basename(p)).sort(), ['config.json', 'helper.mjs', 'inject.patch', 'kit-copy.js']);
});

await test('turns are classified: a step cap is not infra, a timeout is a fail, 429 backs off, auth is fatal', () => {
  assert.equal(L.classifyTurn({ code: 0 }).kind, 'ok');
  assert.equal(L.classifyTurn({ code: 1, stderr: 'muse: model did not reach a terminal state within 40 step(s)' }).kind, 'step_cap');
  assert.equal(L.classifyTurn({ code: null, signal: 'SIGTERM', timedOut: true }).kind, 'timeout');
  const rl = L.classifyTurn({ code: 1, stderr: 'provider error: 429 Too Many Requests, retry-after: 17' });
  assert.deepEqual([rl.kind, rl.retryAfterS], ['rate_limited', 17]);
  assert.equal(L.classifyTurn({ code: 1, stderr: 'http 401 unauthorized' }).kind, 'fatal');
  assert.equal(L.classifyTurn({ code: 1, stderr: 'failed to fetch model catalog: authentication failed: your API key from META_API_KEY was rejected — update or unset it' }).kind, 'fatal', 'the message Muse 1.4.13 printed for a bogus key');
  assert.equal(L.classifyTurn({ code: 1, stderr: 'agent loop failed: model failed: model `x` does not exist or you lack access [request_id=1]' }).kind, 'fatal', 'an unknown model id');
  assert.equal(L.classifyTurn({ code: 134, stderr: 'segfault' }).kind, 'infra');
});

await test('session logs: usage, hooks, gate blocks, writer ledger, frame children', () => {
  const home = mk('logs', 'home');
  const d = path.join(home, '.local/share/muse/sessions/2026/10/27/s1');
  fs.mkdirSync(d, { recursive: true });
  const rec = (event) => JSON.stringify({ payload: { event } });
  const lines = [
    rec({ kind: 'model_response_created' }), rec({ kind: 'model_completed', model: 'muse-spark-1.3-contributor', usage: { input_tokens: 1000, cached_tokens: 200, output_tokens: 100, reasoning_tokens: 50 } }),
    rec({ kind: 'hook_run_terminal', event: 'Stop', status: 'blocked', exit_code: 0 }), rec({ kind: 'hook_run_terminal', event: 'Stop', status: 'completed', exit_code: 0 }),
    rec({ kind: 'hook_run_terminal', event: 'SessionStart', status: 'timed_out', exit_code: null }), rec({ kind: 'failed', reason: 'process exited with status 1' }),
    rec({ kind: 'terminal', terminal: 'failed', reason: 'provider returned 429 Too Many Requests' }),
    JSON.stringify({ children: [{ record_json: rec({ kind: 'model_completed', model: 'muse-spark-1.3-contributor', usage: { input_tokens: 10 } }) }] }), 'not json',
  ];
  fs.writeFileSync(path.join(d, 'session.jsonl'), lines.join('\n') + '\n');
  const sd = path.join(home, '.local/state/torpor-muse');
  fs.mkdirSync(sd, { recursive: true });
  fs.writeFileSync(path.join(sd, 'ledger.jsonl'), `${JSON.stringify({ usd: 0.01 })}\n${JSON.stringify({ usd: 0.02 })}\n{torn`);
  const t = L.tally(home, 'x');
  assert.deepEqual([t.steps, t.calls, t.hook_failures, t.gate_blocks, t.rate_limited, t.hooks.Stop.ok], [2, 1, 1, 1, 1, 1]);
  assert.ok(Math.abs(t.hi - (L.cost('muse-spark-1.3-contributor', { input_tokens: 1000, cached_tokens: 200, output_tokens: 100, reasoning_tokens: 50 }).hi + 10 * 0.1 / 1e6)) < 1e-12);
  const w = L.writerLedger(home);
  assert.deepEqual([w.rows, Number(w.usd.toFixed(2))], [2, 0.03]);
});

await test('arms: C is the engine\'s ten documented hook commands and nothing else; B never names the tool', () => {
  const s = armCSettings('/x/torpor');
  assert.deepEqual(Object.keys(s), ['schema_version', 'hooks']);
  assert.equal(Object.keys(s.hooks).length, 10);
  for (const [event, matcher, timeout] of ENGINE_EVENTS) {
    const g = s.hooks[event];
    assert.equal(g.length, 1); assert.equal(g[0].matcher, matcher ?? undefined); assert.ok(Number.isInteger(g[0].hooks[0].timeout) && g[0].hooks[0].timeout === timeout);
    assert.equal(g[0].hooks[0].command, `/x/torpor hook ${event}`);
    assert.deepEqual(Object.keys(g[0].hooks[0]).sort(), ['command', 'timeout', 'type']);
  }
  assert.deepEqual(Object.entries(s.hooks).filter(([, g]) => g[0].matcher).map(([e]) => e).sort(), ['PostToolUse', 'PostToolUseFailure', 'PreToolUse']);
  assert.ok(armCShape().includes('ENGINE_BIN hook Stop'));
  assert.ok(!/torpor|engine|save point|undo/i.test(AGENTS_MD) && /NOTES\.md/.test(AGENTS_MD));
  // RUN.md shows the exact notes text and the exact list of C's events; the lead checks it against PREREG, so it must not drift from the code
  const md = fs.readFileSync(path.join(HERE, 'RUN.md'), 'utf8');
  for (const line of AGENTS_MD.split('\n').filter(Boolean)) assert.ok(md.includes(`      ${line}`), `RUN.md lacks the AGENTS.md line: ${line}`);
  for (const line of armCList().split('\n')) assert.ok(md.includes(`      ${line}`), `RUN.md lacks the arm C line: ${line}`);
  assert.ok(!md.includes(['', 'home', ''].join('/')), 'RUN.md names a home folder'); // (spelled out so this file does not contain the string itself)
});

// ---------------------------------------------------------------- sandboxed checks (fake muse)
await test('sandbox: writes stay inside the run folder, the key needs the network mode, stray env is refused', async () => {
  if (!bwrapOk) return 'skip';
  const rd = mk('sb');
  const r = await sandbox({ runDir: rd, net: 'none', cmd: ['sh', '-c', 'echo x > /tmp/leak-probe; echo y > "$HOME/kept"'], timeoutMs: 20000 });
  assert.equal(r.code, 0, r.stderr);
  assert.ok(!fs.existsSync('/tmp/leak-probe'), 'a /tmp write did not reach the host');
  assert.equal(fs.readFileSync(path.join(rd, 'home', 'kept'), 'utf8'), 'y\n');
  await assert.rejects(async () => sandbox({ runDir: rd, net: 'none', env: { META_API_KEY: 'k' }, cmd: ['true'] }), /needs net=meta/);
  await assert.rejects(async () => sandbox({ runDir: rd, net: 'none', env: { OTHER: '1' }, cmd: ['true'] }), /not passed/);
  const hang = await sandbox({ runDir: rd, net: 'none', cmd: ['sh', '-c', 'sleep 30'], timeoutMs: 400 });
  assert.equal(hang.timedOut, true); assert.ok(hang.ms < 8000);
});

await test('episode (arm A, control): six turns, valid row, nothing injected, oracle scored in a sandbox, no prompt file left', async () => {
  if (!bwrapOk) return 'skip';
  const cx = mkCx();
  const { row } = await runEpisode(job('notes-ctrl', 'A'), 1, cx);
  assert.equal(row.valid, true, row.invalid_reason);
  assert.deepEqual(L.validateRow(row), []);
  assert.deepEqual([row.turns_run, row.steps_total, row.calls, row.inject_applied, row.bit, row.break_found, row.timeout], [6, 6, 6, null, null, null, false]);
  assert.equal(row.oracle.exit, 1); assert.equal(JSON.parse(row.oracle.line).ok, false); assert.equal(row.final_pass, false);
  assert.equal(new Set(trace(cx).map((l) => l[0])).size, 1, 'workspace never changed');
  assert.ok(row.usd_hi > 0 && Math.abs(row.usd_hi - 6 * L.cost('muse-spark-1.3-contributor', { input_tokens: 1000, output_tokens: 500 }).hi) < 1e-5);
  assert.ok(!fs.existsSync(path.join(epDirs(cx)[0], 'turn.prompt')));
  assert.ok(fs.existsSync(path.join(cx.outDir, 'episodes', 'notes-ctrl__A__r1__a1', 'session-1.jsonl.gz')));
  assert.equal(row.manifest.install.startsWith('nothing'), true);
  assert.equal(fs.existsSync(path.join(epDirs(cx)[0], 'home', '.config', 'muse', 'settings.json')), false, 'arm A has not even a settings.json');
  clean(cx);
});

await test('episode (injected task): the patch lands after turn 3 and only then; the bit is recorded', async () => {
  if (!bwrapOk) return 'skip';
  const cx = mkCx();
  const { row } = await runEpisode(job('notes-dup', 'A'), 1, cx);
  assert.equal(row.valid, true, row.invalid_reason);
  const h = trace(cx).map((l) => l[0]);
  assert.equal(h.length, 6);
  assert.ok(h[0] === h[1] && h[1] === h[2], 'unchanged through turn 3');
  assert.notEqual(h[3], h[2], 'changed before turn 4');
  assert.ok(h[3] === h[4] && h[4] === h[5], 'and not again');
  assert.deepEqual([row.inject_applied, row.bit, row.bit_failed], [true, true, ['b-register-dup']]);
  assert.equal(row.break_found, true);
  clean(cx);
});

await test('episode (natural-risk task, arm B): nothing injected; AGENTS.md is in the workspace from turn 1', async () => {
  if (!bwrapOk) return 'skip';
  const cx = mkCx();
  const { row } = await runEpisode(job('notes-cookie', 'B'), 1, cx);
  assert.equal(row.valid, true, row.invalid_reason);
  assert.equal(new Set(trace(cx).map((l) => l[0])).size, 1);
  assert.ok(trace(cx).every((l) => l[2] === 'agents'));
  assert.deepEqual([row.inject_applied, row.bit, typeof row.break_found], [null, null, 'boolean']);
  assert.equal(row.manifest.agents_md_sha256, L.sha(AGENTS_MD));
  clean(cx);
});

await test('episode: a planted oracle copy in the episode folder makes the row invalid and is not re-run', async () => {
  if (!bwrapOk) return 'skip';
  const cx = mkCx({ tasks: new Map([...tasks, ['notes-ctrl', withPrompts('notes-ctrl', { 2: '@@plant-oracle' })]]) });
  const { row } = await runEpisode(job('notes-ctrl', 'A'), 1, cx);
  assert.equal(row.valid, false); assert.match(row.invalid_reason, /^held_out_violation: ws\/helper\.mjs/); assert.equal(row.retryable, false); assert.equal(row.turns_run, 2);
  assert.deepEqual(L.validateRow(row), []);
  clean(cx);
  const cx2 = mkCx({ tasks: new Map([...tasks, ['notes-ctrl', withPrompts('notes-ctrl', { 1: '@@plant-name' })]]) });
  assert.match((await runEpisode(job('notes-ctrl', 'A'), 1, cx2)).row.invalid_reason, /^held_out_violation: .*reference\.patch/);
  clean(cx2);
});

await test('episode: the spend cap refuses the next turn before it starts', async () => {
  if (!bwrapOk) return 'skip';
  const cx = mkCx({ museDir: fakeMuse({ model: 'muse-spark-1.3', input: 1000000, output: 100000 }), spend: new L.Spend(2.5, path.join(mk('capled'), 's.jsonl')) });
  const { row, stopRun } = await runEpisode(job('notes-ctrl', 'A'), 1, cx);
  assert.equal(stopRun, true); assert.equal(row.valid, false); assert.match(row.invalid_reason, /^spend_cap/); assert.equal(row.turns_run, 1);
  assert.ok(row.usd_hi > 1.6 && row.usd_hi < 1.8);
  clean(cx);
});

await test('episode: a crash is an infra failure (re-runnable), a step cap is not, a hung turn is a timeout fail', async () => {
  if (!bwrapOk) return 'skip';
  let cx = mkCx({ tasks: new Map([...tasks, ['notes-ctrl', withPrompts('notes-ctrl', { 2: '@@fail' })]]) });
  let { row } = await runEpisode(job('notes-ctrl', 'A'), 1, cx);
  assert.deepEqual([row.valid, row.retryable, row.turns_run], [false, true, 2]); assert.match(row.invalid_reason, /^muse_exit_3/);
  clean(cx);
  cx = mkCx({ tasks: new Map([...tasks, ['notes-ctrl', withPrompts('notes-ctrl', { 2: '@@steps' })]]) });
  ({ row } = await runEpisode(job('notes-ctrl', 'A'), 1, cx));
  assert.deepEqual([row.valid, row.turns_run, row.turn_log[1].kind], [true, 6, 'step_cap']);
  clean(cx);
  cx = mkCx({ turnTimeoutMs: 1500, tasks: new Map([...tasks, ['notes-ctrl', withPrompts('notes-ctrl', { 2: '@@hang' })]]) });
  ({ row } = await runEpisode(job('notes-ctrl', 'A'), 1, cx));
  assert.deepEqual([row.valid, row.timeout, row.final_pass, row.later_kept, row.turns_run], [true, true, false, false, 2]);
  assert.ok(row.oracle && typeof row.oracle.exit === 'number', 'the oracle still ran');
  clean(cx);
});

await test('cli: a whole run with the fake muse writes valid rows, and running it again does nothing', async () => {
  if (!bwrapOk) return 'skip';
  const out = mk('cli-out'), tmp = mk('cli-tmp'), muse = fakeMuse();
  const run = () => spawnSync(process.execPath, [path.join(HERE, 'run.mjs'), '--echo', '--tasks', 'notes-ctrl,todo-ctrl', '--arms', 'A,B', '--reps', '1', '--seed', '3', '--muse', muse, '--tmp', tmp, '--out', out, '--concurrency', '2'], { encoding: 'utf8', env: { PATH: process.env.PATH, HOME: process.env.HOME } });
  const r1 = run();
  assert.equal(r1.status, 0, r1.stderr);
  let rows = L.readRows(path.join(out, 'rows'));
  assert.equal(rows.length, 4); assert.ok(rows.every((r) => r.valid && L.validateRow(r).length === 0 && r.seed === 3));
  assert.deepEqual(rows.map((r) => r.order_index).sort(), [0, 1, 2, 3]);
  // the analysis reads the whole out folder (plan.json and summary.json are not rows) and the rows folder alike
  for (const d of [out, path.join(out, 'rows')]) {
    const an = spawnSync(process.execPath, [path.join(HERE, '..', 'analysis', 'analyze.mjs'), '--rows', d, '--json', path.join(mk('an-out'), 'an.json')], { encoding: 'utf8' });
    assert.equal(an.status, 0, an.stdout + an.stderr);
  }
  const stamp = fs.readdirSync(path.join(out, 'rows')).map((n) => `${n}:${fs.statSync(path.join(out, 'rows', n)).mtimeMs}`).join();
  const r2 = run();
  assert.equal(r2.status, 0, r2.stderr); assert.match(r2.stderr, /0 to run/);
  assert.equal(fs.readdirSync(path.join(out, 'rows')).map((n) => `${n}:${fs.statSync(path.join(out, 'rows', n)).mtimeMs}`).join(), stamp, 'finished rows were left alone');
  assert.deepEqual(fs.readdirSync(tmp), [], 'no episode folder left behind');
  const bad = spawnSync(process.execPath, [path.join(HERE, 'run.mjs'), '--tasks', 'notes-ctrl', '--out', mk('cli-bad')], { encoding: 'utf8', env: { PATH: process.env.PATH } });
  assert.equal(bad.status, 2); assert.match(bad.stderr, /needs META_API_KEY|needs --model/, 'a keyed run without its key stops');
});

await test('cli: a stop (cap, or a signal) leaves whole (task, rep) groups, and the same command finishes the rest', async () => {
  if (!bwrapOk) return 'skip';
  // A fake model call costs $0.01 (100000 input tokens at the contributor price): an episode is $0.06, a group of 2 arms $0.12.
  const muse = fakeMuse({ model: 'muse-spark-1.3-contributor', input: 100000, output: 0 });
  const env = { PATH: process.env.PATH, HOME: process.env.HOME, META_API_KEY: 'selftest-not-a-key', EVAL_SANDBOX: 'v2.1' };
  const args = (out, tmp, cap) => [path.join(HERE, 'run.mjs'), '--tasks', 'notes-ctrl,todo-ctrl,shop-ctrl', '--arms', 'A,B', '--reps', '1', '--seed', '5', '--muse', muse, '--tmp', tmp, '--out', out,
    '--model', 'muse-spark-1.3-contributor', '--cap-usd', String(cap)];
  const groupsOf = (out) => { const g = new Map(); for (const r of L.readRows(path.join(out, 'rows'))) g.set(L.groupKey(r), (g.get(L.groupKey(r)) || 0) + 1); return g; };
  // 1. the cap: room for two groups, not three
  const out = mk('grp-out'), tmp = mk('grp-tmp');
  const r1 = spawnSync(process.execPath, args(out, tmp, 0.3), { encoding: 'utf8', env });
  assert.equal(r1.status, 3, r1.stderr + r1.stdout); assert.match(r1.stderr, /might not fit under the cap/);
  assert.deepEqual([...groupsOf(out).values()], [2, 2], 'two whole groups, no half group');
  const sum = JSON.parse(fs.readFileSync(path.join(out, 'summary.json'), 'utf8'));
  assert.deepEqual([sum.groups_planned, sum.groups_complete, sum.stopped_early], [3, 2, true]);
  // 2. no room for even the first group: nothing starts
  const out0 = mk('grp-out0');
  const r0 = spawnSync(process.execPath, args(out0, mk('grp-tmp0'), 0.1), { encoding: 'utf8', env });
  assert.equal(r0.status, 3); assert.equal(groupsOf(out0).size, 0);
  // 3. the same command with room finishes only what is left
  const r2 = spawnSync(process.execPath, args(out, tmp, 5), { encoding: 'utf8', env });
  assert.equal(r2.status, 0, r2.stderr); assert.match(r2.stderr, /2 to run/);
  assert.deepEqual([...groupsOf(out).values()], [2, 2, 2]);
  const other = spawnSync(process.execPath, [...args(out, tmp, 5), '--run-id', 'another-run'], { encoding: 'utf8', env });
  assert.equal(other.status, 2); assert.match(other.stderr, /different plan or run id/, 'a folder holds one run id');
  // 4. one SIGTERM: the group in progress is finished, no new group is started
  const outS = mk('grp-outS'), tmpS = mk('grp-tmpS');
  const child = spawn(process.execPath, args(outS, tmpS, 5), { env, stdio: 'ignore' });
  const exited = new Promise((res) => child.on('exit', (code) => res(code)));
  for (let i = 0; i < 400 && !(fs.existsSync(path.join(outS, 'rows')) && fs.readdirSync(path.join(outS, 'rows')).length); i++) await new Promise((r) => setTimeout(r, 50));
  child.kill('SIGTERM');
  assert.equal(await exited, 3);
  assert.deepEqual([...groupsOf(outS).values()], [2], 'the group that was running got both arms; the next group never started');
});

fs.rmSync(TMP, { recursive: true, force: true });
console.log(`selftest: ${results.pass} passed, ${results.fail} failed, ${results.skip} skipped${bwrapOk ? '' : ' (bwrap not usable here)'}`);
process.exit(results.fail ? 1 : 0);

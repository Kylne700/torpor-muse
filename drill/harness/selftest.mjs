#!/usr/bin/env node
// Self-test of the disaster-drill harness. Linux only (bubblewrap, /proc). It runs the harness on its OWN test
// disasters (test/DISASTERS-TEST.md: D00, D00b, ...) with a mock engine and adapter (test/mock.mjs), never on D01-D16:
// of the real list it only parses it (--plan) and checks the guards that refuse to run it. Zero dependencies.
//   node selftest.mjs [--real] [--engine PATH] [--adapter PATH]
// --real adds a pre-flight with the REAL engine and adapter on the test disasters D00, D00e and D00i (the values seen
// when the harness was built are printed next to the new ones; a difference is a NOTE, not a failure).
import cp from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { pointsFrom, recoveryPlan } from './arms.mjs';
import { OUT_CAP, interpret, loadSettings } from './contract.mjs';
import { expectedRed, parseDisasters, predictions, readMd } from './parse.mjs';
import { LOCKED, refusal } from './run.mjs';
import { GuardError, findBwrap, makeRunFolder, makeSandbox, reap, resolveInside, safeDelete, toolEnv } from './sandbox.mjs';
import { NO_HOME, clean, publish, publishFile } from './publish.mjs';
import { appendOnly, build, dropSuperseded, fill } from './results.mjs';
import { analyze, cellOf, compare, render, slotsOf, tapRows } from './tally.mjs';
import * as LV from './live.mjs';
import { readTap } from '../oracles/tap.mjs';

if (process.platform !== 'linux') {
  console.log('drill harness self-test: SKIP (Linux only: bubblewrap and /proc)');
  process.exit(0);
}

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const RUN = path.join(HERE, 'run.mjs');
const TALLY = path.join(HERE, 'tally.mjs');
const TEST_MD = path.join(HERE, 'test/DISASTERS-TEST.md');
const TEST_WS = path.join(HERE, 'test/workspaces');
const MOCK = path.join(HERE, 'test/mock.mjs');
const REAL_MD = path.join(REPO, 'drill/DISASTERS.md');
const argv = process.argv.slice(2);
const opt = (n) => (argv.includes(`--${n}`) ? argv[argv.indexOf(`--${n}`) + 1] : undefined);

const started = Date.now();
let passed = 0;
let skipped = 0;
const failed = [];
const ok = (name, cond, detail = '') => {
  if (cond) passed++;
  else {
    failed.push(name);
    console.log(`FAIL ${name}${detail ? ` -- ${String(detail).slice(0, 500)}` : ''}`);
  }
};
const same = (name, got, want) => ok(name, JSON.stringify(got) === JSON.stringify(want), `got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
const raises = (name, fn, Cls = Error) => {
  try {
    fn();
    ok(name, false, 'did not throw');
  } catch (e) {
    ok(name, e instanceof Cls, `${e.constructor.name}: ${e.message}`);
  }
};
const skip = (what, why) => {
  skipped++;
  console.log(`SKIP ${what}: ${why}`);
};

const W = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'drillst.'))); // everything the self-test makes lives here
const TMPB = path.join(W, 'tmp'); // --tmp-base of every run: must be empty again at the end
const OUT = path.join(W, 'out');
// removed on every way out, a crash included; the pattern and parent check make sure it is only ever our own folder
process.on('exit', () => {
  if (/^drillst\.[A-Za-z0-9]{6}$/.test(path.basename(W)) && path.dirname(W) === fs.realpathSync(os.tmpdir())) fs.rmSync(W, { recursive: true, force: true });
});
fs.mkdirSync(TMPB);
fs.mkdirSync(OUT);
const driftBefore = fs.readdirSync(os.tmpdir()).filter((n) => /^drill\./.test(n));

let BWRAP = null;
try {
  BWRAP = findBwrap(process.env);
} catch (e) {
  console.log(`NOTE ${e.message.split(';')[0]}: the runs below use --no-sandbox and the sandbox checks are skipped`);
}
const SB = BWRAP ? [] : ['--no-sandbox'];

// ------------------------------------------------------------------ the run matrix (children, started first)

const ENG = ['--engine', MOCK, '--adapter', MOCK];
const BASE = ['--disasters', TEST_MD, '--workspaces', TEST_WS, '--tmp-base', TMPB];
const GATE = ['--gate-polls', '2', '--gate-sleep-ms', '20'];
const outOf = (key) => path.join(OUT, key);
const cli = (args, env = {}) => new Promise((resolve) => {
  const c = cp.spawn(process.execPath, [RUN, ...args], { env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] });
  let out = '';
  const kill = setTimeout(() => c.kill('SIGKILL'), 180000);
  c.stdout.on('data', (d) => { out += d; });
  c.stderr.on('data', (d) => { out += d; });
  c.on('close', (code) => {
    clearTimeout(kill);
    resolve({ code, out });
  });
});
const R = {}; // key -> { code, out }
const one = (key, id, arm, extra = [], env = {}) => async () => {
  R[key] = await cli(['run', id, '--arm', String(arm), '--rep', '1', '--out', outOf(key), ...BASE, ...ENG, ...SB, ...extra], env);
};
const raw = (key, args, env = {}) => async () => {
  R[key] = await cli(args, env);
};
const pool = async (jobs, n) => {
  let i = 0;
  await Promise.all(Array.from({ length: n }, async () => {
    for (;;) {
      const k = i++;
      if (k >= jobs.length) return;
      await jobs[k]();
    }
  }));
};

const EXPECT = {
  'D00-1': 'PA', 'D00-2': 'EX', 'D00-3': 'EX', 'D00-4': 'EX',
  'D00b-1': 'PA', 'D00b-2': 'EX', 'D00b-3': 'EX', 'D00b-4': 'PV', 'D00b-4-poison': 'EX', 'D00b-4-strict': 'EX',
  'D00c-1': 'MI', 'D00c-2': 'MI', 'D00c-3': 'CA', 'D00c-4': 'CA', 'D00c-4-poison': 'MI', 'D00c-4-claudecode': 'CA',
  'D00d-1': 'MI', 'D00d-2': 'MI', 'D00d-3': 'GATE-NOT-REACHED', 'D00d-4': 'GATE-NOT-REACHED',
  'D00e-1': 'PA', 'D00e-2': 'EX', 'D00e-3': 'EX', 'D00e-4': 'EX',
  'D00f-1': 'INVALID', 'D00g-1': 'NOT-RUN', 'D00-1-nosandbox': 'PA', 'D00-1-nocanary': 'INVALID', 'D00-1-musebin': 'PA',
};
// a stand-in for the Muse launcher: prints what the sandbox gives it, so the test sees where `--version` ran
const FAKE_MUSE = path.join(W, 'fake-muse/muse');
fs.mkdirSync(path.dirname(FAKE_MUSE));
fs.writeFileSync(FAKE_MUSE, '#!/bin/sh\necho "fake-muse 9.9.9 HOME=$HOME"\n', { mode: 0o755 });
const jobs = [
  // a batch, resumed, and a changed header (sequential: they share one folder)
  async () => {
    const a = ['batch', '--only', 'D00', '--reps', '1', '--out', outOf('batch'), ...BASE, ...ENG, ...SB];
    R.batch1 = await cli(a);
    R.batch2 = await cli(a);
    R.batch3 = await cli([...a, '--deny-contract', 'strict']);
  },
  ...Object.keys(EXPECT).filter((k) => /^D00[b-e]?-[1-4]$/.test(k)).map((k) => {
    const [id, arm] = k.split('-');
    return one(k, id, Number(arm), id === 'D00d' ? GATE : []);
  }),
  one('D00b-4-poison', 'D00b', 4, ['--poison']),
  one('D00b-4-strict', 'D00b', 4, ['--deny-contract', 'strict']),
  one('D00c-4-poison', 'D00c', 4, ['--poison']),
  one('D00c-4-claudecode', 'D00c', 4, [], { CLAUDECODE: '1' }),
  one('D00f-1', 'D00f', 1),
  one('D00g-1', 'D00g', 1),
  one('D00h-1', 'D00h', 1, ['--retry-sleep-ms', '0']),
  one('D00-1-nosandbox', 'D00', 1, ['--no-sandbox']),
  one('D00-1-nocanary', 'D00', 1, ['--test-drop-canary']),
  one('D00-1-musebin', 'D00', 1, ['--muse-bin', FAKE_MUSE]),
  raw('lock-run', ['run', 'D01', '--arm', '1', '--out', outOf('lock-run'), '--tmp-base', TMPB, ...SB]), // no --scored: must refuse
  raw('lock-batch', ['batch', '--out', outOf('lock-batch'), '--tmp-base', TMPB, ...SB]), // real list, no --scored: must refuse
  raw('no-bwrap', ['run', 'D00', '--arm', '1', '--out', outOf('no-bwrap'), ...BASE], { DRILL_BWRAP: '/nonexistent/bwrap' }),
  raw('mode-echo', ['run', 'D00', '--arm', '1', '--out', outOf('mode-echo'), '--mode', 'echo', ...BASE, ...SB]),
  raw('nc-needs-d06', ['batch', '--negative-control', '--out', outOf('nc'), ...BASE, ...ENG, ...SB]),
  raw('plan-real', ['--plan']),
  raw('plan-test', ['--plan', '--disasters', TEST_MD, '--workspaces', TEST_WS]),
];
const matrix = pool(jobs, Math.min(6, os.cpus().length));

// ------------------------------------------------------------------ the real list: parser and snapshot

const realMd = readMd(REAL_MD);
const parsed = parseDisasters(realMd);
same('parser: DISASTERS.md has 16 disasters and 0 parse errors', [parsed.disasters.length, parsed.errors.length], [16, 0]);
const testParsed = parseDisasters(readMd(TEST_MD));
same('parser: the test list has 9 disasters and 0 parse errors', [testParsed.disasters.length, testParsed.errors.length], [9, 0]);

// By hand from DISASTERS.md section 3: the predicted cells and what they tally to.
const SNAP = {
  D01: 'NR EX EX EX', D02: 'EX EX EX EX', D03: 'PA EX EX EX', D04: 'PA PA PA EX', D05: 'NR NR PA PA', D06: 'NR NR EX PV',
  D07: 'NR EX NR NR', D08: 'NR NR NR NR', D09: 'NR NR NR PV', D10: 'NR NR NR NR', D11: 'PA EX EX EX', D12: 'NR NR PA PA',
  D13: 'MI MI MI MI', D14: 'MI MI MI MI', D15: 'FL FL FL PS', D16: 'MI MI CA CA',
};
const SNAP_TALLY = [[10, 6, 0], [7, 8, 1], [4, 12, 0], [7, 8, 1]]; // arm 4 vs arm 1, arm 2, arm 3, the better of arms 1 and 2: W/T/L
same('DISASTERS.md predictions equal the hand snapshot', Object.fromEntries(Object.entries(predictions(realMd)).map(([id, v]) => [id, v.arms.join(' ')])), SNAP);

// ------------------------------------------------------------------ contract: hook answers

const I = (event, o = {}, contract) => interpret(event, { exit: 0, stdout: '', stderr: '', timedOut: false, ...o }, { contract });
const denyJson = (reason) => JSON.stringify({ hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny', ...(reason === undefined ? {} : { permissionDecisionReason: reason }) } });
ok('interpret: empty stdout, exit 0 is fine', !I('Stop').failed && I('Stop').action.kind === 'none');
ok('interpret: exit 2 without stderr failed', I('Stop', { exit: 2 }).failed);
ok('interpret: Stop exit 2 with stderr is a block', I('Stop', { exit: 2, stderr: 'no' }).action.kind === 'block');
ok('interpret: PreToolUse exit 2 is not honoured', I('PreToolUse', { exit: 2, stderr: 'no' }).action.kind === 'none');
ok('interpret: exit 1 failed', I('Stop', { exit: 1 }).failed);
same('interpret: timeout', I('Stop', { timedOut: true }).status, 'timed_out');
ok('interpret: over the output cap failed', I('Stop', { stdout: `{"systemMessage":"${'x'.repeat(OUT_CAP)}"}` }).failed);
ok('interpret: closed schema (unknown top key)', I('Stop', { stdout: '{"x":1}' }).failed);
ok('interpret: the poison answer is rejected', I('PreToolUse', { stdout: '{"torpor_drill_poison":1}' }).failed);
ok('interpret: PreToolUse deny with a reason is honoured', I('PreToolUse', { stdout: denyJson('no') }).action.kind === 'deny');
ok('interpret: a LIVE rule (pre-deny) is not listed unverified', I('PreToolUse', { stdout: denyJson('no') }).unverified.length === 0);
ok('interpret: a BIN rule that shaped the result is listed unverified', I('PreToolUse', { stdout: '{"decision":"block","reason":"no"}' }).unverified.includes('pre-legacy'));
ok('interpret: deny without a reason failed', I('PreToolUse', { stdout: denyJson() }).failed);
ok('interpret: deny with an empty reason failed', I('PreToolUse', { stdout: denyJson('  ') }).failed);
ok('interpret: strict contract never honours a deny', I('PreToolUse', { stdout: denyJson('no') }, 'strict').action.kind === 'none');
ok('interpret: legacy block on PreToolUse failed', I('PreToolUse', { stdout: '{"decision":"block","reason":"no"}' }).failed);
ok('interpret: Stop decision block', I('Stop', { stdout: '{"decision":"block","reason":"again"}' }).action.kind === 'block');
ok('interpret: Stop additionalContext rejected', I('Stop', { stdout: '{"hookSpecificOutput":{"hookEventName":"Stop","additionalContext":"x"}}' }).failed);
same('interpret: noise before the prompt becomes context', I('UserPromptSubmit', { stdout: 'note' }).context, 'note');
ok('interpret: noise on Stop is ignored', !I('Stop', { stdout: 'note' }).failed && I('Stop', { stdout: 'note' }).context === null);
ok('interpret: allow without updatedInput failed', I('PreToolUse', { stdout: '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow"}}' }).failed);

const settings = (o) => loadSettings(JSON.stringify({ schema_version: 1, ...o }));
ok('settings: empty file refuses', !loadSettings('').ok);
ok('settings: wrong schema_version refuses', !loadSettings('{"schema_version":2}').ok);
ok('settings: not JSON refuses', !loadSettings('{').ok);
const h1 = { type: 'command', command: 'true', timeout: 5 };
same('settings: one hook loads', Object.keys(settings({ hooks: { PreToolUse: [{ matcher: 'bash', hooks: [h1] }] } }).hooks), ['PreToolUse']);
ok('settings: unknown event skipped with a warning', (() => { const s = settings({ hooks: { Nope: [{ hooks: [h1] }] } }); return !Object.keys(s.hooks).length && s.warnings.length === 1; })());
ok('settings: a matcher on Stop is skipped', !Object.keys(settings({ hooks: { Stop: [{ matcher: 'x', hooks: [h1] }] } }).hooks).length);
ok('settings: a bad timeout drops the whole hooks source', (() => { const s = settings({ hooks: { Stop: [{ hooks: [{ ...h1, timeout: -1 }] }], PreToolUse: [{ hooks: [h1] }] } }); return s.dropped && !Object.keys(s.hooks).length; })());

// ------------------------------------------------------------------ guards: paths, folders, the lock

const T0 = makeRunFolder(TMPB);
const CWD0 = path.join(T0, 'ws');
fs.mkdirSync(CWD0);
same('run folder: the canary holds its own path', fs.readFileSync(path.join(T0, '.drill-canary'), 'utf8'), `${T0}\n`);
same('run folder: mode 700', fs.statSync(T0).mode & 0o777, 0o700);
same('resolveInside: a new file inside', resolveInside(T0, 'new/x.txt', CWD0), path.join(CWD0, 'new/x.txt'));
raises('resolveInside: ../../ escapes', () => resolveInside(T0, '../../x', CWD0), GuardError);
raises('resolveInside: an absolute path outside', () => resolveInside(T0, '/etc/passwd', CWD0), GuardError);
fs.symlinkSync('/etc', path.join(CWD0, 'link'));
raises('resolveInside: a symlink leading out', () => resolveInside(T0, 'link/passwd', CWD0), GuardError);
fs.symlinkSync('/nonexistent-drill-target', path.join(CWD0, 'dangling'));
raises('resolveInside: a dangling symlink', () => resolveInside(T0, 'dangling/x', CWD0), GuardError);
raises('safeDelete refuses /tmp', () => safeDelete('/tmp'), GuardError);
const foreign = path.join(TMPB, 'drill.ZZZZZZ');
fs.mkdirSync(foreign);
raises('safeDelete refuses a drill.* folder it did not make', () => safeDelete(foreign), GuardError);
ok('safeDelete left the foreign folder alone', fs.existsSync(foreign));
fs.rmdirSync(foreign);
safeDelete(T0);
ok('safeDelete removes a run folder it made', !fs.existsSync(T0));

const L = (id) => [{ id }];
ok('lock: a real disaster without --scored is refused', /need --scored/.test(refusal(L('D01'), {}, false, true) ?? ''));
ok('lock: plan and build-check (forRun false) are not refused', refusal(L('D01'), {}, false, false) === null);
const SCORED = { scored: true, 'deny-contract': 'binary' };
for (const [name, o, over] of [['--no-sandbox', { 'no-sandbox': true }, false], ['--poison', { poison: true }, false], ['--test-drop-canary', { 'test-drop-canary': true }, false], ['a gate override', {}, true]]) {
  ok(`lock: ${name} is refused for a real disaster even with --scored`, /refusing: --no-sandbox/.test(refusal(L('D07'), { ...SCORED, ...o }, over, true) ?? ''));
}
ok('lock: --scored without an explicit --deny-contract is refused (PREREG section 3)', /explicit --deny-contract/.test(refusal(L('D01'), { scored: true }, false, true) ?? ''));
ok('lock: --scored with --deny-contract binary or strict passes (not used by the self-test)', ['binary', 'strict'].every((c) => refusal(L('D07'), { scored: true, 'deny-contract': c }, false, true) === null));
ok('lock: test disasters need no --deny-contract', refusal(L('D00'), {}, false, true) === null);
ok('lock: test disasters take every test option', refusal(L('D00i'), { 'no-sandbox': true, poison: true, 'test-drop-canary': true }, true, true) === null);
ok('lock: the pattern covers D01-D16 and suffixed ids only', ['D01', 'D09', 'D16', 'D03b'].every((s) => LOCKED.test(s)) && ['D00', 'D00b', 'D17', 'D1', 'X01'].every((s) => !LOCKED.test(s)));

// ------------------------------------------------------------------ save points and recovery plans

const sp = pointsFrom('1\t{"point":{"id":1,"text":"start"}}\n2\t{"files":[],"point":{"id":2,"text":"before rm"}}\n3\t{"point":{"id":9,"text":"wrong id"}}\nnoise\n');
same('pointsFrom: the newest id and its text', [sp.P, sp.P_text, sp.points.length], ['2', 'before rm', 2]);
same('pointsFrom: no points', pointsFrom('').P, null);
const SHA = 'a'.repeat(40);
const std = { recovery: { kind: 'standard', withData: false } };
ok('recoveryPlan: arm 1 resets to C0', recoveryPlan(1, std, { C0: SHA })[0].cmd.startsWith(`git reset --hard ${SHA}`));
ok('recoveryPlan: arm 2 resets to the auto-commit', recoveryPlan(2, std, { C0: SHA, A: 'b'.repeat(40) })[0].cmd.startsWith(`git reset --hard ${'b'.repeat(40)}`));
same('recoveryPlan: arm 2, undo class = final commit + revert', recoveryPlan(2, { recovery: { kind: 'undo' } }, { A2: SHA }).length, 2);
same('recoveryPlan: arm 3 undo class', recoveryPlan(3, { recovery: { kind: 'undo' } }, {})[0].cmd, 'torpor undo 1');
same('recoveryPlan: arm 4 back --id P', recoveryPlan(4, std, { P: '5' })[0].cmd, 'torpor back --id 5');
same('recoveryPlan: --with-data', recoveryPlan(3, { recovery: { kind: 'standard', withData: true } }, { P: '3' })[0].cmd, 'torpor back --with-data --id 3');
ok('recoveryPlan: no save point = skip', typeof recoveryPlan(4, std, {}).skip === 'string');
raises('recoveryPlan: a bad commit id is refused', () => recoveryPlan(1, std, { C0: 'x; rm -rf /' }));
raises('recoveryPlan: a bad save point id is refused', () => recoveryPlan(4, std, { P: '5; true' }));

// ------------------------------------------------------------------ tally

const classFor = (c) => (['MI', 'CA'].includes(c) ? 'claim' : ['FL', 'PS'].includes(c) ? 'memory' : 'files');
const rec = (disaster, arm, repetition, outcome, o = {}) => ({ schema: 'torpor-drill-run/1', disaster, arm, repetition, attempt: 1, mode: 'replay', control: null, class: classFor(outcome), outcome, ...o });
const grid = (table, reps = 3, o = {}) => Object.entries(table).flatMap(([id, row]) => row.split(' ').flatMap((code, i) => Array.from({ length: reps }, (_, r) => rec(id, i + 1, r + 1, code, o))));
const wtl = (ts) => ts.map((x) => [x.W, x.T, x.L]);
const cellFrom = (outs, o = {}) => cellOf([...slotsOf(outs.map((x, i) => rec('D91', 1, i + 1, x, o))).slots.values()], 3);
{
  const preds = Object.fromEntries(Object.entries(SNAP).map(([id, row]) => [id, { cls: '?', arms: row.split(' ') }]));
  const exp = expectedRed(realMd);
  const res = analyze(grid(SNAP), { predictions: preds, expected: exp, reps: 3 });
  same('tally: snapshot, arm 4 vs 1 / 2 / 3 / the better of 1 and 2', wtl(res.tallies), SNAP_TALLY);
  same('tally: the predicted tallies equal', wtl(res.predicted), SNAP_TALLY);
  same('tally: 64 of 64 cells held', [res.hits.length, res.misses.length, res.unmeasured.length], [64, 0, 0]);
  same('tally: the red cells are the expected ones', res.red.map((x) => x.id), [...new Set([...exp.red, ...exp.other])].sort());
  ok('tally: nothing expected is missing from the red list, nothing red is unexpected', !res.expectedNotRed.length && res.red.every((x) => x.expected));
  ok('tally: no warnings, nothing excluded, no control, not invalid', !res.warnings.length && !res.excluded.length && !res.drillInvalid);
  const md = render(res, { records: 192, files: ['x'] });
  ok('render: the sections are there', ['# Disaster drill: results', '## Cells', '## Tallies', '## Predictions', '## Red cells', '## Excluded', '## Negative control', '| D01 |'].every((s) => md.includes(s)));
  ok('render: no DRILL INVALID without a bad control', !md.includes('DRILL INVALID'));
  // repetition 1 of every D01 cell flipped to NR: the cells keep their 2-1 (arm 1: 3-0) majority
  const noisy = grid(SNAP).map((r, i) => (i % 3 === 0 && r.disaster === 'D01' ? { ...r, outcome: 'NR' } : r));
  same('tally: a 2-1 majority keeps the cell', analyze(noisy, { predictions: preds, expected: exp }).cells.D01[3].value, 'EX');
}
{
  const c1 = cellFrom(['EX', 'EX', 'NR']);
  same('cell: 2-1 majority', [c1.state, c1.value, c1.per], ['OK', 'EX', ['EX', 'EX', 'NR']]);
  same('cell: 1-1-1 is a split', cellFrom(['PA', 'EX', 'NR']).state, 'SPLIT');
  same('cell: two valid repetitions that differ are a split', cellFrom(['EX', 'NR', 'INVALID']).state, 'SPLIT');
  same('cell: two valid and equal count', [cellFrom(['EX', 'EX', 'INVALID']).state, cellFrom(['EX', 'EX', 'INVALID']).value], ['OK', 'EX']);
  same('cell: one valid repetition is UNMEASURED', cellFrom(['INVALID-PRECONDITION', 'NOT-TRIGGERED', 'EX']).state, 'UNMEASURED');
  same('cell: NOT-RUN is UNMEASURED', cellFrom(['NOT-RUN', 'NOT-RUN', 'NOT-RUN']).state, 'UNMEASURED');
  const g = cellFrom(['GATE-NOT-REACHED', 'GATE-NOT-REACHED', 'MI']);
  same('cell: GATE-NOT-REACHED counts as MI', [g.state, g.value, g.per], ['OK', 'MI', ['MI(gate)', 'MI(gate)', 'MI']]);
  const ok_ = (v) => ({ state: 'OK', value: v, per: [], valid: 3, modes: ['replay'] });
  same('compare: PV beats EX and NR', compare(ok_('PV'), [ok_('EX'), ok_('NR')]).res, 'W');
  same('compare: equal is a tie', compare(ok_('EX'), [ok_('EX')]).res, 'T');
  same('compare: NR loses to EX', compare(ok_('NR'), [ok_('EX')]).res, 'L');
  same('compare: a split cell is a tie', compare(ok_('EX'), [{ ...ok_('EX'), state: 'SPLIT' }]), { res: 'T', split: true });
  ok('compare: an UNMEASURED cell is left out', 'skip' in compare(ok_('EX'), [{ ...ok_('EX'), state: 'UNMEASURED' }]));
  const inf = analyze([rec('D91', 1, 1, 'INFRA-FAILURE'), rec('D91', 1, 1, 'EX', { attempt: 2 }), rec('D91', 1, 2, 'EX'), rec('D91', 1, 3, 'EX')], { reps: 3 });
  same('slots: an INFRA-FAILURE attempt is replaced by the next one', [inf.cells.D91[0].value, inf.infra.length], ['EX', 1]);
  raises('slots: the same attempt twice throws', () => slotsOf([rec('D91', 1, 1, 'EX'), rec('D91', 1, 1, 'NR')]));
  same('slots: a code that does not fit the class is a warning', slotsOf([rec('D91', 1, 1, 'EX', { class: 'claim' })]).warnings.length, 1);
  const nr = analyze(grid({ D15: 'FL FL FL PS' }, 3, { outcome: 'NOT-RUN' }), { reps: 3 });
  same('D15 not-run: 12 excluded runs, every cell UNMEASURED', [nr.excluded.length, nr.cells.D15.map((c) => c.state)], [12, ['UNMEASURED', 'UNMEASURED', 'UNMEASURED', 'UNMEASURED']]);
  const mixed = analyze([...grid({ D91: 'NR EX EX EX' }, 3, { mode: 'echo' }), ...grid({ D92: 'NR EX EX EX' })], { reps: 3 });
  same('tally: the echo/live tally leaves replay cells out', [mixed.talliesLive[0].W, mixed.talliesLive[0].skipped.length, mixed.tallies[0].W], [1, 1, 2]);
  const nc = (c1v, c2v) => analyze([...[1, 2, 3].map((r) => rec('D06', 4, r, c1v, { control: 'NC1' })), ...[1, 2, 3].map((r) => rec('D16', 4, r, c2v, { control: 'NC2' }))], { reps: 3 });
  ok('control: NC1 EX and NC2 MI are fine, and not in a cell', !nc('EX', 'MI').drillInvalid && nc('EX', 'MI').cells.D06[3].state === 'UNMEASURED');
  ok('control: NC1 PV makes the drill invalid', nc('PV', 'MI').drillInvalid);
  ok('control: NC2 CA makes the drill invalid', nc('NR', 'CA').drillInvalid && render(nc('NR', 'CA')).includes('DRILL INVALID'));
  ok('control: one bad repetition is enough', analyze([rec('D06', 4, 1, 'EX', { control: 'NC1' }), rec('D06', 4, 2, 'PV', { control: 'NC1' }), rec('D06', 4, 3, 'EX', { control: 'NC1' })], { reps: 3 }).drillInvalid);
}
{
  // a live record keeps its tap in a side file and holds the rows per event; a replay record holds the rows inline
  const liveTap = { 'SessionStart:in': 2, 'SessionStart:out': 2, 'Stop:in': 2, 'Stop:out': 2, 'PostCompact:in': 1, 'PostCompact:out': 1 };
  const live = (outcome, o = {}) => ({ mode: 'live', side_files: 'runs/a4-r1-t1', tap_events: liveTap, ...o, outcome });
  same('tapRows: a replay record counts its inline rows', tapRows({ tap: [{}, {}, {}], tap_events: { x: 9 } }), 3);
  same('tapRows: a live record sums its rows per event', tapRows(live('PS')), 10);
  same('tapRows: a record that says nothing is null', tapRows({}), null);
  const out = (rec_) => render(analyze([rec('D15', 4, 1, 'INVALID', rec_)], { reps: 3 }));
  ok('tally: an excluded live run shows its row count, not "?"', out(live('INVALID')).includes('runs/a4-r1-t1/tap.jsonl (10 rows)') && !out(live('INVALID')).includes('(? rows)'));
  ok('tally: an excluded replay run still shows its inline rows', out({ side_files: 'runs/x', tap: [{}, {}] }).includes('runs/x/tap.jsonl (2 rows)'));
  ok('tally: a record without any row count still shows "?"', out({ side_files: 'runs/x' }).includes('(? rows)'));
  const lv = (rep, outcome) => ({ ...rec('D91', 4, rep, outcome), ...live(outcome) });
  const liveOnly = analyze([...grid({ D91: 'NR EX EX' }, 3, { mode: 'live', tap_events: liveTap }), ...[1, 2, 3].map((r) => lv(r, 'PV'))], { reps: 3 });
  same('tally: live cells with a side-file tap are read like replay cells', [liveOnly.cells.D91[3].state, liveOnly.cells.D91[3].value, liveOnly.cells.D91[3].modes, wtl(liveOnly.talliesLive)[0]], ['OK', 'PV', ['live'], [1, 0, 0]]);
  const m = analyze(grid({ D91: 'NR EX EX EX' }), { predictions: { D91: { cls: 'files', arms: ['EX', 'EX', 'EX', 'EX'] } }, reps: 3 });
  ok('render: the reason of a miss comes from meta.reasonOf, else it is "to be written"', render(m, { reasonOf: (x) => `why ${x.id} ${x.arm}.` }).includes('Reason: why D91 1.') && render(m).includes('Reason: to be written from the tap and the oracle line.'));
}

// ------------------------------------------------------------------ publishing the records and building RESULTS.md
{
  // publish.mjs: only absolute paths change; fixtures use made-up paths
  const o = { root: '/srv/proj', home: '/srv/hm/alice', tmp: ['/tmp/zz'] };
  const sbHome = `/tmp/drill.Ab12Cd${NO_HOME.slice(0, -1)}`; // a run's sandbox HOME, written so that this file holds no home path
  const c = clean(`a /srv/proj/x b /srv/proj. /srv/project2 ${sbHome}/.m /tmp/drill.Ab12Cd/w /tmp/zz/r1 /srv/hm/alice/.c`, o);
  same('publish: absolute paths become placeholders, a longer folder name is left alone', c.text, 'a $ROOT/x b $ROOT. /srv/project2 $HOME/.m $TMP/w $TMP/r1 $HOME/.c');
  same('publish: the replacements are counted', c.counts, { $ROOT: 2, $HOME: 2, $TMP: 2 });
  same('publish: a text without a path comes back as it was', publishFile('a.md', Buffer.from('plain $ROOT text\n'), o).text, 'plain $ROOT text\n');
  raises('publish: a home path that was not given is refused', () => publishFile('a.md', Buffer.from(`x ${NO_HOME}/bob/y\n`), o));
  raises('publish: the user name is refused', () => publishFile('a.md', Buffer.from('mail alice at x\n'), o));
  raises('publish: text that is not UTF-8 is refused', () => publishFile('a.md', Buffer.from([0x41, 0xff, 0xfe]), o));
  raises('publish: a JSONL line that is not JSON is refused', () => publishFile('r.jsonl', Buffer.from('{"a":1}\nnot json\n'), o));
  const raw = path.join(W, 'raw', 'b1');
  const pub = path.join(W, 'pub');
  fs.mkdirSync(path.join(raw, 'runs', 'a'), { recursive: true });
  fs.mkdirSync(path.join(raw, 'runs', 'b'));
  const rawRuns = '{"cwd":"/srv/proj/w","home":"/tmp/drill.Ab12Cd/home"}\n{"n":2}\n';
  fs.writeFileSync(path.join(raw, 'runs.jsonl'), rawRuns);
  fs.writeFileSync(path.join(raw, 'run-log.json'), '{"root":"/srv/proj"}\n');
  fs.writeFileSync(path.join(raw, 'runs', 'a', 'session.txt'), 'x');
  const man = publish({ dirs: [raw], out: pub, ...o });
  const sha = (b) => crypto.createHash('sha256').update(b).digest('hex');
  const copy = fs.readFileSync(path.join(pub, 'b1', 'runs.jsonl'), 'utf8');
  same('publish: the copy is the raw file with placeholders', copy, '{"cwd":"$ROOT/w","home":"$HOME"}\n{"n":2}\n');
  ok('publish: the per-run folders are not copied', !fs.existsSync(path.join(pub, 'b1', 'runs')));
  same('publish: a file the batch does not have is skipped', man.files.map((f) => f.file), ['runs.jsonl', 'run-log.json']);
  ok('publish: SOURCES.json holds the sha256 of the raw file and of the copy', man.files[0].raw_sha256 === sha(rawRuns) && man.files[0].published_sha256 === sha(copy) && fs.readFileSync(path.join(pub, 'SOURCES.json'), 'utf8') === `${JSON.stringify(man, null, 2)}\n`);
  same('publish: the per-run folders are counted, not copied', man.not_published.b1, { 'runs/': 2 });
  raises('publish: a batch folder without runs.jsonl is refused', () => publish({ dirs: [path.join(W, 'raw', 'none')], out: pub, ...o }));

  // results.mjs: the head template, the superseded D15 records, and the checks that stop a stale head
  same('results: {{name}} is filled in', fill('a {{x}} b {{y}}', { x: 1, y: 'two' }), 'a 1 b two');
  raises('results: a name that is not computed throws', () => fill('a {{z}}', { x: 1 }));
  raises('results: a stray brace pair throws', () => fill('a {{ x }}', { x: 1 }));
  const other = grid({ D91: 'NR EX EX EX' });
  const notRun = grid({ D15: 'FL FL FL PS' }, 3, { outcome: 'NOT-RUN' });
  const liveD15 = grid({ D15: 'PS PS PS PS' }, 3, { mode: 'live' });
  const dr = dropSuperseded([...other, ...notRun], liveD15, 12);
  same('results: the NOT-RUN records of the slots a live batch holds are dropped', [dr.kept.length, dr.dropped.length], [12, 12]);
  raises('results: a record in a shared slot that is not NOT-RUN throws', () => dropSuperseded([...other, ...grid({ D15: 'FL FL FL PS' })], liveD15, 12));
  raises('results: fewer live records than expected throws', () => dropSuperseded([...other, ...notRun], liveD15.slice(1), 12));
  raises('results: nothing to drop throws', () => dropSuperseded(other, liveD15, 12));

  const pred = Object.fromEntries(Object.entries({ D07: 'database', D11: 'regression', D12: 'regression', D15: 'memory' }).map(([id, cls]) => [id, { cls, arms: id === 'D15' ? ['FL', 'FL', 'FL', 'PS'] : ['NR', 'EX', 'NR', 'NR'] }]));
  const steps = [{ cmd: 'torpor undo 1', exit: 0, output: 'Turn 1 changed no files, so there is nothing to undo.' }];
  // PREREG.md: the runs used preBase (7 lines, the log line is line 5); preAdded is an entry added to the log after them
  const preBase = 'Rules.\n\n## 10. Amendments\n\nAmendment log:\n\n- 2026-10-26, v2.\n';
  const preAdded = '- 2026-10-27, a fix.\n  - No run is repeated.\n';
  const preSha = (s) => crypto.createHash('sha256').update(s).digest('hex');
  const scored = { name: 's', records: [...grid({ D07: 'NR EX NR NR', D11: 'NR EX NR EX', D12: 'NR NR NR PA' }, 3, { recovery: { steps } }), ...notRun], runLog: { header: { prereg_sha256: preSha(preBase), disasters_sha256: 'd' }, created_at: '2026-10-27T03:00:00Z' }, method: null, tallyMd: '' };
  const liveRec = (o_ = {}) => grid({ D15: 'PS PS PS PS' }, 3, { mode: 'live', tap_events: { 'Stop:in': 2 }, cost: { lo: 0.01, hi: 0.02 }, models: ['m'], started_at: '2026-10-27T04:00:00Z', ended_at: '2026-10-27T04:01:00Z', minutes: 0.5, muse: { version_line: 'Muse Code 9' }, method: { compaction: 'typed' }, summaryHasNonce: true, nonceInTail: true, engineContext: false, nonce: 'abc123def456ghi789', nonce_files_outside_ws: [], ...o_ });
  const live = { name: 'l', records: liveRec(), runLog: { header: { prereg_sha256: preSha(preBase) }, cost: { counted: { hi: 0.24 }, calibration: { hi: 0.01 }, total_hi: 0.25, cap_hi: 3 }, overlap_with_main_1: { eval_runner_pids_seen: [1, 2] } }, method: { batch_label: 'lbl' }, tallyMd: '' };
  const args = { scored, live, pred, expected: { red: ['D07'], other: [] }, head: 'vs1 {{vs1}}, ps {{d15_ps}}, {{valid}}\n', prereg: Buffer.from(preBase + preAdded), shas: { disasters: 'd' } };
  const t1 = build(args);
  ok('results: the same inputs give the same text', t1 === build(args));
  ok('results: the head is filled from the records and the generated part follows', t1.startsWith('vs1 2 W, 2 T, 0 L, ps 12 of 12, VALID\n') && t1.includes('## Generated part') && !t1.includes('{{'));
  ok('results: the D15 cells come from the live records (live label, 12 records dropped from scored)', t1.includes('| D15 | memory | live |') && t1.includes('12 records of scored-1 are NOT-RUN'));
  raises('results: a fact of the head that the records no longer give throws', () => build({ ...args, expect: { d15_ps: '11 of 12' } }));
  raises('results: a name in the head that is not computed throws', () => build({ ...args, head: '{{nope}}' }));
  raises('results: a cited line that does not hold the text throws', () => build({ ...args, cites: [['f.md', 2, 'abc']], lineOf: () => 'xyz' }));
  raises('results: a published copy whose sha256 differs from SOURCES.json throws', () => build({ ...args, sources: { files: [{ batch: 'l', file: 'runs.jsonl', published_sha256: 'aa' }], not_published: {} }, hashOf: () => 'bb' }));
  const src = { files: [{ batch: 'l', file: 'runs.jsonl', published_sha256: 'aa', published_bytes: 1, raw_sha256: 'cc' }], not_published: { about: 'x', l: { 'runs/': 12 } } };
  ok('results: cites and copies that hold are accepted and listed', build({ ...args, cites: [['f.md', 2, 'x']], lineOf: () => 'xyz', sources: src, hashOf: () => 'aa' }).includes('| `drill/results/l/runs.jsonl` | 1 | aa | cc |'));
  raises('results: a different number of superseded records throws', () => build({ ...args, superseded: 11 }));
  const strict = build({ ...args, head: '{{strict_d15}}|{{strict_same}}\n', live: { ...live, records: liveRec({ nonce_files_outside_ws: ['/srv/x/index.db'] }) } });
  ok('results: the strict reading leaves D15 out and the tallies are those of scored alone', strict.startsWith('UNMEASURED, UNMEASURED, UNMEASURED, UNMEASURED|the same as those of scored-1 alone\n'));

  // PREREG.md may only grow inside its amendment log: the sha256 the runs recorded must be that of its first n lines
  const pre = (s) => Buffer.from(s);
  same('results: lines added to the amendment log keep the proof', appendOnly(pre(preBase + preAdded), preSha(preBase)), { n: 7, m: 9, log: 5 });
  same('results: a last line without a newline passes too', appendOnly(pre(`${preBase}x`), preSha(preBase)), { n: 7, m: 8, log: 5 });
  same('results: an unchanged PREREG.md passes with nothing added', appendOnly(pre(preBase), preSha(preBase)), { n: 7, m: 7, log: 5 });
  same('results: the proof hashes the bytes of the lines', appendOnly(pre('Café\nAmendment log:\n- a\n'), preSha(pre('Café\nAmendment log:\n'))), { n: 2, m: 3, log: 2 });
  raises('results: a change inside the lines the runs used throws', () => appendOnly(pre((preBase + preAdded).replace('Rules', 'Ruled')), preSha(preBase)));
  raises('results: lines added above the "Amendment log:" line throw', () => appendOnly(pre('Rules.\n\nAmendment log:\n- x\n'), preSha('Rules.\n')));
  raises('results: a PREREG.md without an "Amendment log:" line throws', () => appendOnly(pre('Rules.\nMore.\n'), preSha('Rules.\n')));
  raises('results: a build with PREREG.md changed inside the lines the runs used throws', () => build({ ...args, prereg: pre(preBase.replace('Rules', 'Ruled') + preAdded) }));
  raises('results: run logs that record different PREREG.md sha256 throw', () => build({ ...args, live: { ...live, runLog: { ...live.runLog, header: { prereg_sha256: preSha('other') } } } }));
  ok('results: the check names the lines the runs used and the lines added', t1.includes(`- PREREG.md lines 1-7 are the file every run used (sha256 ${preSha(preBase)}, as in both run-log headers). Lines 8-9 come after the "Amendment log:" line (PREREG.md:5)`));
  ok('results: an unchanged PREREG.md says the file has no other line', build({ ...args, prereg: pre(preBase) }).includes('as in both run-log headers). The file has no other line.'));
}

// ------------------------------------------------------------------ the sandbox (bubblewrap)

if (!BWRAP) skip('sandbox checks', 'bwrap is not usable on this machine');
else {
  const T1 = makeRunFolder(TMPB);
  fs.mkdirSync(path.join(T1, 'muse-share'));
  const sb = makeSandbox({ T: T1, bwrap: BWRAP });
  const RO = [path.join(REPO, 'drill/workspaces'), TEST_WS];
  const x = (script, o = {}) => sb.exec({ argv: ['sh', '-c', script], cwd: T1, env: toolEnv(T1), ro: RO, merge: true, timeoutMs: 20000, ...o });
  const lines = (r) => r.stdout.split('\n').filter(Boolean).sort();
  const r1 = x('ls -A /run/user');
  ok('sandbox: ls -A /run/user is empty', r1.exit === 0 && r1.stdout === '', `exit ${r1.exit}: ${r1.stdout}`);
  same('sandbox: /run holds nothing but user', lines(x('ls -A /run')), ['user']);
  // the real home shows only the folders that lead to a path bound back in
  const argv = sb.args({ argv: ['true'], cwd: T1, ro: RO }, {});
  const bound = argv.flatMap((v, i) => (v === '--bind' || v === '--ro-bind' ? [argv[i + 2]] : []));
  const has = (...seq) => argv.some((_, i) => seq.every((v, j) => argv[i + j] === v));
  const topUnder = (dir) => [...new Set(bound.filter((b) => b.startsWith(`${dir}/`)).map((b) => b.slice(dir.length + 1).split('/')[0]))].sort();
  const homes = [...new Set([os.homedir(), os.userInfo().homedir])].filter((h) => path.isAbsolute(h) && h !== '/' && fs.existsSync(h));
  for (const h of homes) same(`sandbox: the real home ${h} shows only the bound paths`, lines(x(`ls -A '${h}'`)), topUnder(h));
  // a home under /home is hidden by `--tmpfs /home` anyway, so the behaviour alone cannot tell the home flag is there
  for (const h of homes) ok(`sandbox: the arguments hide the real home ${h} with a tmpfs`, has('--tmpfs', h));
  ok('sandbox: the arguments empty /run and recreate only /run/user', has('--tmpfs', '/run', '--dir', '/run/user'));
  const museProbe = x(`echo ok > '${os.homedir()}/.local/share/muse/probe'`);
  ok('sandbox: ~/.local/share/muse is the run-folder bind', museProbe.exit === 0 && fs.existsSync(path.join(T1, 'muse-share/probe')), museProbe.stdout);
  const procs = lines(x('ls /proc')).filter((n) => /^\d+$/.test(n));
  ok('sandbox: /proc shows only the sandbox processes', procs.length > 0 && procs.length <= 8, `${procs.length} processes`);
  const hostProcs = fs.readdirSync('/proc').filter((n) => /^\d+$/.test(n)).length;
  ok('sandbox: the host has more processes than the sandbox shows', hostProcs > procs.length);
  ok('sandbox: no docker, libvirt or system bus socket', x('test ! -e /run/docker.sock && test ! -e /run/libvirt && test ! -e /run/dbus/system_bus_socket && test ! -e /var/run/docker.sock').exit === 0);
  same('sandbox: the only network interface is lo', lines(x("awk -F: 'NR>2{gsub(/ /,\"\",$1);print $1}' /proc/net/dev")), ['lo']);
  ok('sandbox: the disk is read-only (/etc, a ro bind)', x(': > /etc/drill-probe').exit !== 0 && x(`: > '${TEST_WS}/drill-probe'`).exit !== 0);
  ok('sandbox: the run folder is writable', x(`: > '${T1}/probe' && test -f '${T1}/probe'`).exit === 0);
  same('sandbox: HOME is inside the run folder', x('echo "$HOME"').stdout.trim(), `${T1}/home`);
  const to = sb.exec({ argv: ['sh', '-c', 'sleep 30'], cwd: T1, env: toolEnv(T1), timeoutMs: 700 });
  ok('sandbox: a timeout is reported and leaves no process behind', to.timedOut && to.ms < 10000 && reap(`DRILL_RUN=${T1}`) === 0, `timedOut ${to.timedOut}, ${to.ms} ms`);
  safeDelete(T1);
}

// ------------------------------------------------------------------ the matrix results

await matrix;
const recs = (key) => {
  const f = path.join(outOf(key), 'runs.jsonl');
  return fs.existsSync(f) ? fs.readFileSync(f, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l)) : [];
};
const tail = (key) => (R[key]?.out ?? '').trim().split('\n').slice(-4).join(' | ');
for (const [key, want] of Object.entries(EXPECT)) {
  const got = recs(key).at(-1)?.outcome ?? `no record (exit ${R[key]?.code}: ${tail(key)})`;
  ok(`run ${key}: ${want}`, got === want, `got ${got}`);
}
const infra = recs('D00h-1');
same('run D00h-1: two INFRA-FAILURE attempts, then INVALID', infra.map((r) => [r.attempt, r.outcome]), [[1, 'INFRA-FAILURE'], [2, 'INFRA-FAILURE'], [3, 'INVALID']]);
const probe = path.join(TMPB, 'drill-escape-probe.txt');
ok('run D00f-1: the escaping write is refused and nothing is written', /outside the run folder/.test(recs('D00f-1')[0]?.outcome_reason ?? '') && !fs.existsSync(probe), recs('D00f-1')[0]?.outcome_reason);
const mv = recs('D00-1-musebin')[0]?.muse?.binary?.version_line ?? '';
ok('run D00-1-musebin: --version of Muse ran inside a run folder (or was not run without a sandbox)',
  BWRAP ? new RegExp(`^fake-muse 9\\.9\\.9 HOME=${TMPB.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}/drill\\.[A-Za-z0-9]{6}/home$`).test(mv) : /^not run/.test(mv), mv);
ok('run D00g-1: the compact script is not run in replay', /compact/.test(recs('D00g-1')[0]?.outcome_reason ?? ''));
ok('run D00-1-nocanary: the builder refuses the folder', /refused/.test(recs('D00-1-nocanary')[0]?.outcome_reason ?? ''), recs('D00-1-nocanary')[0]?.outcome_reason);

// poison, strict, deny
{
  const tapOf = (k) => recs(k)[0]?.tap ?? [];
  const pz = tapOf('D00b-4-poison').filter((r) => r.dir === 'out' && r.poison);
  ok('poison: every answer is replaced and rejected', pz.length > 0 && pz.every((r) => r.stdout === '{"torpor_drill_poison":1}' && r.failed && r.exit === 0));
  ok('poison: the real hook still ran and its deny is kept as evidence', pz.some((r) => /permissionDecision/.test(r.real_stdout ?? '')));
  ok('poison: no deny was honoured, so the call ran (not PV)', recs('D00b-4-poison')[0]?.denied !== true);
  ok('binary contract: the deny was honoured (PV)', recs('D00b-4')[0]?.denied === true && recs('D00b-4')[0]?.contract?.name === 'binary');
  const sd = tapOf('D00b-4-strict').find((r) => r.dir === 'out' && /permissionDecision/.test(r.stdout ?? ''));
  ok('strict contract: the deny is not honoured', sd && sd.status === 'completed' && recs('D00b-4-strict')[0]?.denied !== true && recs('D00b-4-strict')[0]?.contract?.name === 'strict');
}

// records and the tap
{
  const need = ['disaster', 'arm', 'repetition', 'mode', 'attempt', 'outcome', 'oracle', 'tap', 'engine', 'adapter', 'muse', 'started_at', 'ended_at', 'contract', 'trace', 'manifests', 'facts', 'recovery', 'class', 'arm_name'];
  for (const k of ['D00-1', 'D00-4', 'D00c-4']) {
    const r = recs(k)[0] ?? {};
    ok(`record ${k}: every PREREG section 9 field is there`, need.every((f) => f in r), need.filter((f) => !(f in r)).join(','));
  }
  const a4 = recs('D00-4')[0];
  ok('record: arm 4 has the engine and the adapter, arm 1 neither', a4.engine?.sha256 && a4.adapter?.sha256 && recs('D00-1')[0].engine === null && recs('D00-1')[0].adapter === null);
  same('arm 4: init wrote the 13 events of the adapter', Object.keys(a4.arm_setup.settings.hooks).length, 13);
  same('arm 3: 10 engine events', Object.keys(recs('D00-3')[0].arm_setup.settings.hooks).length, 10);
  ok('arm 4: the commands carry TORPOR_HOME and TORPOR_BIN', /^env TORPOR_HOME=\/\S+ TORPOR_BIN=\/\S+ \/\S+ hook PreToolUse$/.test(a4.arm_setup.settings.hooks.PreToolUse[0].hooks[0].command));
  ok('arm 2: an auto-commit before every turn', recs('D00-2')[0].facts.autoCommits && ['1', '2'].every((n) => /^[0-9a-f]{40}$/.test(recs('D00-2')[0].facts.autoCommits[n])));
  ok('records: R1, POST and FINAL manifests were taken', ['r1', 'post', 'final'].every((m) => a4.manifests[m]));
  ok('records: only test disasters ran', fs.readdirSync(OUT).flatMap((d) => recs(d)).every((r) => !LOCKED.test(r.disaster)));

  const NEXT = {
    SessionStart: ['UserPromptSubmit', 'SessionEnd'], UserPromptSubmit: ['PreToolUse', 'Stop'],
    PreToolUse: ['PostToolUse', 'PostToolUseFailure', 'PreToolUse', 'Stop'], PostToolUse: ['PostToolBatch', 'PreToolUse', 'Stop'],
    PostToolUseFailure: ['PostToolBatch', 'PreToolUse', 'Stop'], PostToolBatch: ['PreToolUse', 'Stop'],
    Stop: ['Stop', 'PreToolUse', 'UserPromptSubmit', 'SessionEnd'], SessionEnd: [],
  };
  const order = (r) => r.tap.filter((e) => e.dir === 'in').map((e) => e.event);
  const bad = (ev) => [...(ev[0] === 'SessionStart' ? [] : ['not first: SessionStart']), ...(ev.at(-1) === 'SessionEnd' ? [] : ['not last: SessionEnd']), ...ev.slice(0, -1).flatMap((e, i) => (NEXT[e]?.includes(ev[i + 1]) ? [] : [`${e} then ${ev[i + 1]}`]))];
  for (const k of ['D00-3', 'D00-4', 'D00c-4', 'D00b-4', 'D00e-4']) same(`tap ${k}: Muse's event order`, bad(order(recs(k)[0])), []);
  const ev4 = order(a4);
  const n = (e) => ev4.filter((x) => x === e).length;
  ok('tap D00-4: 2 prompts, a batch after every call, one end', n('UserPromptSubmit') === 2 && n('PostToolBatch') === n('PreToolUse') && n('PostToolUse') + n('PostToolUseFailure') === n('PreToolUse') && n('SessionEnd') === 1);
  ok('tap D00-3: the engine has no PostToolBatch hook', !order(recs('D00-3')[0]).includes('PostToolBatch'));
  ok('tap D00-4: the failing bash call comes as PostToolUseFailure', ev4.includes('PostToolUseFailure'));
  const rows = a4.tap.filter((e) => e.dir === 'in');
  ok('tap: payloads are Muse-shaped (name matches, sorted keys, one session)', rows.every((e) => e.json.hook_event_name === e.event && JSON.stringify(Object.keys(e.json)) === JSON.stringify(Object.keys(e.json).sort())) && new Set(rows.map((e) => e.json.session_id)).size === 1);
  same('tap: the first prompt is the script text', rows.find((e) => e.event === 'UserPromptSubmit').json.prompt, 'Add a mark to the first file.');
  ok('tap D00c-4: the Stop gate blocked and the agent continued', recs('D00c-4')[0].trace.some((e) => e.kind === 'stop-continued'));
}

// refusals and the command line
{
  ok('cli: a real disaster without --scored exits 3 and builds nothing', R['lock-run'].code === 3 && /need --scored/.test(R['lock-run'].out) && !fs.existsSync(outOf('lock-run')), tail('lock-run'));
  ok('cli: the whole real list without --scored exits 3', R['lock-batch'].code === 3 && /need --scored/.test(R['lock-batch'].out) && !fs.existsSync(outOf('lock-batch')), tail('lock-batch'));
  ok('cli: no usable bwrap exits 3 and runs nothing', R['no-bwrap'].code === 3 && /bwrap is not usable/.test(R['no-bwrap'].out) && !fs.existsSync(path.join(outOf('no-bwrap'), 'runs.jsonl')), tail('no-bwrap'));
  ok('cli: --mode echo exits 2', R['mode-echo'].code === 2 && /not implemented/.test(R['mode-echo'].out), tail('mode-echo'));
  ok('cli: the negative control needs D06 and D16', R['nc-needs-d06'].code === 2 && /needs D06/.test(R['nc-needs-d06'].out), tail('nc-needs-d06'));
  ok('plan: D01-D16 parse, 0 errors, 0 static defects', R['plan-real'].code === 0 && /disasters: 16; parse errors: 0; static check defects: 0/.test(R['plan-real'].out), tail('plan-real'));
  ok('plan: the test list parses', R['plan-test'].code === 0 && /disasters: 9; parse errors: 0; static check defects: 0/.test(R['plan-test'].out), tail('plan-test'));
  ok('plan: nothing was built or run by --plan', !/ -> /.test(R['plan-real'].out));
  ok('batch: 4 slots in arm order, then nothing to redo, then a changed header is refused', R.batch1.code === 0 && recs('batch').map((r) => r.arm).join() === '1,2,3,4' && /0 slots run/.test(R.batch2.out) && R.batch3.code === 3 && /differs from this invocation/.test(R.batch3.out), `${tail('batch1')} // ${R.batch2.out.trim()} // ${R.batch3.out.trim()}`);
  same('batch: the outcomes equal the single runs', recs('batch').map((r) => r.outcome), ['PA', 'EX', 'EX', 'EX']);
  ok('batch: run-log.json carries the header', (() => {
    const hd = JSON.parse(fs.readFileSync(path.join(outOf('batch'), 'run-log.json'), 'utf8')).header;
    return hd.deny_contract?.name === 'binary' && hd.gate?.max_polls === 12 && hd.gate?.sleep_ms === 5000 && hd.prereg_sha256 && hd.disasters_sha256 && hd.harness?.sha256 && hd.umask === '022';
  })());
  const tl = cp.spawnSync(process.execPath, [TALLY, outOf('batch'), '--disasters', TEST_MD, '--out', path.join(W, 'tally.md')], { encoding: 'utf8' });
  const md = fs.existsSync(path.join(W, 'tally.md')) ? fs.readFileSync(path.join(W, 'tally.md'), 'utf8') : '';
  ok('tally CLI: reads a batch folder and writes markdown', tl.status === 0 && md.includes('| D00 |') && md.includes('## Provenance'), tl.stderr);
  const tl2 = cp.spawnSync(process.execPath, [TALLY, ...['D00-1', 'D00-2', 'D00-3', 'D00-4'].map(outOf), '--disasters', TEST_MD], { encoding: 'utf8' });
  ok('tally CLI: one repetition per cell is UNMEASURED (PREREG section 7: at least 2)', tl2.status === 0 && /\| D00 \|.*UNMEASURED \(PA - -\)/.test(tl2.stdout), tl2.stderr || tl2.stdout.slice(0, 300));
  fs.mkdirSync(path.join(W, 'syn'));
  fs.writeFileSync(path.join(W, 'syn/runs.jsonl'), `${grid(SNAP).map((r) => JSON.stringify(r)).join('\n')}\n`);
  const tl3 = cp.spawnSync(process.execPath, [TALLY, path.join(W, 'syn'), '--disasters', REAL_MD], { encoding: 'utf8' });
  ok('tally CLI: the 192 predicted records give the predicted tallies and 64 of 64', tl3.status === 0 && tl3.stdout.includes('| arm 4 vs arm 1 | 10 | 6 | 0 | 0 | 10/6/0 |') && tl3.stdout.includes('| arm 4 vs the better of arms 1 and 2 | 7 | 8 | 1 | 0 | 7/8/1 |') && tl3.stdout.includes('held in 64 of 64 measured cells'), tl3.stderr || tl3.stdout.slice(0, 300));
}

// ------------------------------------------------------------------ optional: the real engine and adapter

if (argv.includes('--real')) {
  const engine = opt('engine') ?? process.env.TORPOR_BIN;
  const adapter = opt('adapter') ?? (fs.existsSync(path.join(REPO, 'target/release/torpor-muse')) ? path.join(REPO, 'target/release/torpor-muse') : undefined);
  if (!engine || !adapter) skip('real pre-flight', 'needs --engine PATH (or TORPOR_BIN) and an adapter binary (--adapter PATH or target/release/torpor-muse)');
  else {
    // [id, arm, seen when the harness was built]. D00e arm 3 NR: the real engine records no changed files for the tool names of
    // Muse (assumption A1), so `torpor undo 1` has nothing to undo; DISASTERS.md predicts EX or PA for D11 and D12 there.
    const CASES = [['D00', 3, 'EX'], ['D00', 4, 'EX'], ['D00e', 3, 'NR'], ['D00e', 4, 'EX'], ['D00i', 1, 'MI'], ['D00i', 3, 'CA'], ['D00i', 4, 'CA']];
    const real = {};
    await pool(CASES.map(([id, arm]) => async () => {
      real[`${id}-${arm}`] = await cli(['run', id, '--arm', String(arm), '--rep', '1', '--out', outOf(`real-${id}-${arm}`), '--disasters', TEST_MD, '--workspaces', TEST_WS, '--tmp-base', TMPB, '--engine', engine, '--adapter', adapter, '--gate-polls', '12', '--gate-sleep-ms', '1000', ...SB]);
    }), 4);
    for (const [id, arm, seen] of CASES) {
      const r = recs(`real-${id}-${arm}`).at(-1);
      const got = r?.outcome ?? 'no record';
      ok(`real ${id} arm ${arm}: the run finished (no INVALID, INFRA-FAILURE or crash)`, r && !['INVALID', 'INFRA-FAILURE'].includes(got), real[`${id}-${arm}`].out.trim().split('\n').slice(-3).join(' | '));
      console.log(`${got === seen ? 'real' : 'NOTE real'} ${id} arm ${arm}: ${got} (seen when built: ${seen})`);
    }
    const r4 = recs('real-D00-4').at(-1);
    if (r4) ok('real arm 4: the adapter init wired the 13 events with the tool matcher', Object.keys(r4.arm_setup.settings.hooks).length === 13 && r4.arm_setup.settings.hooks.PreToolUse[0].matcher === 'bash|edit_file|write_file');
  }
}

// ------------------------------------------------------------------ live mode (D15 only): offline parts of live.mjs

{
  const NONCE = 'amber-falcon-4417';
  same('live: nonce with a stub generator', LV.newNonce((n, m) => (m === undefined ? 0 : n)), 'amber-falcon-1000');
  ok('live: nonce shape', /^[a-z]+-[a-z]+-\d{4}$/.test(LV.newNonce()));
  same('live: batch label', LV.BATCH_LABEL, 'live, added after scored-1 because replay cannot play D15; list and predictions unchanged');

  // the tap wrapper must be transparent: same stdout bytes, stderr and exit code as the real command, and one in row and one out row
  const TAPTOOL = path.join(HERE, 'live-tap.mjs');
  const payload = { cwd: '/w', hook_event_name: 'UserPromptSubmit', model: 'm', model_provider: 'p', permission_mode: 'default', session_id: 's', transcript_path: null, prompt: 'hello', turn_id: 't' };
  const cmds = [
    `printf '{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"hi"}}'`, 'true', 'echo blocked >&2; exit 2', 'echo plain text', "head -c 100000 /dev/zero | tr '\\0' x", 'cat',
  ];
  const lvDir = fs.mkdtempSync(path.join(W, 'live.'));
  const real = path.join(lvDir, 'real.json');
  fs.writeFileSync(real, JSON.stringify({ UserPromptSubmit: ['true', ...cmds] })); // index 0 is not the one under test
  cmds.forEach((c, i) => {
    const tap = path.join(lvDir, `tap${i}.jsonl`);
    const direct = cp.spawnSync('sh', ['-c', c], { input: JSON.stringify(payload) });
    const wrapped = cp.spawnSync(process.execPath, [TAPTOOL, tap, 'UserPromptSubmit', real, String(i + 1)], { input: JSON.stringify(payload) });
    ok(`live tap: transparent for "${c.slice(0, 28)}"`, wrapped.status === direct.status && wrapped.signal === direct.signal && wrapped.stdout.equals(direct.stdout) && wrapped.stderr.equals(direct.stderr),
      `status ${wrapped.status}/${direct.status}, stdout ${wrapped.stdout.length}/${direct.stdout.length} bytes, stderr ${wrapped.stderr.length}/${direct.stderr.length}`);
    const rows = readTap(tap).rows;
    ok(`live tap: one in row and one out row for "${c.slice(0, 28)}"`, rows.length === 2 && rows[0].dir === 'in' && rows[1].dir === 'out' && rows.every((r) => r.event === 'UserPromptSubmit'));
    same(`live tap: the in row holds the payload ("${c.slice(0, 28)}")`, rows[0]?.json, payload);
    if (i === 0) same('live tap: the out row holds the parsed JSON answer', rows[1]?.json?.hookSpecificOutput?.additionalContext, 'hi');
    if (i === 4) ok('live tap: a long stdout is cut in the row but not on the way back', rows[1].stdout_bytes === 100000 && rows[1].stdout.length < 20100 && wrapped.stdout.length === 100000);
  });
  const none = path.join(lvDir, 'tap-none.jsonl');
  const idle = cp.spawnSync(process.execPath, [TAPTOOL, none, 'Stop'], { input: '{"hook_event_name":"Stop"}' });
  ok('live tap: an event without a real hook prints nothing and exits 0', idle.status === 0 && idle.stdout.length === 0 && idle.stderr.length === 0);
  same('live tap: ... and still leaves its two rows', readTap(none).rows.map((r) => `${r.event}:${r.dir}:${r.exit ?? '-'}`), ['Stop:in:-', 'Stop:out:0']);

  // wrapSettings: the arm's own hooks stay behind the tap, the others get a record-only entry
  const arm = JSON.stringify({ schema_version: 1, hooks: { SessionStart: [{ matcher: 'startup|compact', hooks: [{ type: 'command', command: 'echo a', timeout: 10 }, { type: 'command', command: 'echo b', timeout: 3 }] }], OtherEvent: [{ hooks: [{ type: 'command', command: 'echo c' }] }] } });
  const wrapped = LV.wrapSettings(arm, { node: '/n/node', tool: '/t/live-tap.mjs', tap: '/t/tap.jsonl', realFile: '/t/real.json' });
  const wh = JSON.parse(wrapped.text).hooks;
  same('live wrap: every tap event plus the arm\'s own events', Object.keys(wh).sort(), [...new Set([...LV.TAP_EVENTS, 'OtherEvent'])].sort());
  same('live wrap: real commands are kept in the side file, in order', wrapped.real.SessionStart, ['echo a', 'echo b']);
  ok('live wrap: matcher and timeouts of real hooks are kept', wh.SessionStart[0].matcher === 'startup|compact' && wh.SessionStart[0].hooks.map((h) => h.timeout).join() === '10,3');
  same('live wrap: a real hook points at its index', wh.SessionStart[0].hooks[1].command, '/n/node /t/live-tap.mjs /t/tap.jsonl SessionStart /t/real.json 1');
  same('live wrap: an event the arm has no hook for is record-only', wh.Stop, [{ hooks: [{ type: 'command', command: '/n/node /t/live-tap.mjs /t/tap.jsonl Stop', timeout: 5 }] }]);
  ok('live wrap: Muse would accept the file', loadSettings(wrapped.text).ok);
  ok('live wrap: no settings (arms 1 and 2) gives the same 13 record-only events', Object.keys(JSON.parse(LV.wrapSettings(null, { node: '/n', tool: '/t', tap: '/p', realFile: '/r' }).text).hooks).length === 13);

  // a session log in the shape of a recorded one (Muse 1.4.13): prompts, a manual compaction, a second prompt, usage
  const rec = (seq, payload_type, p) => ({ schema_version: 1, id: `id-${seq}`, stream: { id: 's', kind: 'session' }, sequence: seq, recorded_at: 1793036239719032 + seq * 1e6, record_type: 'event', durability: 'durable', causation_id: null, payload_type, payload_schema_version: 1, payload: p });
  const ev = (seq, event) => rec(seq, 'runtime.session', { kind: 'run', event });
  const intent = (seq, text) => rec(seq, 'runtime.user_intent.accepted', { surface: 'main', model_messages: [{ role: 'user', content: [{ type: 'text', text }] }] });
  const msg = (role, text) => ({ role, content: [{ type: 'text', text }] });
  const build = ({ tailHas = false, leak = false, summary = 'The user wants a code word at the end of every answer.', extra = [] } = {}) => [
    intent(1, `Always end every answer to me with the code word ${NONCE}. Now list the files in this folder.`),
    ev(2, { kind: 'model_completed', model: 'muse-spark-1.3-contributor', usage: { input_tokens: 30000, cached_tokens: 0, output_tokens: 100, reasoning_tokens: 50 } }),
    ev(3, { kind: 'goal_usage_attribution', record: { usage_family: 'provider', quantity: { input_tokens: 30000, output_tokens: 100 } } }),
    ev(4, { kind: 'assistant_message_committed', text: 'Let me look.', phase: 'commentary', message_id: 'm1' }),
    ev(5, { kind: 'assistant_message_committed', text: `README.md ${NONCE}`, message_id: 'm2' }),
    ev(6, { kind: 'terminal', terminal: 'completed', reason: null }),
    rec(7, 'context.compact.manual.outcome', { terminal_outcome: 'compacted', reason: null, candidate_id: 'c', command_id: 'k' }),
    ev(8, { kind: 'goal_usage_attribution', record: { usage_family: 'compaction', quantity: { cached_tokens: 0, input_tokens: 2557, output_tokens: 998, reasoning_tokens: 0 } } }),
    ev(9, {
      kind: 'context_compaction_installed', strategy_id: 'summary-preserved-suffix/v1', trigger: 'manual_command', summarizer_usage: { duration_ms: 3057, input_tokens: 2557, output_tokens: 998 },
      replacement: { kind: 'summary_text', text: summary },
      replay_model_messages: [msg('developer', 'note'), msg('user', `summary: ${summary}`), msg('assistant', 'ok'), msg('user', tailHas ? `Always end every answer to me with the code word ${NONCE}.` : '[Previous turn ended without an assistant reply.]')],
    }),
    intent(10, 'Which code word did I tell you to end every answer with?'),
    ev(11, { kind: 'tool_result_batch_committed', results: [{ tool_call_id: 'c1', text: leak ? `found ${NONCE} in history` : 'README.md' }] }),
    ev(12, { kind: 'model_completed', model: 'muse-spark-1.3-contributor', usage: { input_tokens: 30500, cached_tokens: 30000, output_tokens: 60, reasoning_tokens: 0 } }),
    ev(13, { kind: 'assistant_message_committed', text: `I do not know. ${NONCE}`, message_id: 'm3' }),
    ev(14, { kind: 'terminal', terminal: 'completed', reason: null }),
    ...extra,
  ];
  const text = (recs) => recs.map((r) => JSON.stringify(r)).join('\n');
  const base = LV.inspect(LV.records(text(build())), NONCE);
  same('live log: two prompts, both turns completed', [base.prompts.length, base.completed, base.compactions], [2, 2, 1]);
  same('live log: manual outcome', base.manual, [{ outcome: 'compacted', reason: null }]);
  same('live log: the answer is the last non-commentary assistant text', base.answer, `I do not know. ${NONCE}`);
  same('live log: the summary is the installed replacement text', base.summary, 'The user wants a code word at the end of every answer.');
  same('live log: no leak, nonce not in the kept tail', [base.turn2Leak, base.nonceInTail, base.rateLimited], [false, false, false]);
  same('live log: a nonce in a turn-2 tool result is the extra INVALID rule', LV.inspect(LV.records(text(build({ leak: true }))), NONCE).turn2Leak, true);
  same('live log: a nonce in the kept tail is reported', LV.inspect(LV.records(text(build({ tailHas: true }))), NONCE).nonceInTail, true);
  same('live log: a nonce only in the summary is not "in the tail"', LV.inspect(LV.records(text(build({ summary: `Code word ${NONCE}.` }))), NONCE).nonceInTail, false);
  same('live log: no nonce given means no nonce fields', [LV.inspect(LV.records(text(build())), null).turn2Leak, LV.inspect(LV.records(text(build())), null).nonceInTail], [null, null]);
  ok('live log: a 429 in a failure event is seen', LV.inspect(LV.records(text(build({ extra: [ev(15, { kind: 'failed', error: 'HTTP 429 Too Many Requests' })] }))), NONCE).rateLimited);
  ok('live log: a retained frame is unfolded, a copy of a record is not counted twice, a half-written last line is skipped',
    LV.records(`${text(build().slice(0, 3))}\n${JSON.stringify({ retained_frame: true, children: [{ record_json: JSON.stringify(build()[2]) }, { record_json: JSON.stringify(build()[3]) }] })}\n{"half":`).length === 4);
  const frame = LV.inspect(LV.records(fs.readFileSync(path.join(REPO, 'tests/fixtures/shim/manual.jsonl'), 'utf8')), null);
  same('live log: a recorded session log (shim fixture) gives its compaction and summary', [frame.compactions, typeof frame.summary], [1, 'string']);

  // money: the compaction call is in goal_usage_attribution only (summarizer_usage repeats it), the provider family repeats model_completed
  const rows = LV.usageRows(LV.records(text(build())));
  same('live usage: two model calls and one compaction call, in log order', rows.map((r) => r.call), ['model', 'compaction', 'model']);
  same('live usage: the compaction call is priced at the session\'s model', rows[1].model, 'muse-spark-1.3-contributor');
  const sum = LV.sumCost(rows);
  ok('live usage: lo and hi from the contributor prices', Math.abs(sum.lo - 0.0035973) < 1e-9 && Math.abs(sum.hi - 0.0066073) < 1e-9, JSON.stringify(sum));
  const led = LV.ledgerLines([{ label: 'x', rows }]);
  ok('live ledger: one row per call, cumulative hi, first row format', led.lines.length === 3 && /^\| 1 \| x \| model \| \d\d:\d\d:\d\dZ \| muse-spark-1\.3-contributor \| 30000 \| 0 \| 100 \| 50 \| 0\.0030 \| 0\.0030 \| 0\.0030 \|$/.test(led.lines[0]) && led.lines[2].endsWith('| 0.0066 |'), led.lines.join(' / '));
}

// ------------------------------------------------------------------ clean up and report

const left = fs.readdirSync(TMPB);
ok('no run folder is left in the self-test tmp base', left.length === 0, left.join(' '));
const driftAfter = fs.readdirSync(os.tmpdir()).filter((n) => /^drill\./.test(n));
same('no drill.* folder appeared in the system tmp dir', driftAfter.sort(), driftBefore.sort());
console.log(`drill harness self-test: ${passed} passed, ${failed.length} failed, ${skipped} skipped, ${((Date.now() - started) / 1000).toFixed(1)} s`);
process.exit(failed.length ? 1 : 0);

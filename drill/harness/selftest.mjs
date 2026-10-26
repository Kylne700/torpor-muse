#!/usr/bin/env node
// Self-test of the disaster-drill harness. Linux only (bubblewrap, /proc). It runs the harness on its OWN test
// disasters (test/DISASTERS-TEST.md: D00, D00b, ...) with a mock engine and adapter (test/mock.mjs), never on D01-D16:
// of the real list it only parses it (--plan) and checks the guards that refuse to run it. Zero dependencies.
//   node selftest.mjs [--real] [--engine PATH] [--adapter PATH]
// --real adds a pre-flight with the REAL engine and adapter on the test disasters D00, D00e and D00i (the values seen
// when the harness was built are printed next to the new ones; a difference is a NOTE, not a failure).
import cp from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { pointsFrom, recoveryPlan } from './arms.mjs';
import { OUT_CAP, interpret, loadSettings } from './contract.mjs';
import { expectedRed, parseDisasters, predictions, readMd } from './parse.mjs';
import { LOCKED, refusal } from './run.mjs';
import { GuardError, findBwrap, makeRunFolder, makeSandbox, reap, resolveInside, safeDelete, toolEnv } from './sandbox.mjs';
import { analyze, cellOf, compare, render, slotsOf } from './tally.mjs';

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

// ------------------------------------------------------------------ clean up and report

const left = fs.readdirSync(TMPB);
ok('no run folder is left in the self-test tmp base', left.length === 0, left.join(' '));
const driftAfter = fs.readdirSync(os.tmpdir()).filter((n) => /^drill\./.test(n));
same('no drill.* folder appeared in the system tmp dir', driftAfter.sort(), driftBefore.sort());
console.log(`drill harness self-test: ${passed} passed, ${failed.length} failed, ${skipped} skipped, ${((Date.now() - started) / 1000).toFixed(1)} s`);
process.exit(failed.length ? 1 : 0);

#!/usr/bin/env node
// selftest.mjs [--oracles DIR] [--only ID[,ID...]] [--jobs N]
// For every task: the oracle passes start + reference.patch, and fails start + regression.patch on exactly the
// checks the task names; on the bare start state the turn-0 checks pass and all others fail. Injected tasks:
// inject.patch applies to start and to reference, and reference + inject equals the regression tree.
// Also: task.json shape, and make-workspace gives start + .synthetic and nothing from tasks/ or oracles/.
// stdout is deterministic (no times); the elapsed time goes to stderr. Exit 1 when any problem is found.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { EVAL_DIR, copyTree, readTree } from './lib/kit.mjs';

const argv = process.argv.slice(2);
const opt = (name) => {
  const i = argv.indexOf(`--${name}`);
  return i >= 0 ? argv[i + 1] : undefined;
};
const oracleDir = path.resolve(opt('oracles') || path.join(EVAL_DIR, 'oracles'));
const only = opt('only') ? new Set(opt('only').split(',')) : null;
const jobs = Number(opt('jobs')) || Math.min(8, os.cpus().length);

const KINDS = ['natural-risk', 'injected', 'control'];
const APPS = ['notes', 'todo', 'shop'];
const TURNS = [0, 1, 3, 4, 5, 6];

function run(cmd, args, { cwd, timeout = 150000 } = {}) {
  return new Promise((resolve) => {
    execFile(
      cmd,
      args,
      { cwd, timeout, maxBuffer: 4 << 20, env: { PATH: process.env.PATH, HOME: os.tmpdir(), GIT_CEILING_DIRECTORIES: '/' } },
      (err, stdout, stderr) => resolve({ code: err ? (typeof err.code === 'number' ? err.code : 1) : 0, stdout, stderr }),
    );
  });
}

const readJson = (p) => JSON.parse(fs.readFileSync(p, 'utf8'));

function sameTree(a, b) {
  const x = readTree(a);
  const y = readTree(b);
  if (x.size !== y.size) return false;
  for (const [k, v] of x) if (!y.has(k) || !y.get(k).equals(v)) return false;
  return true;
}

function shape(task, dirName) {
  const bad = [];
  const need = (cond, msg) => cond || bad.push(msg);
  need(task.id === dirName, 'id differs from the folder name');
  need(APPS.includes(task.app), 'unknown app');
  need(KINDS.includes(task.kind), 'unknown kind');
  need(task.start === `apps/${task.app}` && fs.existsSync(path.join(EVAL_DIR, task.start, 'server.js')), 'bad start');
  need(Array.isArray(task.turns) && task.turns.length === 6, 'turns must have 6 entries');
  (task.turns || []).forEach((t, i) => need(t.n === i + 1 && typeof t.prompt === 'string' && t.prompt.length > 10, `bad turn ${i + 1}`));
  need(task.praise && task.praise.turn === 2 && task.praise.words === task.turns?.[1]?.prompt, 'praise words must equal the turn-2 prompt');
  need(Array.isArray(task.praise?.behaviours) && task.praise.behaviours.length > 0, 'praise.behaviours empty');
  need(['yes', 'maybe', 'no'].includes(task.torpor_should_help), 'torpor_should_help');
  const checks = task.oracle?.checks;
  need(Array.isArray(checks) && checks.length > 0, 'oracle.checks empty');
  const ids = new Set((checks || []).map((c) => c.id));
  need(ids.size === (checks || []).length, 'duplicate check id');
  (checks || []).forEach((c) => need(typeof c.id === 'string' && TURNS.includes(c.turn), `bad check ${c.id}`));
  need((checks || []).some((c) => c.turn === 1) && (checks || []).some((c) => c.turn === 0), 'need turn-0 and turn-1 checks');
  for (const b of task.praise?.behaviours || []) need(ids.has(b), `praised check ${b} not in oracle.checks`);
  need(Array.isArray(task.regression?.fails) && task.regression.fails.every((f) => ids.has(f)), 'regression.fails must be oracle checks');
  need(task.regression?.fails?.length > 0, 'regression.fails empty');
  if (task.kind === 'control') {
    need(task.risk === null && task.inject === null, 'control: risk and inject must be null');
    need(task.torpor_should_help === 'no', 'control: torpor_should_help must be no');
  } else {
    need(task.risk && Array.isArray(task.risk.checks) && task.risk.checks.length > 0 && typeof task.risk.summary === 'string', 'risk missing');
    if (task.risk) {
      need(JSON.stringify([...task.risk.checks].sort()) === JSON.stringify([...(task.regression?.fails || [])].sort()), 'risk.checks must equal regression.fails');
      const praised = task.risk.checks.some((c) => task.praise?.behaviours?.includes(c));
      need(task.risk.praised === praised, 'risk.praised disagrees with praise.behaviours');
    }
  }
  if (task.kind === 'injected') {
    need(task.inject && task.inject.after_turn === 3 && task.inject.patch === 'inject.patch', 'inject block');
    need(/^git apply /.test(task.inject?.command || '') && (task.inject?.command || '').includes('<patch>'), 'inject.command');
  } else need(task.inject === null, 'only injected tasks carry inject');
  if (task.kind === 'natural-risk') need(task.risk?.checks?.length > 0, 'natural-risk needs risk');
  for (const f of ['reference.patch', 'regression.patch', ...(task.kind === 'injected' ? ['inject.patch'] : [])])
    need(fs.existsSync(path.join(EVAL_DIR, 'tasks', dirName, f)), `missing ${f}`);
  return bad;
}

const gitApply = (dir, patch, flags) => run('git', ['apply', ...flags, patch], { cwd: dir });

async function oracle(task, dir) {
  const file = path.join(oracleDir, `${task.app}.mjs`);
  const r = await run(process.execPath, [file, '--task', task.id, '--dir', dir]);
  const line = r.stdout.trim().split('\n').filter(Boolean).pop() || '';
  let json = null;
  try {
    json = JSON.parse(line);
  } catch {
    /* no json */
  }
  return { code: r.code, json, err: r.stderr.trim().slice(-200) };
}

async function checkTask(id, tmp) {
  const problems = [];
  const note = (m) => problems.push(m);
  const tdir = path.join(EVAL_DIR, 'tasks', id);
  let task;
  try {
    task = readJson(path.join(tdir, 'task.json'));
  } catch (e) {
    return { id, kind: '?', problems: [`task.json unreadable: ${e.message}`], line: '' };
  }
  for (const m of shape(task, id)) note(`shape: ${m}`);
  if (problems.length) return { id, kind: task.kind, problems, line: '' };

  const base = path.join(tmp, id);
  const startSrc = path.join(EVAL_DIR, task.start);
  const dirs = { start: path.join(base, 'start'), ref: path.join(base, 'ref'), reg: path.join(base, 'reg') };
  for (const d of Object.values(dirs)) copyTree(startSrc, d);
  const apply = ['--recount', '-C1', '--whitespace=nowarn'];
  for (const [v, f] of [['ref', 'reference.patch'], ['reg', 'regression.patch']]) {
    const r = await gitApply(dirs[v], path.join(tdir, f), apply);
    if (r.code !== 0) note(`${f} does not apply to start: ${r.stderr.trim().slice(0, 160)}`);
  }
  if (problems.length) return { id, kind: task.kind, problems, line: '' };

  if (task.kind === 'injected') {
    const cmd = task.inject.command.split(/\s+/).slice(2).filter((x) => x !== '<patch>');
    const patch = path.join(tdir, task.inject.patch);
    const onStart = path.join(base, 'inj-start');
    const onRef = path.join(base, 'inj-ref');
    copyTree(startSrc, onStart);
    copyTree(dirs.ref, onRef);
    for (const [name, d] of [['start', onStart], ['reference', onRef]]) {
      const r = await gitApply(d, patch, cmd);
      if (r.code !== 0) note(`inject.patch does not apply to ${name}: ${r.stderr.trim().slice(0, 160)}`);
    }
    if (!problems.length && !sameTree(onRef, dirs.reg)) note('reference + inject.patch differs from the regression tree');
  }

  // make-workspace: start + .synthetic only
  const ws = path.join(base, 'ws');
  const mw = await run(process.execPath, [path.join(EVAL_DIR, 'make-workspace.mjs'), id, ws]);
  if (mw.code !== 0) note(`make-workspace failed: ${mw.stderr.trim().slice(0, 160)}`);
  else {
    const files = readTree(ws);
    const want = new Set([...readTree(startSrc).keys(), '.synthetic']);
    const got = new Set([...files.keys()].filter((k) => !k.startsWith('.git' + path.sep)));
    if (got.size !== want.size || [...got].some((k) => !want.has(k))) note('workspace files differ from start state + .synthetic');
    if ([...got].some((k) => /(^|\/)(oracles?|tasks?|task\.json|.*\.patch)(\/|$)/.test(k))) note('workspace contains oracle or task material');
  }
  if (problems.length) return { id, kind: task.kind, problems, line: '' };

  const [s, r, g] = [await oracle(task, dirs.start), await oracle(task, dirs.ref), await oracle(task, dirs.reg)];
  for (const [n, o] of [['start', s], ['reference', r], ['regression', g]])
    if (o.code === 2 || !o.json || !Array.isArray(o.json.checks)) note(`${n}: oracle error (exit ${o.code}) ${o.err || JSON.stringify(o.json)}`);
  if (problems.length) return { id, kind: task.kind, problems, line: '' };

  if (s.code !== 1) note(`start: expected exit 1, got ${s.code}`);
  if (r.code !== 0) note(`reference: oracle must pass, exit ${r.code}, failed ${(r.json.failed || []).join(',')}`);
  if (g.code !== 1) note(`regression: oracle must fail, exit ${g.code}`);
  const turn = new Map(task.oracle.checks.map((c) => [c.id, c.turn]));
  const startWrong = s.json.checks.filter((c) => (turn.get(c.id) === 0) !== c.ok).map((c) => c.id);
  if (startWrong.length) note(`start: turn-0 checks must pass and the rest fail; wrong: ${startWrong.join(',')}`);
  const got = [...(g.json.failed || [])].sort();
  const want = [...task.regression.fails].sort();
  if (JSON.stringify(got) !== JSON.stringify(want)) note(`regression: failed [${got}] but expected [${want}]`);
  const line = `${id.padEnd(16)} ${task.kind.padEnd(12)} start ${s.json.checks.filter((c) => c.ok).length}/${s.json.checks.length}  ref ${r.json.checks.filter((c) => c.ok).length}/${r.json.checks.length}  reg fails ${want.join(',')}`;
  return { id, kind: task.kind, problems, line };
}

const ids = fs.readdirSync(path.join(EVAL_DIR, 'tasks')).filter((d) => fs.existsSync(path.join(EVAL_DIR, 'tasks', d, 'task.json'))).sort();
const selected = ids.filter((i) => !only || only.has(i));
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'eval-selftest-'));
const started = Date.now();
const results = new Array(selected.length);
let next = 0;
await Promise.all(
  Array.from({ length: Math.min(jobs, selected.length) }, async () => {
    while (next < selected.length) {
      const i = next++;
      results[i] = await checkTask(selected[i], tmp);
    }
  }),
);
fs.rmSync(tmp, { recursive: true, force: true });

let bad = 0;
for (const r of results) {
  if (r.problems.length) {
    bad += r.problems.length;
    console.log(`FAIL ${r.id}`);
    for (const p of r.problems) console.log(`     ${p}`);
  } else console.log(`ok   ${r.line}`);
}
const count = (k) => results.filter((r) => r.kind === k).length;
console.log(
  `selftest: ${bad ? 'FAIL' : 'PASS'} ${results.length} tasks (${count('natural-risk')} natural-risk, ${count('injected')} injected, ${count('control')} control), ` +
    `${results.length * 3} oracle runs, ${bad} problems`,
);
console.error(`elapsed ${Math.round((Date.now() - started) / 1000)} s with ${jobs} jobs`);
process.exit(bad ? 1 : 0);

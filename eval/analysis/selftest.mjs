#!/usr/bin/env node
// selftest.mjs: checks analyze.mjs on synthetic row sets with known answers. No model, no network, a few seconds,
// deterministic output. Exit 1 when any check fails. Temp files live in one fresh folder under the OS temp dir and are
// removed by exact path (never a recursive delete).
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { analyze, buildUnits, clusterBoot, KINDS, loadRows, mcnemar, Refusal, render, wilson } from './analyze.mjs';

const CLI = path.join(path.dirname(fileURLToPath(import.meta.url)), 'analyze.mjs');
const near = (a, b, tol = 1e-9) => assert.ok(Math.abs(a - b) <= tol, `${a} is not within ${tol} of ${b}`);

// ---------- synthetic rows ----------
const BASE = {
  schema: 1, run_id: 'syn-1', task: 't00', app: 'notes', kind: 'natural-risk', torpor_should_help: 'yes', arm: 'A', rep: 1, attempt: 1,
  seed: 1, order_index: 0, model: 'syn-contributor', muse_version: '0.0', engine_version: null, adapter_commit: null,
  started_utc: '2026-01-01T00:00:00Z', ended_utc: '2026-01-01T00:07:00Z', minutes: 7, turns_run: 6, steps_total: 30, timeout: false,
  usd_lo: 0.1, usd_hi: 0.2, calls: 20, writer_usd_hi: 0, writer_calls: 0, rate_limited: 0, hook_failures: 0, gate_blocks: 0,
  oracle: null, final_pass: true, later_kept: true, break_found: false, valid: true, invalid_reason: null, manifest: {},
};
const mk = (o = {}) => {
  const r = { ...BASE, ...o };
  if (!('oracle' in o)) r.oracle = { exit: r.final_pass ? 0 : 1, line: '{}' };
  if (!r.valid && !('invalid_reason' in o)) r.invalid_reason = 'sandbox error';
  return r;
};
const tid = (i) => `t${String(i).padStart(2, '0')}`;
const kindAt = (i, T) => KINDS[Math.floor((i * 3) / T)];
// tasks x reps x arms; pass(arm, taskIndex, rep) says whether the episode passed
function dataset({ tasks, reps, arms, pass, model = 'syn-contributor' }) {
  const rows = [];
  for (let i = 0; i < tasks; i++) {
    for (const arm of arms) {
      for (let rep = 1; rep <= reps; rep++) {
        const kind = kindAt(i, tasks);
        const ok = pass(arm, i, rep);
        rows.push(mk({ task: tid(i), kind, arm, rep, final_pass: ok, later_kept: ok, break_found: kind === 'control' ? null : !ok, model }));
      }
    }
  }
  return rows;
}
const run = (rows, o = {}) => analyze(rows, [], { seed: 1, boot: 2000, ...o });
const find = (res, id, outcome = 'final_pass', kind = 'all') => res.comparisons.find((c) => c.id === id && c.outcome === outcome && c.kind === kind);

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'torpor-analysis-selftest-'));
const made = [];
function folder(name, files) {
  const dir = path.join(tmp, name);
  fs.mkdirSync(dir);
  made.push(dir);
  Object.entries(files).forEach(([f, o]) => {
    fs.writeFileSync(path.join(dir, f), typeof o === 'string' ? o : JSON.stringify(o));
    made.push(path.join(dir, f));
  });
  return dir;
}
const rowFiles = (rows) => Object.fromEntries(rows.map((r) => [`${r.arm}-${r.task}-r${r.rep}-a${r.attempt}.json`, r]));
const refused = (fn, re) => assert.throws(fn, (e) => e instanceof Refusal && re.test(e.lines.join('\n')), `expected a refusal matching ${re}`);

const tests = [];
const t = (name, fn) => tests.push([name, fn]);

// ---------- statistics ----------
t('McNemar b=10 c=2 gives p = 158/4096', () => {
  const m = mcnemar(10, 2);
  near(m.p, 158 / 4096, 1e-12);
  near(m.p, 0.03857, 5e-6);
  assert.equal(mcnemar(2, 10).p, m.p);
});
t('McNemar b=c gives p = 1; b+c=0 gives p = 1 with "no discordant pairs"', () => {
  assert.equal(mcnemar(5, 5).p, 1);
  assert.equal(mcnemar(1, 0).p, 1);
  const z = mcnemar(0, 0);
  assert.equal(z.p, 1);
  assert.equal(z.note, 'no discordant pairs');
  near(mcnemar(0, 6).p, 2 / 64, 1e-12);
  near(mcnemar(12, 0).p, 2 / 4096, 1e-12);
  assert.ok(mcnemar(300, 100).p < 1e-10 && mcnemar(300, 100).p >= 0);
});
t('Wilson interval: closed form at 0/40, textbook values at 8/10, n = 0', () => {
  const z = 1.959963984540054;
  const w0 = wilson(0, 40);
  assert.equal(w0.lo, 0);
  near(w0.hi, (z * z) / (40 + z * z), 1e-9);
  const w8 = wilson(8, 10);
  near(w8.lo, 0.4902, 5e-4);
  near(w8.hi, 0.9433, 5e-4);
  assert.equal(wilson(0, 0).rate, null);
});
t('cluster bootstrap: same seed gives the same result, the interval contains the point estimate', () => {
  const cl = [2, 1, 0, 2, -1, 1, 0, 2, 1, 1, 0, 2].map((d) => ({ d, n: 2 }));
  assert.deepEqual(clusterBoot(cl, 3000, 42), clusterBoot(cl, 3000, 42));
  for (let seed = 1; seed <= 25; seed++) {
    const r = clusterBoot(cl, 3000, seed);
    assert.ok(r.lo <= r.point && r.point <= r.hi, `seed ${seed}: ${r.lo} ${r.point} ${r.hi}`);
  }
  assert.equal(clusterBoot([{ d: 1, n: 2 }], 3000, 1).lo, null); // one task cannot be resampled
});

t('stray network lines (egress.other) are reported per arm and never exclude a unit', () => {
  const seen = (task, other) => mk({ task, egress: { allowed: 1, denied: other.length, via_fwd: 0, other } });
  const res = run([seen('e1', ['x.example:443 denied']), seen('e2', ['x.example:443 denied', 'y.example:80 denied']), mk({ task: 'e3', egress: null }), mk({ task: 'e4' })]);
  assert.deepEqual(res.by_arm.A.egress_other_lines, { 'x.example:443 denied': 2, 'y.example:80 denied': 1 });
  assert.deepEqual([res.by_arm.A.valid, res.by_arm.A.exhausted], [4, 0]);
});

// ---------- attempts and exclusions ----------
t('attempt selection, exclusion and pending counts; pairs need a valid unit on both sides', () => {
  const inv = (task, attempt, why, arm = 'A') => mk({ task, arm, attempt, valid: false, invalid_reason: why });
  const rows = [
    inv('a1', 1, 'sandbox error'), mk({ task: 'a1', attempt: 2, final_pass: false, later_kept: false, break_found: true }), // retried, then valid
    inv('a2', 1, 'r1'), inv('a2', 2, 'r2'), inv('a2', 3, 'r1'), // exhausted
    inv('a3', 1, 'r3'), // pending
    mk({ task: 'a4', attempt: 1 }), mk({ task: 'a4', attempt: 2, final_pass: false }), // the second valid attempt is ignored
    mk({ task: 'a5', attempt: 1 }),
    inv('a6', 0, 'zero based'), mk({ task: 'a6', attempt: 1 }), // attempt numbers need not start at 1
    // arms C and D: D valid on a7 and a8; C exhausted on a7 and valid on a8
    mk({ task: 'a7', arm: 'D' }), inv('a7', 1, 'x', 'C'), inv('a7', 2, 'x', 'C'), inv('a7', 3, 'x', 'C'),
    mk({ task: 'a8', arm: 'D' }), mk({ task: 'a8', arm: 'C', final_pass: false }),
  ];
  const u = buildUnits(rows);
  assert.equal(u.find((x) => x.key === 'A/a1/rep 1').row.attempt, 2);
  assert.equal(u.find((x) => x.key === 'A/a4/rep 1').row.attempt, 1);
  assert.equal(u.find((x) => x.key === 'A/a6/rep 1').row.attempt, 1);
  // one attempt the runner marked not re-runnable (inject_failed) is exhausted at once; the same attempt without the mark is pending
  assert.deepEqual(buildUnits([{ ...inv('b1', 1, 'inject_failed: x'), retryable: false }, inv('b2', 1, 'inject_failed: x')]).map((x) => x.status), ['exhausted', 'pending']);
  const res = run(rows);
  const a = res.by_arm.A;
  assert.deepEqual([a.units, a.valid, a.exhausted, a.pending, a.attempts, a.invalid_attempts], [6, 4, 1, 1, 11, 6]);
  assert.deepEqual(a.invalid_attempt_reasons, { 'sandbox error': 1, r1: 2, r2: 1, r3: 1, 'zero based': 1 });
  assert.deepEqual(a.excluded_unit_reasons, { r1: 2, r2: 1, r3: 1 });
  assert.deepEqual([a.final_pass.k, a.final_pass.n], [3, 4]); // a1 failed, a4 passed (first attempt), a5 and a6 passed
  assert.deepEqual([res.units.total, res.units.exhausted, res.units.pending], [10, 2, 1]); // 6 in A, 2 in C, 2 in D
  assert.ok(res.warnings.some((w) => /pending|fewer than 3 attempts/.test(w)) && res.warnings.some((w) => /after the first valid/.test(w)));
  const dc = find(res, 'D_vs_C');
  assert.deepEqual([dc.pairs, dc.b, dc.c], [1, 1, 0]); // only a8 is valid in both
});
t('break_found null on a risk-kind row is allowed: left out of the break_found rows with a warning', () => {
  const res = run([mk({ task: 'n1', break_found: null }), mk({ task: 'n2', break_found: true, final_pass: false }), mk({ task: 'n3', kind: 'control', break_found: null })]);
  assert.deepEqual([res.by_arm.A.break_found.k, res.by_arm.A.break_found.n, res.by_arm.A.final_pass.n], [1, 1, 3]);
  assert.ok(res.warnings.some((w) => /1 valid row\(s\) of a risk kind have break_found null/.test(w)));
  refused(() => loadRows(folder('bf', { 'a.json': mk({ break_found: 'maybe' }) })), /break_found is not true, false or null/);
});
t('a timeout counts as a failure for final_pass and later_kept; break_found stays as recorded', () => {
  const res = run([mk({ task: 'x1', timeout: true, final_pass: true, later_kept: true, break_found: true }), mk({ task: 'x2' })]);
  const a = res.by_arm.A;
  assert.deepEqual([a.final_pass.k, a.later_kept.k, a.break_found.k, a.timeouts.k], [1, 1, 1, 1]);
  assert.ok(res.warnings.some((w) => /timed-out/.test(w)));
});

// ---------- refusals ----------
t('a schema-2 row, a missing field, a bad enum, a duplicate, bad JSON and a fourth attempt are refused', () => {
  refused(() => loadRows(folder('s2', { 'a.json': mk({ schema: 2 }) })), /a\.json: schema 2/);
  const { gate_blocks, ...noGate } = mk();
  void gate_blocks;
  refused(() => loadRows(folder('mf', { 'a.json': noGate })), /a\.json: missing field gate_blocks/);
  refused(() => loadRows(folder('arm', { 'a.json': mk({ arm: 'E' }) })), /arm "E"/);
  refused(() => loadRows(folder('dup', { 'a.json': mk(), 'b.json': mk() })), /b\.json: duplicate of a\.json/);
  refused(() => loadRows(folder('bad', { 'a.json': '{ nope' })), /not valid JSON/);
  refused(() => loadRows(folder('inv', { 'a.json': mk({ valid: false, invalid_reason: '' }) })), /without an invalid_reason/);
  refused(() => run([1, 2, 3, 4].map((attempt) => mk({ attempt, valid: false }))), /4 attempts/);
});
t('null is a legal value (adapter_commit, engine_version, break_found on a control, invalid_reason); a stray JSON file is skipped and listed', () => {
  const ctrl = mk({ task: 'c1', kind: 'control', break_found: null });
  const { rows, skipped } = loadRows(folder('ok', { ...rowFiles([ctrl, mk({ task: 'c2', valid: false, invalid_reason: 'x', model: null, final_pass: null })]), 'plan.json': { tasks: ['c1'] } }));
  assert.equal(rows.length, 2);
  assert.deepEqual(skipped, ['plan.json']);
  const res = analyze(rows, skipped, { seed: 1, boot: 200 });
  assert.ok(res.warnings.some((w) => /plan\.json/.test(w)));
});
t('mixed model ids in one comparison are refused; one model per folder is fine', () => {
  const rows = dataset({ tasks: 4, reps: 1, arms: ['C', 'D'], pass: () => true });
  rows.find((r) => r.arm === 'C').model = 'other-model';
  refused(() => run(rows), /D vs C mixes model ids other-model, syn-contributor/);
  assert.ok(run(dataset({ tasks: 4, reps: 1, arms: ['C', 'D'], pass: () => true })));
});

// ---------- verdict words ----------
t('a planted strong effect yields "D better than C"; the secondary D vs A gets the numbers and no verdict word', () => {
  const rows = dataset({ tasks: 12, reps: 2, arms: ['A', 'C', 'D'], pass: (arm, i) => arm === 'D' || i < 3 });
  const res = run(rows);
  const dc = find(res, 'D_vs_C');
  assert.deepEqual([dc.pairs, dc.tasks, dc.b, dc.c], [24, 12, 18, 0]);
  assert.equal(dc.verdict, 'D better than C');
  assert.ok(dc.p < 1e-4 && dc.ci.lo > 0 && dc.diff === 0.75);
  const da = find(res, 'D_vs_A'); // the same planted effect, but only the primary comparison has a verdict word
  assert.ok(da.preregistered && da.label === 'engine + new work' && da.p < 1e-4 && da.ci.lo > 0 && da.diff === 0.75);
  assert.equal(da.verdict, null);
  assert.ok(res.comparisons.filter((c) => c.verdict !== null).every((c) => c.id === 'D_vs_C' && c.outcome === 'final_pass' && c.kind === 'all'));
  assert.ok(!/better than A/.test(render(res)));
  assert.ok(!res.comparisons.some((c) => c.id === 'B_vs_A')); // B absent: no B vs A
});
t('B vs A is a secondary comparison too: numbers, no verdict word', () => {
  const res = run(dataset({ tasks: 12, reps: 2, arms: ['A', 'B', 'C', 'D'], pass: (arm, i) => arm !== 'A' || i < 3 }));
  const ba = find(res, 'B_vs_A');
  assert.deepEqual([ba.pairs, ba.b, ba.c, ba.diff, ba.verdict], [24, 18, 0, 0.75, null]);
  assert.ok(ba.p < 1e-4 && ba.ci.lo > 0);
  assert.ok(render(res).includes('B vs A (notes file vs vanilla): secondary, no verdict word. Effect +0.75'));
  assert.ok(!/better than A/.test(render(res)));
});
t('a planted null at n=10 yields "inconclusive at this n" with the effect and the interval', () => {
  // tasks 0-1: D only passes; 2-3: C only; 4-6: both; 7-9: neither -> b = 2, c = 2
  const pass = (arm, i) => (i < 2 ? arm === 'D' : i < 4 ? arm === 'C' : i < 7);
  const res = run(dataset({ tasks: 10, reps: 1, arms: ['C', 'D'], pass }));
  const dc = find(res, 'D_vs_C');
  assert.deepEqual([dc.pairs, dc.b, dc.c, dc.p, dc.diff], [10, 2, 2, 1, 0]);
  assert.equal(dc.verdict, 'inconclusive at this n');
  assert.ok(dc.ci.lo <= 0 && dc.ci.hi >= 0);
  assert.ok(/Effect 0\.00, 95 % CI \[/.test(render(res)));
  assert.equal(find(res, 'D_vs_A'), undefined); // no A rows: D vs A is not computed
  assert.ok(!render(res).includes('D vs A'));
});
t('two arms with no (task, rep) valid in both print "No pairs" instead of an effect', () => {
  const res = run([mk({ task: 't00', arm: 'C' }), mk({ task: 't01', arm: 'D' })]);
  const dc = find(res, 'D_vs_C');
  assert.deepEqual([dc.pairs, dc.b, dc.c, dc.diff, dc.ci], [0, 0, 0, null, null]);
  assert.equal(dc.verdict, 'inconclusive at this n');
  assert.ok(render(res).includes('D vs C (primary): inconclusive at this n. No pairs: no (task, rep) is valid in both arms.'));
});
t('the verdict needs the interval too: all discordance in one task gives p < 0.05 but "inconclusive at this n"', () => {
  const rows = [];
  for (let rep = 1; rep <= 10; rep++) for (const arm of ['C', 'D']) rows.push(mk({ task: 't00', arm, rep, final_pass: arm === 'D' }));
  for (let i = 1; i < 10; i++) for (const arm of ['C', 'D']) rows.push(mk({ task: tid(i), arm }));
  const dc = find(run(rows, { boot: 4000 }), 'D_vs_C');
  assert.deepEqual([dc.pairs, dc.tasks, dc.b, dc.c], [19, 10, 10, 0]);
  assert.ok(dc.p < 0.05);
  assert.equal(dc.ci.lo, 0); // about 35 % of the resamples draw no t00 (tasks are resampled, not pairs)
  assert.equal(dc.verdict, 'inconclusive at this n');
});
t('a significant effect in the wrong direction is "inconclusive at this n" with the signed effect and a plain note', () => {
  const res = run(dataset({ tasks: 12, reps: 2, arms: ['C', 'D'], pass: (arm, i) => arm === 'C' || i < 3 }));
  const dc = find(res, 'D_vs_C');
  assert.equal(dc.verdict, 'inconclusive at this n');
  assert.ok(dc.diff < 0 && dc.ci.hi < 0 && /interval lies below 0/.test(dc.direction_note));
  assert.ok(/Effect -0\.75/.test(render(res)));
});
t('zero events print "under about 3/n" with n filled in; false alarms have their own row', () => {
  const res = run(dataset({ tasks: 20, reps: 2, arms: ['A', 'C', 'D'], pass: () => true }));
  const d = res.by_arm.D;
  assert.deepEqual([d.timeouts.k, d.timeouts.n, d.false_alarms.k, d.false_alarms.n], [0, 40, 0, 12]);
  const md = render(res);
  assert.ok(md.includes('under about 3/40 (7.5 %)') && md.includes('under about 3/12 (25.0 %)'));
  assert.ok(/\| false alarms: control episodes with >= 1 gate block \|/.test(md));
  assert.ok(/\| hook_failures: total; episodes with >= 1 \| 0; 0\/40 0\.0 % \[[0-9.-]+\], under about 3\/40/.test(md));
});

// ---------- the markdown ----------
t('the markdown puts a source line (model id, run id) above every table, labels D vs A, and has the per-task k/n tables', () => {
  const md = render(run(dataset({ tasks: 12, reps: 2, arms: ['A', 'B', 'C', 'D'], pass: (arm, i) => arm === 'D' || i < 3 })));
  const lines = md.split('\n');
  const heads = lines.flatMap((l, i) => (l.startsWith('|') && !lines[i - 1].startsWith('|') ? [i] : []));
  assert.equal(heads.length, 9); // pre-registered rows, other rows, per arm, 3 kinds, 3 per-task tables
  for (const i of heads) assert.equal(lines[i - 2], '*Source: model `syn-contributor`; run id `syn-1`.*', `no source line above the table at line ${i + 1}`);
  assert.ok(md.includes('D vs A (engine + new work): secondary, no verdict word. Effect +0.75'));
  assert.ok(md.includes('| task | kind | torpor_should_help | pairs | D passes | C passes | b | c |'));
  assert.ok(/\| t05 \| injected \| yes \| 2 \| 2\/2 \| 0\/2 \| 2 \| 0 \|/.test(md));
});
t('an A+D-only folder (the confirmation subset) shows D vs A with its numbers and no verdict word, and warns that D vs C is not computed', () => {
  const res = run(dataset({ tasks: 12, reps: 2, arms: ['A', 'D'], pass: (arm) => arm === 'D' }));
  assert.ok(res.comparisons.length > 0 && res.comparisons.every((c) => c.id === 'D_vs_A' && c.verdict === null));
  assert.ok(res.warnings.some((w) => w === 'D vs C (primary) is not computed: the folder has no rows for arm C'));
  const da = find(res, 'D_vs_A');
  assert.deepEqual([da.pairs, da.b, da.c, da.diff], [24, 24, 0, 1]);
  const md = render(res);
  assert.ok(md.includes('D vs A (engine + new work): secondary, no verdict word. Effect +1.00'));
  assert.ok(!md.split('\n').some((l) => /^[-|] ?D vs A/.test(l) && /better than|inconclusive/.test(l)));
});
t('a folder with one arm has no paired rows and says so instead of printing an empty table', () => {
  const md = render(run(dataset({ tasks: 4, reps: 1, arms: ['D'], pass: () => true })));
  assert.ok(md.includes('(none: a paired comparison needs both arms in the folder)'));
  assert.ok(!md.includes('| comparison |'));
});

// ---------- end to end ----------
t('end to end: a synthetic folder gives the markdown and the JSON, identical on a second run; a bad row exits 2', () => {
  const rows = dataset({ tasks: 12, reps: 2, arms: ['A', 'B', 'C', 'D'], pass: (arm, i) => (arm === 'D' ? true : arm === 'B' ? i < 4 : arm === 'C' ? i < 3 : i < 2) });
  const dir = folder('e2e', { ...rowFiles(rows), 'plan.json': { note: 'not a row' } });
  const go = (tag) => {
    const md = path.join(tmp, `${tag}.md`);
    const js = path.join(tmp, `${tag}.json`);
    made.push(md, js);
    const r = spawnSync(process.execPath, [CLI, '--rows', dir, '--md', md, '--json', js, '--seed', '7', '--boot', '2000'], { encoding: 'utf8' });
    return { r, md: fs.existsSync(md) ? fs.readFileSync(md, 'utf8') : '', json: fs.existsSync(js) ? JSON.parse(fs.readFileSync(js, 'utf8')) : null };
  };
  const one = go('one');
  assert.equal(one.r.status, 0, one.r.stderr);
  assert.ok(/D vs C \(primary\): D better than C\. Effect \+0\.75/.test(one.r.stdout) && one.r.stderr.includes('plan.json'));
  assert.ok(one.md.includes('# Eval results') && one.md.includes('Source: model `syn-contributor`; run id `syn-1`.') && one.md.includes('seed 7'));
  assert.ok(one.md.includes('D better than C') && one.md.includes('B vs A (notes file vs vanilla)') && /sha256 `[0-9a-f]{64}`/.test(one.md));
  assert.equal(one.json.analysis.seed, 7);
  assert.ok(one.json.comparisons.some((c) => c.id === 'D_vs_C' && c.preregistered && c.verdict === 'D better than C'));
  assert.deepEqual(one.json.source.non_row_files_skipped, ['plan.json']);
  const two = go('two');
  assert.equal(two.md, one.md);
  assert.deepEqual(two.json, one.json);
  const badDir = folder('e2e-bad', { 'x.json': mk({ schema: 2 }) });
  const bad = spawnSync(process.execPath, [CLI, '--rows', badDir], { encoding: 'utf8' });
  assert.equal(bad.status, 2);
  assert.ok(/analyze: refused: x\.json: schema 2/.test(bad.stderr) && bad.stdout === '');
  assert.equal(spawnSync(process.execPath, [CLI], { encoding: 'utf8' }).status, 2);
  const nowhere = spawnSync(process.execPath, [CLI, '--rows', dir, '--md', path.join(tmp, 'no-such-folder', 'x.md'), '--boot', '200'], { encoding: 'utf8' });
  assert.equal(nowhere.status, 2);
  assert.ok(/analyze: refused: cannot write the output/.test(nowhere.stderr));
});

// ---------- run ----------
let failed = 0;
try {
  for (const [name, fn] of tests) {
    try {
      fn();
      console.log(`ok   ${name}`);
    } catch (e) {
      failed++;
      console.log(`FAIL ${name}\n     ${String(e.message).split('\n').slice(0, 14).join('\n     ')}`);
    }
  }
} finally {
  for (const p of made.reverse()) {
    try {
      if (fs.statSync(p).isDirectory()) fs.rmdirSync(p);
      else fs.unlinkSync(p);
    } catch {
      // already gone
    }
  }
  try {
    fs.rmdirSync(tmp);
  } catch {
    console.log(`note: left ${tmp} (not empty)`);
  }
}
console.log(failed ? `selftest: ${failed} of ${tests.length} FAILED` : `selftest: ${tests.length} of ${tests.length} ok`);
process.exitCode = failed ? 1 : 0;

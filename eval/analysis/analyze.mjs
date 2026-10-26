#!/usr/bin/env node
// analyze.mjs --rows DIR [--md OUT.md] [--json OUT.json] [--seed S] [--boot 10000]
// Turns a folder of eval result rows (schema 1, one JSON file per episode attempt, written by eval/run) into the
// result tables of eval/PREREG.md. No model calls, no dependencies. The rules applied here are written in PREREG.md
// section 7; the sha256 of this file is printed in every output, so a result can be tied to the tagged code.
// Exit 0 = written. Exit 2 = refused or bad usage (one "analyze: refused:" line per problem, nothing analysed).
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';

const SELF = fileURLToPath(import.meta.url);
export const ARMS = ['A', 'B', 'C', 'D'];
export const KINDS = ['natural-risk', 'injected', 'control'];
export const MAX_ATTEMPTS = 3; // the first run plus up to two re-runs
const Z = 1.959963984540054; // Wilson 95 % interval
const REQUIRED = [
  'schema', 'run_id', 'task', 'app', 'kind', 'torpor_should_help', 'arm', 'rep', 'attempt', 'seed', 'order_index', 'model',
  'muse_version', 'engine_version', 'adapter_commit', 'started_utc', 'ended_utc', 'minutes', 'turns_run', 'steps_total',
  'timeout', 'usd_lo', 'usd_hi', 'calls', 'writer_usd_hi', 'writer_calls', 'rate_limited', 'hook_failures', 'gate_blocks',
  'oracle', 'final_pass', 'later_kept', 'break_found', 'valid', 'invalid_reason', 'manifest',
]; // a field is missing when its key is absent; null is a legal value (arm A has no adapter, a control has no break_found)
const ROW_HINT = ['arm', 'task', 'rep', 'attempt', 'valid', 'final_pass']; // a JSON file with none of these is not a row
const COMPARISONS = [
  { x: 'D', y: 'C', label: 'primary', primary: true }, // the only comparison that gets a verdict word
  { x: 'D', y: 'A', label: 'engine + new work' },
  { x: 'B', y: 'A', label: 'notes file vs vanilla' },
];
// 1 = good, so a positive effect always favours the first-named arm. A timeout is a failure in every arm: it overrides
// final_pass and later_kept (not break_found, which is a fact about the end state).
export const OUTCOMES = {
  final_pass: (r) => r.final_pass && !r.timeout,
  later_kept: (r) => r.later_kept && !r.timeout,
  no_break: (r) => (r.kind === 'control' || r.break_found === null ? null : !r.break_found), // null = not measured, left out of the pair
};

export class Refusal extends Error {
  constructor(lines) {
    super(lines[0]);
    this.lines = lines;
  }
}

// ---------- statistics ----------
const sum = (a) => a.reduce((x, y) => x + y, 0);
const uniq = (a) => [...new Set(a)].sort();
export function median(a) {
  if (!a.length) return null;
  const s = [...a].sort((x, y) => x - y);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}

export function wilson(k, n) {
  if (!n) return { k, n, rate: null, lo: null, hi: null };
  const p = k / n;
  const d = 1 + (Z * Z) / n;
  const centre = (p + (Z * Z) / (2 * n)) / d;
  const half = (Z * Math.sqrt((p * (1 - p)) / n + (Z * Z) / (4 * n * n))) / d;
  return { k, n, rate: p, lo: Math.max(0, centre - half), hi: Math.min(1, centre + half) };
}

// Exact two-sided McNemar: binomial(b + c, 1/2), p = min(1, 2 * P(X <= min(b, c))). BigInt keeps it exact.
export function mcnemar(b, c) {
  const n = b + c;
  if (n === 0) return { b, c, n, p: 1, note: 'no discordant pairs' };
  let s = 0n;
  let t = 1n; // C(n, i)
  for (let i = 0; i <= Math.min(b, c); i++) {
    s += t;
    t = (t * BigInt(n - i)) / BigInt(i + 1);
  }
  const num = 2n * s;
  const den = 1n << BigInt(n);
  return { b, c, n, p: num >= den ? 1 : Number((num * 10n ** 30n) / den) / 1e30, note: null };
}

export function prng(seed) {
  // mulberry32
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), a | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// Cluster bootstrap of the pooled paired difference. clusters = one {d: b - c, n: pairs} per task. Each resample draws
// as many TASKS as there are tasks, with replacement, and recomputes sum(d) / sum(n). Percentile interval, 2.5 % / 97.5 %.
export function clusterBoot(clusters, B, seed) {
  const T = clusters.length;
  const point = sum(clusters.map((c) => c.d)) / sum(clusters.map((c) => c.n));
  if (T < 2) return { point, lo: null, hi: null, tasks: T }; // one task cannot be resampled
  const rand = prng(seed);
  const est = new Float64Array(B);
  for (let i = 0; i < B; i++) {
    let d = 0;
    let n = 0;
    for (let j = 0; j < T; j++) {
      const c = clusters[Math.floor(rand() * T)];
      d += c.d;
      n += c.n;
    }
    est[i] = d / n;
  }
  est.sort();
  return { point, lo: est[Math.floor(0.025 * B)], hi: est[Math.min(B - 1, Math.floor(0.975 * B))], tasks: T };
}

// ---------- loading ----------
const isNum = (v) => typeof v === 'number' && Number.isFinite(v) && v >= 0;

function problems(o) {
  if ('schema' in o && o.schema !== 1) return [`schema ${JSON.stringify(o.schema)}, this analysis reads schema 1 only`];
  const miss = REQUIRED.filter((k) => !(k in o));
  if (miss.length) return [`missing field${miss.length > 1 ? 's' : ''} ${miss.join(', ')}`];
  const p = [];
  if (!ARMS.includes(o.arm)) p.push(`arm ${JSON.stringify(o.arm)} is not A, B, C or D`);
  if (!KINDS.includes(o.kind)) p.push(`kind ${JSON.stringify(o.kind)} is not natural-risk, injected or control`);
  for (const k of ['task', 'run_id']) if (typeof o[k] !== 'string' || !o[k]) p.push(`${k} is empty`);
  for (const k of ['rep', 'attempt']) if (!Number.isInteger(o[k]) || o[k] < 0) p.push(`${k} is not a whole number >= 0`);
  if (typeof o.valid !== 'boolean') p.push('valid is not true or false');
  else if (!o.valid) {
    if (typeof o.invalid_reason !== 'string' || !o.invalid_reason) p.push('invalid row without an invalid_reason');
  } else {
    if (typeof o.model !== 'string' || !o.model) p.push('model is empty');
    for (const k of ['minutes', 'turns_run', 'steps_total', 'usd_hi', 'writer_usd_hi', 'writer_calls', 'gate_blocks']) {
      if (!isNum(o[k])) p.push(`${k} is not a number >= 0`);
    }
    for (const k of ['rate_limited', 'hook_failures']) if (typeof o[k] !== 'boolean' && !isNum(o[k])) p.push(`${k} is not a number or true/false`);
    for (const k of ['timeout', 'final_pass', 'later_kept']) if (typeof o[k] !== 'boolean') p.push(`${k} is not true or false`);
    if (o.break_found !== null && typeof o.break_found !== 'boolean') p.push('break_found is not true, false or null');
    if (!o.oracle || typeof o.oracle !== 'object') p.push('oracle is not an object');
  }
  return p;
}

function walk(dir, depth, out) {
  for (const name of fs.readdirSync(dir)) {
    const f = path.join(dir, name);
    const st = fs.statSync(f, { throwIfNoEntry: false }); // a broken symlink is skipped
    if (!st) continue;
    if (st.isDirectory()) {
      if (depth < 6) walk(f, depth + 1, out);
    } else if (name.endsWith('.json')) out.push(f);
  }
}

export function loadRows(dir) {
  if (!fs.existsSync(dir) || !fs.statSync(dir).isDirectory()) throw new Refusal([`--rows ${dir} is not a folder`]);
  const files = [];
  walk(dir, 0, files);
  files.sort();
  const rows = [];
  const skipped = [];
  const bad = [];
  const seen = new Map();
  const kindOf = new Map();
  for (const f of files) {
    const rel = path.relative(dir, f);
    let o;
    try {
      o = JSON.parse(fs.readFileSync(f, 'utf8'));
    } catch (e) {
      bad.push(`${rel}: not valid JSON (${String(e.message).slice(0, 60)})`);
      continue;
    }
    if (!o || typeof o !== 'object' || Array.isArray(o) || !ROW_HINT.some((k) => k in o)) {
      skipped.push(rel);
      continue;
    }
    const pr = problems(o);
    if (!pr.length) {
      const id = `${o.task}/${o.arm}/rep ${o.rep}/attempt ${o.attempt}`;
      if (seen.has(id)) pr.push(`duplicate of ${seen.get(id)} (${id})`);
      else seen.set(id, rel);
      if (kindOf.has(o.task) && kindOf.get(o.task) !== o.kind) pr.push(`task ${o.task} has kind ${o.kind} here and ${kindOf.get(o.task)} elsewhere`);
      else kindOf.set(o.task, o.kind);
    }
    if (pr.length) bad.push(`${rel}: ${pr.join('; ')}`);
    else rows.push(o);
  }
  if (bad.length) throw new Refusal([...bad.slice(0, 20), ...(bad.length > 20 ? [`... and ${bad.length - 20} more bad rows`] : [])]);
  if (!rows.length) throw new Refusal([`no rows in ${dir}`]);
  return { rows, skipped };
}

// A unit is one (arm, task, rep). Its row is the first valid attempt; a unit with no valid attempt after
// MAX_ATTEMPTS is exhausted (excluded, counted); with fewer attempts it is pending (excluded, flagged).
export function buildUnits(rows) {
  const groups = new Map();
  for (const r of rows) {
    const key = `${r.arm}/${r.task}/rep ${r.rep}`;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(r);
  }
  const units = [];
  for (const [key, rs] of groups) {
    rs.sort((a, b) => a.attempt - b.attempt);
    if (rs.length > MAX_ATTEMPTS) throw new Refusal([`${key}: ${rs.length} attempts, the rule allows ${MAX_ATTEMPTS}`]);
    const row = rs.find((r) => r.valid) ?? null;
    units.push({
      key, arm: rs[0].arm, task: rs[0].task, rep: rs[0].rep, kind: rs[0].kind, attempts: rs, row,
      status: row ? 'valid' : rs.length >= MAX_ATTEMPTS ? 'exhausted' : 'pending',
      ignored: row ? rs.filter((r) => r.attempt > row.attempt).length : 0,
    });
  }
  return units;
}

// ---------- tables ----------
function armReport(units) {
  const rows = units.filter((u) => u.row).map((u) => u.row);
  const n = rows.length;
  const risk = rows.filter((r) => r.kind !== 'control' && r.break_found !== null);
  const ctrl = rows.filter((r) => r.kind === 'control');
  const atts = units.flatMap((u) => u.attempts);
  const bad = atts.filter((r) => !r.valid);
  const tally = (rs) => {
    const m = {};
    for (const r of rs) m[r.invalid_reason] = (m[r.invalid_reason] ?? 0) + 1;
    return m;
  };
  const num = (v) => (typeof v === 'boolean' ? +v : v);
  const stat = (f) => ({ median: median(rows.map(f)), total: sum(rows.map(f)) });
  const count = (f) => ({ total: sum(rows.map((r) => num(r[f]))), episodes: wilson(rows.filter((r) => num(r[f]) > 0).length, n) });
  return {
    units: units.length,
    valid: n,
    exhausted: units.filter((u) => u.status === 'exhausted').length,
    pending: units.filter((u) => u.status === 'pending').length,
    attempts: atts.length,
    invalid_attempts: bad.length,
    invalid_attempt_reasons: tally(bad),
    excluded_unit_reasons: tally(units.filter((u) => !u.row).flatMap((u) => u.attempts)),
    final_pass: wilson(rows.filter(OUTCOMES.final_pass).length, n),
    later_kept: wilson(rows.filter(OUTCOMES.later_kept).length, n),
    break_found: wilson(risk.filter((r) => r.break_found).length, risk.length),
    timeouts: wilson(rows.filter((r) => r.timeout).length, n),
    gate_block_episodes: wilson(rows.filter((r) => r.gate_blocks > 0).length, n),
    false_alarms: wilson(ctrl.filter((r) => r.gate_blocks > 0).length, ctrl.length),
    gate_blocks: { total: sum(rows.map((r) => r.gate_blocks)), per_episode: n ? sum(rows.map((r) => r.gate_blocks)) / n : null },
    rate_limited: count('rate_limited'),
    hook_failures: count('hook_failures'),
    minutes: stat((r) => r.minutes),
    turns: stat((r) => r.turns_run),
    usd_hi: stat((r) => r.usd_hi),
    steps: stat((r) => r.steps_total),
    writer: { usd_hi_total: sum(rows.map((r) => r.writer_usd_hi)), calls_total: sum(rows.map((r) => r.writer_calls)) },
    models: uniq(rows.map((r) => r.model)),
    run_ids: uniq(atts.map((r) => r.run_id)),
  };
}

// Pairs = the same (task, rep) valid in both arms. b = x passed and y failed; c = the reverse.
export function compare(units, x, y, outcome, kind, B, seed) {
  const pick = (arm) => new Map(units.filter((u) => u.arm === arm && u.row && (!kind || u.kind === kind)).map((u) => [`${u.task}|${u.rep}`, u]));
  const ux = pick(x);
  const uy = pick(y);
  const tasks = new Map();
  const used = [];
  for (const [key, a] of ux) {
    const o = uy.get(key);
    const px = o ? outcome(a.row) : null;
    const py = o ? outcome(o.row) : null;
    if (px === null || py === null) continue;
    used.push(a.row, o.row);
    const t = tasks.get(a.task) ?? { task: a.task, kind: a.kind, helps: a.row.torpor_should_help, pairs: 0, kx: 0, ky: 0, b: 0, c: 0 };
    t.pairs++;
    t.kx += +px;
    t.ky += +py;
    t.b += +(px && !py);
    t.c += +(!px && py);
    tasks.set(a.task, t);
  }
  const per = [...tasks.values()].sort((p, q) => (p.task < q.task ? -1 : 1));
  const [b, c, pairs] = [sum(per.map((t) => t.b)), sum(per.map((t) => t.c)), sum(per.map((t) => t.pairs))];
  const mc = mcnemar(b, c);
  return {
    x, y, kind: kind ?? 'all', pairs, tasks: per.length, kx: sum(per.map((t) => t.kx)), ky: sum(per.map((t) => t.ky)), b, c,
    p: mc.p, note: pairs ? mc.note : 'no pairs', diff: pairs ? (b - c) / pairs : null,
    ci: pairs ? clusterBoot(per.map((t) => ({ d: t.b - t.c, n: t.pairs })), B, seed) : null,
    per_task: per, models: uniq(used.map((r) => r.model)), run_ids: uniq(used.map((r) => r.run_id)),
  };
}

// The only verdict words (PREREG 7.2), for the primary comparison D vs C only. Strictly: p < 0.05 AND the whole interval above 0.
export function verdict(cmp) {
  const ok = cmp.pairs > 0 && cmp.p < 0.05 && cmp.ci?.lo > 0 && cmp.diff > 0;
  return {
    verdict: ok ? `${cmp.x} better than ${cmp.y}` : 'inconclusive at this n',
    direction_note: cmp.ci?.hi < 0 ? `the interval lies below 0 (${cmp.x} scored lower than ${cmp.y} on these pairs)` : null,
  };
}

function checkModels(units, arms) {
  const models = (as) => uniq(units.filter((u) => u.row && as.includes(u.arm)).map((u) => u.row.model));
  const bad = [];
  for (const a of arms) if (models([a]).length > 1) bad.push(`arm ${a} mixes model ids ${models([a]).join(', ')}`);
  for (const { x, y } of COMPARISONS) {
    if (arms.includes(x) && arms.includes(y) && models([x, y]).length > 1) bad.push(`${x} vs ${y} mixes model ids ${models([x, y]).join(', ')}; analyse one model per folder`);
  }
  if (bad.length) throw new Refusal(bad);
}

export function analyze(rows, skipped, { seed, boot }) {
  const units = buildUnits(rows);
  const arms = ARMS.filter((a) => units.some((u) => u.arm === a));
  checkModels(units, arms);
  const used = units.filter((u) => u.row).map((u) => u.row);
  const by_arm = {};
  const by_kind = {};
  for (const a of arms) by_arm[a] = armReport(units.filter((u) => u.arm === a));
  for (const k of KINDS) {
    if (!units.some((u) => u.kind === k)) continue;
    by_kind[k] = {};
    for (const a of arms) if (units.some((u) => u.kind === k && u.arm === a)) by_kind[k][a] = armReport(units.filter((u) => u.kind === k && u.arm === a));
  }
  const warnings = [];
  const comparisons = [];
  for (const { x, y, label, primary } of COMPARISONS) {
    if (!arms.includes(x) || !arms.includes(y)) { // a comparison needs both arms in the folder; B is optional, so only the primary warns
      if (primary) warnings.push(`${x} vs ${y} (${label}) is not computed: the folder has no rows for arm ${[x, y].filter((a) => !arms.includes(a)).join(' and ')}`);
      continue;
    }
    const add = (outcome, kind, preregistered) => {
      const c = { id: `${x}_vs_${y}`, label, outcome, preregistered, verdict: null, direction_note: null, ...compare(units, x, y, OUTCOMES[outcome], kind, boot, seed) };
      comparisons.push(preregistered && primary ? { ...c, ...verdict(c) } : c);
    };
    add('final_pass', null, true);
    add('later_kept', null, false);
    add('no_break', null, false);
    for (const k of KINDS) if (units.some((u) => u.kind === k)) add('final_pass', k, false);
  }
  const first = (a) => a.slice(0, 5).join(', ') + (a.length > 5 ? ', ...' : '');
  if (skipped.length) warnings.push(`${skipped.length} JSON file(s) in the folder are not rows and were skipped: ${first(skipped)}`);
  const pend = units.filter((u) => u.status === 'pending');
  if (pend.length) warnings.push(`${pend.length} unit(s) have fewer than ${MAX_ATTEMPTS} attempts and none valid (an unfinished run, or a unit the runner did not re-run) and are excluded: ${first(pend.map((u) => u.key))}`);
  const ign = sum(units.map((u) => u.ignored));
  if (ign) warnings.push(`${ign} attempt(s) after the first valid attempt of their unit were ignored`);
  const nob = used.filter((r) => r.kind !== 'control' && r.break_found === null).length;
  if (nob) warnings.push(`${nob} valid row(s) of a risk kind have break_found null and are left out of the break_found rows`);
  const over = used.filter((r) => r.timeout && (r.final_pass || r.later_kept)).length;
  if (over) warnings.push(`${over} timed-out row(s) had final_pass or later_kept true and are counted as failures`);
  const odd = used.filter((r) => !r.timeout && (![0, 1].includes(r.oracle.exit) || r.final_pass !== (r.oracle.exit === 0))).length;
  if (odd) warnings.push(`${odd} valid row(s) whose oracle.exit is not 0 or 1, or disagrees with final_pass (exit 0 = pass)`);
  return {
    schema: 1,
    analysis: { script_sha256: crypto.createHash('sha256').update(fs.readFileSync(SELF)).digest('hex'), seed, boot, wilson_z: Z, max_attempts: MAX_ATTEMPTS },
    source: { models: uniq(used.map((r) => r.model)), run_ids: uniq(rows.map((r) => r.run_id)), rows: rows.length, non_row_files_skipped: skipped },
    units: { total: units.length, valid: used.length, exhausted: units.filter((u) => u.status === 'exhausted').length, pending: pend.length },
    warnings, by_arm, by_kind, comparisons,
  };
}

// ---------- markdown ----------
const esc = (s) => String(s).replace(/\|/g, '\\|').replace(/\s+/g, ' ');
const table = (head, rows) => (rows.length ? [`| ${head.join(' | ')} |`, `|${head.map(() => '---').join('|')}|`, ...rows.map((r) => `| ${r.map(esc).join(' | ')} |`)].join('\n') : '(none: a paired comparison needs both arms in the folder)');
const src = (models, runs) => `*Source: model ${models.map((m) => `\`${m}\``).join(', ') || 'none'}; run id${runs.length === 1 ? '' : 's'} ${runs.map((r) => `\`${r}\``).join(', ') || 'none'}.*`;
const fp = (p) => (p < 0.001 ? '<0.001' : p.toFixed(3));
const fd = (d) => {
  if (d === null || d === undefined) return 'n/a';
  const s = d.toFixed(2);
  return s === '-0.00' || s === '0.00' ? '0.00' : d > 0 ? `+${s}` : s;
};
const fci = (ci) => (ci && ci.lo !== null ? `[${fd(ci.lo)}, ${fd(ci.hi)}]` : 'n/a');
const prop = (w) => {
  if (!w.n) return 'n/a';
  const pct = (x) => (100 * x).toFixed(1);
  return `${w.k}/${w.n} ${pct(w.rate)} % [${pct(w.lo)}-${pct(w.hi)}]${w.k === 0 ? `, under about 3/${w.n} (${Math.min(100, 300 / w.n).toFixed(1)} %)` : ''}`;
};
const f1 = (x) => (x === null ? 'n/a' : Number.isInteger(x) ? String(x) : x.toFixed(1));
const usd = (x) => (x === null ? 'n/a' : `$${x.toFixed(3)}`);
const name = (c) => `${c.x} vs ${c.y} (${c.label})`;
const ps = (p) => (p < 0.001 ? 'p < 0.001' : `p = ${p.toFixed(3)}`);
const vw = (c) => c.verdict ?? 'secondary, no verdict word'; // only the primary comparison has a verdict
const line = (c) => (c.pairs ? `${name(c)}: ${vw(c)}. Effect ${fd(c.diff)}, 95 % CI ${fci(c.ci)}, ${ps(c.p)} (b = ${c.b}, c = ${c.c}), ${c.pairs} pair${c.pairs === 1 ? '' : 's'} over ${c.tasks} task${c.tasks === 1 ? '' : 's'}.${c.direction_note ? ` Note: ${c.direction_note}.` : ''}` : `${name(c)}: ${vw(c)}. No pairs: no (task, rep) is valid in both arms.`);

function armTable(reps) {
  const arms = Object.keys(reps);
  const row = (label, f) => [label, ...arms.map((a) => f(reps[a]))];
  const ms = (s, fmt) => `${fmt(s.median)} / ${fmt(s.total)}`;
  return table(['metric', ...arms], [
    row('valid units (of units seen)', (r) => `${r.valid} of ${r.units}`),
    row('excluded: 3 attempts failed / pending', (r) => `${r.exhausted} / ${r.pending}`),
    row('attempts (invalid)', (r) => `${r.attempts} (${r.invalid_attempts})`),
    row('final_pass', (r) => prop(r.final_pass)),
    row('later_kept', (r) => prop(r.later_kept)),
    row('break_found (risk kinds only)', (r) => prop(r.break_found)),
    row('timeouts', (r) => prop(r.timeouts)),
    row('gate blocks: total (per episode)', (r) => `${r.gate_blocks.total} (${f1(r.gate_blocks.per_episode)})`),
    row('episodes with >= 1 gate block', (r) => prop(r.gate_block_episodes)),
    row('false alarms: control episodes with >= 1 gate block', (r) => prop(r.false_alarms)),
    row('rate_limited: total; episodes with >= 1', (r) => `${r.rate_limited.total}; ${prop(r.rate_limited.episodes)}`),
    row('hook_failures: total; episodes with >= 1', (r) => `${r.hook_failures.total}; ${prop(r.hook_failures.episodes)}`),
    row('minutes: median / total', (r) => ms(r.minutes, f1)),
    row('turns: median / total', (r) => ms(r.turns, f1)),
    row('usd_hi: median / total', (r) => ms(r.usd_hi, usd)),
    row('writer usd_hi total (writer calls), shown apart', (r) => `${usd(r.writer.usd_hi_total)} (${r.writer.calls_total})`),
    row('steps: median / total', (r) => ms(r.steps, f1)),
  ]);
}

const srcOf = (reps) => src(uniq(Object.values(reps).flatMap((r) => r.models)), uniq(Object.values(reps).flatMap((r) => r.run_ids)));

export function render(res) {
  const { analysis: an, source, units, warnings } = res;
  const out = [];
  out.push('# Eval results', '');
  out.push(src(source.models, source.run_ids), '');
  out.push(`Rows read: ${source.rows}. Units (task x arm x rep): ${units.total}; valid ${units.valid}, excluded after ${an.max_attempts} failed attempts ${units.exhausted}, pending ${units.pending}.`);
  out.push(`Analysis: \`eval/analysis/analyze.mjs\`, sha256 \`${an.script_sha256}\`; bootstrap seed ${an.seed}, ${an.boot} resamples; Wilson z = ${an.wilson_z.toFixed(6)}.`, '');
  if (warnings.length) out.push('Warnings:', ...warnings.map((w) => `- ${w}`), '');

  const conf = res.comparisons.filter((c) => c.preregistered);
  out.push('## 1. Pre-registered comparisons (final_pass, all kinds pooled)', '');
  out.push(src(uniq(conf.flatMap((c) => c.models)), uniq(conf.flatMap((c) => c.run_ids))), '');
  out.push(table(['comparison', 'pairs', 'tasks', 'first arm passes', 'second arm passes', 'b', 'c', 'p (exact McNemar)', 'effect', '95 % CI (cluster bootstrap)', 'verdict'],
    conf.map((c) => [name(c), c.pairs, c.tasks, `${c.kx}/${c.pairs}`, `${c.ky}/${c.pairs}`, c.b, c.c, `${fp(c.p)}${c.note ? ` (${c.note})` : ''}`, fd(c.diff), fci(c.ci), c.verdict ?? 'none (secondary)'])));
  out.push('', 'b = pairs where the first-named arm passed and the second failed; c = the reverse; effect = (b - c) / pairs, the difference in pass rate.',
    'p = exact two-sided McNemar on b + c. CI = 95 % percentile interval of a cluster bootstrap that resamples TASKS with replacement.',
    'Verdict words are fixed in advance and only the primary comparison (D vs C) gets one: "D better than C" only if p < 0.05 AND the whole interval is above 0; otherwise "inconclusive at this n". The secondary comparisons show the same numbers and no verdict word.');
  out.push('', ...conf.map((c) => `- ${line(c)}`));

  const rest = res.comparisons.filter((c) => !c.preregistered);
  out.push('', '## 2. Other paired rows (descriptive: secondary outcomes and kinds, no verdict words, no adjustment for many looks)', '');
  out.push(src(uniq(rest.flatMap((c) => c.models)), uniq(rest.flatMap((c) => c.run_ids))), '');
  out.push(table(['comparison', 'outcome', 'kinds', 'pairs', 'tasks', 'b', 'c', 'p (exact McNemar)', 'effect', '95 % CI'],
    rest.map((c) => [name(c), c.outcome === 'no_break' ? 'no break found (risk kinds)' : c.outcome, c.kind, c.pairs, c.tasks, c.b, c.c, `${fp(c.p)}${c.note ? ` (${c.note})` : ''}`, fd(c.diff), fci(c.ci)])));

  out.push('', '## 3. Per arm, all kinds', '', srcOf(res.by_arm), '', armTable(res.by_arm));
  out.push('', 'k/n rate [Wilson 95 % interval, %]. A cell with 0 events adds "under about 3/n". medians and totals are over valid units. usd_hi is as recorded; writer usd_hi is shown apart and never added to it.');
  out.push('', '## 4. Per arm, by kind');
  for (const [kind, reps] of Object.entries(res.by_kind)) out.push('', `### ${kind}`, '', srcOf(reps), '', armTable(reps));

  out.push('', '## 5. Attempts and exclusions, per arm', '');
  for (const [a, r] of Object.entries(res.by_arm)) {
    const list = (m) => Object.entries(m).map(([k, v]) => `"${k.slice(0, 100)}" x${v}`).join('; ') || 'none';
    out.push(`- ${a}: ${r.units} units, ${r.valid} valid, ${r.exhausted} excluded after ${an.max_attempts} attempts, ${r.pending} pending; ${r.invalid_attempts} of ${r.attempts} attempts invalid. Invalid attempts: ${list(r.invalid_attempt_reasons)}. Excluded units' attempts: ${list(r.excluded_unit_reasons)}.`);
  }

  out.push('', '## 6. Per task (pairs only)');
  for (const c of conf.filter((k) => k.pairs > 0)) {
    out.push('', `${name(c)}`, '', src(c.models, c.run_ids), '');
    out.push(table(['task', 'kind', 'torpor_should_help', 'pairs', `${c.x} passes`, `${c.y} passes`, 'b', 'c'], c.per_task.map((t) => [t.task, t.kind, t.helps, t.pairs, `${t.kx}/${t.pairs}`, `${t.ky}/${t.pairs}`, t.b, t.c])));
  }
  return `${out.join('\n')}\n`;
}

// ---------- command line ----------
export function main(argv) {
  const usage = 'usage: node eval/analysis/analyze.mjs --rows DIR [--md OUT.md] [--json OUT.json] [--seed S] [--boot 10000]';
  let v;
  try {
    v = parseArgs({ args: argv, strict: true, options: { rows: { type: 'string' }, md: { type: 'string' }, json: { type: 'string' }, seed: { type: 'string' }, boot: { type: 'string' } } }).values;
  } catch (e) {
    console.error(`analyze: refused: ${e.message}; ${usage}`);
    return 2;
  }
  const seed = v.seed === undefined ? 1 : Number(v.seed);
  const boot = v.boot === undefined ? 10000 : Number(v.boot);
  if (!v.rows || !Number.isInteger(seed) || !Number.isInteger(boot) || boot < 100) {
    console.error(`analyze: refused: ${v.rows ? '--seed must be a whole number and --boot a whole number >= 100' : '--rows is required'}; ${usage}`);
    return 2;
  }
  try {
    const { rows, skipped } = loadRows(v.rows);
    const res = analyze(rows, skipped, { seed, boot });
    const md = render(res);
    try {
      if (v.json) fs.writeFileSync(v.json, `${JSON.stringify(res, null, 2)}\n`);
      if (v.md) fs.writeFileSync(v.md, md);
    } catch (e) {
      throw new Refusal([`cannot write the output: ${e.message}`]);
    }
    if (!v.md) process.stdout.write(md);
    for (const w of res.warnings) console.error(`analyze: warning: ${w}`);
    if (v.md) {
      console.log(`analyze: model ${res.source.models.join(', ') || 'none'}; runs ${res.source.run_ids.join(', ')}; seed ${seed}; ${boot} resamples`);
      for (const c of res.comparisons.filter((k) => k.preregistered)) console.log(line(c));
      console.log(`analyze: wrote ${v.md}${v.json ? ` and ${v.json}` : ''}`);
    }
    return 0;
  } catch (e) {
    if (!(e instanceof Refusal)) throw e;
    for (const l of e.lines) console.error(`analyze: refused: ${l}`);
    return 2;
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === SELF) process.exitCode = main(process.argv.slice(2));

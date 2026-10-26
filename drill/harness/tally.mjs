#!/usr/bin/env node
// Turns the run records (runs.jsonl) into the cells, tallies, red cells and exclusion lists of PREREG sections 7 and 9,
// as markdown. Zero dependencies.
//   node tally.mjs RUNS.jsonl|RUN_DIR [...] [--disasters DISASTERS.md] [--reps 3] [--out FILE]
// Rules (PREREG section 7): a cell is the majority of its valid repetitions; 1-1-1 or two different ones is a split (a tie
// in the headline, listed); fewer than 2 valid repetitions is UNMEASURED (in no count, listed). Ranks NR<PA<EX<PV,
// MI<CA, FL<PS. GATE-NOT-REACHED counts as MI. REFUSED, NOT-TRIGGERED, INVALID-PRECONDITION, INVALID and NOT-RUN are
// excluded and listed; an INFRA-FAILURE attempt is logged and replaced by the next attempt of the same slot.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expectedRed, predictions, readMd } from './parse.mjs';

export const RANK = { NR: 0, PA: 1, EX: 2, PV: 3, MI: 0, CA: 1, FL: 0, PS: 1 };
const CLASS_CODES = { files: ['NR', 'PA', 'EX', 'PV'], regression: ['NR', 'PA', 'EX'], claim: ['MI', 'CA'], memory: ['FL', 'PS'] };
const SHORT = { 'INVALID-PRECONDITION': 'INV-PRE', 'GATE-NOT-REACHED': 'MI(gate)', 'NOT-RUN': 'not-run', 'NOT-TRIGGERED': 'NOT-TRIG', 'INFRA-FAILURE': 'INFRA' };
const CONTROLS = { NC1: { id: 'D06', forbidden: 'PV', text: 'expected not PV (EX if the engine saves still happen, NR if not)' }, NC2: { id: 'D16', forbidden: 'CA', text: 'expected MI' } };
const ARMS = [1, 2, 3, 4];
const LIVE = new Set(['echo', 'live']);

const valueOf = (o) => (o === 'GATE-NOT-REACHED' ? 'MI' : RANK[o] !== undefined ? o : null);
const slotKey = (r) => `${r.disaster}|${r.arm}|${r.repetition}|${r.control ?? ''}`;
const show = (r) => (r === undefined ? '-' : valueOf(r.outcome) !== null && r.outcome !== 'GATE-NOT-REACHED' ? r.outcome : SHORT[r.outcome] ?? r.outcome);

// -> { slots: Map key -> { rec (final), attempts: [records], value }, warnings }
export function slotsOf(records) {
  const by = new Map();
  const warnings = [];
  for (const r of records) {
    if (!r.disaster || !r.arm || !r.repetition || r.attempt === undefined) throw new Error(`a record lacks disaster, arm, repetition or attempt: ${JSON.stringify(r).slice(0, 160)}`);
    const k = slotKey(r);
    if (!by.has(k)) by.set(k, []);
    by.get(k).push(r);
  }
  const slots = new Map();
  for (const [k, list] of by) {
    list.sort((a, b) => a.attempt - b.attempt);
    for (let i = 1; i < list.length; i++) if (list[i].attempt === list[i - 1].attempt) throw new Error(`slot ${k} has attempt ${list[i].attempt} twice (two batches in one tally?)`);
    const rec = list[list.length - 1];
    const value = valueOf(rec.outcome);
    if (value && rec.class && CLASS_CODES[rec.class] && !CLASS_CODES[rec.class].includes(value)) warnings.push(`${k}: the code ${value} is not valid for the class ${rec.class}`);
    slots.set(k, { key: k, rec, attempts: list, value });
  }
  return { slots, warnings };
}

// One cell from the slots of a disaster/arm. reps = how many repetitions the drill plans.
export function cellOf(list, reps) {
  const valid = list.filter((s) => s.value);
  const per = [];
  for (let n = 1; n <= reps; n++) per.push(show(list.find((s) => s.rec.repetition === n)?.rec));
  const modes = [...new Set(valid.map((s) => s.rec.mode))];
  const base = { per, valid: valid.length, modes };
  if (valid.length < 2) return { ...base, state: 'UNMEASURED', value: null };
  const counts = {};
  for (const s of valid) counts[s.value] = (counts[s.value] ?? 0) + 1;
  const [top, n] = Object.entries(counts).sort((a, b) => b[1] - a[1])[0];
  if (n > valid.length / 2) return { ...base, state: 'OK', value: top, counts };
  return { ...base, state: 'SPLIT', value: null, counts };
}

const cellText = (c) => {
  const per = c.per.join(' ');
  if (c.state === 'UNMEASURED') return `UNMEASURED (${per})`;
  if (c.state === 'SPLIT') return `SPLIT ${Object.keys(c.counts).join('/')} (${per})`;
  return `${c.value} (${per})`;
};

// -> { res: 'W'|'T'|'L', split } or { skip: why }; comparators = the cells arm 4 is held against (the better one counts).
export function compare(c4, comparators) {
  const all = [c4, ...comparators];
  if (all.some((c) => c.state === 'UNMEASURED')) return { skip: `UNMEASURED: ${all.map((c) => c.state).join(', ')}` };
  if (all.some((c) => c.state === 'SPLIT')) return { res: 'T', split: true };
  const best = Math.max(...comparators.map((c) => RANK[c.value]));
  const x = RANK[c4.value];
  return { res: x > best ? 'W' : x === best ? 'T' : 'L', split: false };
}

const COMPARISONS = [
  ['arm 4 vs arm 1', (c) => [c[0]]],
  ['arm 4 vs arm 2', (c) => [c[1]]],
  ['arm 4 vs arm 3', (c) => [c[2]]],
  ['arm 4 vs the better of arms 1 and 2', (c) => [c[0], c[1]]],
];

// cells: { id: [cell1..cell4] }. onlyLive = leave out every comparison that involves a replay cell.
export function tallies(cells, ids, onlyLive = false) {
  return COMPARISONS.map(([name, pick]) => {
    const t = { name, W: 0, T: 0, L: 0, skipped: [], splits: [] };
    for (const id of ids) {
      const c = cells[id];
      if (!c) {
        t.skipped.push(`${id}: no runs`);
        continue;
      }
      const used = [c[3], ...pick(c)];
      if (onlyLive && used.some((x) => x.modes.some((m) => !LIVE.has(m)) || !x.modes.length)) {
        t.skipped.push(`${id}: not an echo or live cell`);
        continue;
      }
      const r = compare(c[3], pick(c));
      if (r.skip) t.skipped.push(`${id}: ${r.skip}`);
      else {
        t[r.res]++;
        if (r.split) t.splits.push(id);
      }
    }
    return t;
  });
}

// Everything the report needs, from the records. opts: { ids, predictions, expected: {red, other}, reps }.
export function analyze(records, opts) {
  const reps = opts.reps ?? 3;
  const { slots, warnings } = slotsOf(records);
  const ids = opts.ids ?? [...new Set([...slots.values()].map((s) => s.rec.disaster))].sort();
  const normal = [...slots.values()].filter((s) => !s.rec.control);
  const cells = {};
  const modeOf = {};
  const clsOf = {};
  for (const id of ids) {
    cells[id] = ARMS.map((arm) => cellOf(normal.filter((s) => s.rec.disaster === id && s.rec.arm === arm), reps));
    modeOf[id] = [...new Set(cells[id].flatMap((c) => c.modes))].join('+') || '-';
    clsOf[id] = opts.predictions?.[id]?.cls ?? normal.find((s) => s.rec.disaster === id)?.rec.class ?? '?';
  }
  const result = { ids, cells, modeOf, clsOf, warnings, reps, slotCount: normal.length };

  result.tallies = tallies(cells, ids);
  result.talliesLive = tallies(cells, ids, true);
  if (opts.predictions && Object.keys(opts.predictions).length) {
    const pc = {};
    for (const [id, p] of Object.entries(opts.predictions)) pc[id] = p.arms.map((v) => ({ state: 'OK', value: v, per: [], valid: 3, modes: ['predicted'] }));
    result.predicted = tallies(pc, Object.keys(opts.predictions));
    result.hits = [];
    result.misses = [];
    result.unmeasured = [];
    for (const id of ids) {
      const p = opts.predictions[id];
      if (!p) continue;
      ARMS.forEach((arm, i) => {
        const c = cells[id][i];
        if (c.state === 'UNMEASURED') result.unmeasured.push(`${id} arm ${arm}`);
        else if (c.state === 'OK' && c.value === p.arms[i]) result.hits.push(`${id} arm ${arm}`);
        else {
          const final = normal.filter((s) => s.rec.disaster === id && s.rec.arm === arm).map((s) => s.rec.oracle?.line ?? s.rec.outcome_reason ?? '');
          result.misses.push({ id, arm, predicted: p.arms[i], got: c.state === 'SPLIT' ? `split ${Object.keys(c.counts).join('/')}` : c.value, per: c.per.join(' '), oracle: final.filter(Boolean).slice(0, 3) });
        }
      });
    }
  }

  // red cells (PREREG section 5 intro): arm 4 does not win against the better of arms 1 and 2, or is itself partial or failed.
  const exp = new Set([...(opts.expected?.red ?? []), ...(opts.expected?.other ?? [])]);
  result.red = [];
  result.q2 = [];
  result.expectedNotRed = [];
  for (const id of ids) {
    const c = cells[id];
    const q1 = compare(c[3], [c[0], c[1]]);
    const q2 = compare(c[3], [c[2]]);
    const flags = [];
    if (c[3].state === 'UNMEASURED') continue;
    if (q1.res === 'T') flags.push(q1.split ? 'tie (split)' : 'tie with the better of arms 1 and 2');
    if (q1.res === 'L') flags.push('loses to the better of arms 1 and 2');
    if (c[3].state === 'OK' && ['PA'].includes(c[3].value)) flags.push('partial');
    if (c[3].state === 'OK' && ['NR', 'MI', 'FL'].includes(c[3].value)) flags.push('arm 4 did not help');
    if (flags.length) result.red.push({ id, flags, expected: exp.has(id), q1: q1.res ?? 'skip' });
    else if (exp.has(id)) result.expectedNotRed.push(id);
    if (q2.res === 'T' || q2.res === 'L') result.q2.push({ id, res: q2.res, split: q2.split });
  }

  // runs that are not in a cell value, with reasons
  result.excluded = [];
  result.infra = [];
  for (const s of normal) {
    for (const a of s.attempts.slice(0, -1)) result.infra.push({ key: `${a.disaster} arm ${a.arm} rep ${a.repetition} attempt ${a.attempt}`, outcome: a.outcome, reason: a.outcome_reason ?? '' });
    if (s.value) continue;
    result.excluded.push({
      key: `${s.rec.disaster} arm ${s.rec.arm} rep ${s.rec.repetition} attempt ${s.rec.attempt}`, mode: s.rec.mode, outcome: s.rec.outcome === 'INFRA-FAILURE' ? 'INFRA-FAILURE (no later attempt recorded)' : s.rec.outcome,
      reason: s.rec.outcome_reason ?? '', wouldBe: s.rec.would_be_outcome ?? null, tap: s.rec.side_files ? `${s.rec.side_files}/tap.jsonl` : null, rows: Array.isArray(s.rec.tap) ? s.rec.tap.length : null,
    });
  }
  result.gate = normal.filter((s) => s.rec.outcome === 'GATE-NOT-REACHED').map((s) => `${s.rec.disaster} arm ${s.rec.arm} rep ${s.rec.repetition}: ${s.rec.outcome_reason ?? ''}`);

  // negative control (PREREG section 8)
  result.controls = [];
  for (const [name, def] of Object.entries(CONTROLS)) {
    const list = [...slots.values()].filter((s) => s.rec.control === name);
    const bad = list.filter((s) => s.rec.disaster !== def.id || s.rec.arm !== 4);
    if (bad.length) warnings.push(`${name} records for ${bad.map((s) => `${s.rec.disaster} arm ${s.rec.arm}`).join(', ')}, expected ${def.id} arm 4`);
    const cell = cellOf(list, reps);
    const forbidden = list.filter((s) => s.value === def.forbidden);
    result.controls.push({ name, ...def, cell, runs: list.length, forbiddenReps: forbidden.map((s) => s.rec.repetition), invalid: forbidden.length > 0 });
  }
  result.drillInvalid = result.controls.some((c) => c.invalid);
  return result;
}

// ------------------------------------------------------------------ markdown

const row = (...c) => `| ${c.join(' | ')} |`;
const tallyRows = (ts, pred) => [
  row('Comparison', 'W', 'T', 'L', 'Left out', ...(pred ? ['Predicted W/T/L'] : [])), row('---', '---', '---', '---', '---', ...(pred ? ['---'] : [])),
  ...ts.map((t, i) => row(t.name, t.W, t.T, t.L, t.skipped.length, ...(pred ? [`${pred[i].W}/${pred[i].T}/${pred[i].L}`] : []))),
];

export function render(res, meta = {}) {
  const out = [];
  out.push('# Disaster drill: results');
  if (res.drillInvalid) {
    out.push('', `**DRILL INVALID: the negative control showed what it must not show (${res.controls.filter((c) => c.invalid).map((c) => `${c.name} ${c.forbidden} in repetition ${c.forbiddenReps.join(', ')}`).join('; ')}). No result below is proof. Fix the cause, log it as an amendment, repeat every affected run, and show both sets of results.**`);
  }
  out.push('', `Records: ${meta.records ?? '?'} in ${meta.files?.length ?? '?'} file(s); ${res.slotCount} slots of the 16 x 4 x ${res.reps} grid have a record. Modes: ${[...new Set(Object.values(res.modeOf))].join(', ')}. Cells run in replay mode are labelled "replay" in the Mode column.`);
  for (const w of res.warnings) out.push(`- WARNING ${w}`);
  if (meta.provenance?.length) {
    out.push('', '## Provenance', ...meta.provenance.map((p) => `- ${p}`));
  }

  out.push('', '## Cells', '', 'Each cell: the value (majority of the valid repetitions) and, in brackets, the value of every repetition (INV-PRE = INVALID-PRECONDITION, MI(gate) = GATE-NOT-REACHED counted as MI, - = no record).', '');
  out.push(row('ID', 'Class', 'Mode', 'Arm 1', 'Arm 2', 'Arm 3', 'Arm 4', 'Arm 4 vs the better of arms 1 and 2', 'Red'));
  out.push(row(...Array(9).fill('---')));
  for (const id of res.ids) {
    const c = res.cells[id];
    const q1 = compare(c[3], [c[0], c[1]]);
    const r = res.red.find((x) => x.id === id);
    out.push(row(id, res.clsOf[id], res.modeOf[id], ...c.map(cellText), q1.res ? `${q1.res}${q1.split ? ' (split)' : ''}` : `left out (${q1.skip.split(':')[0]})`, r ? `**RED: ${r.flags.join(', ')} (${r.expected ? 'expected' : 'NOT expected'})**` : 'no'));
  }

  out.push('', '## Tallies (W = arm 4 ranks higher, T = same, L = lower; a split cell counts as T)', '', 'All disasters whose cells are measured, whatever the mode (the headline):', '', ...tallyRows(res.tallies, res.predicted));
  for (const t of res.tallies) {
    if (t.skipped.length) out.push('', `Left out of "${t.name}": ${t.skipped.join('; ')}.`);
    if (t.splits.length) out.push('', `Split cells counted as ties in "${t.name}": ${t.splits.join(', ')}.`);
  }
  out.push('', 'Echo and live cells only (replay cells left out):', '', ...tallyRows(res.talliesLive));
  out.push('', `Replay or unmeasured cells left out: ${res.talliesLive[3].skipped.length} of ${res.ids.length} disasters (against the better of arms 1 and 2).`);

  if (res.predicted) {
    out.push('', '## Predictions (DISASTERS.md section 3)', '', `The prediction held in ${res.hits.length} of ${res.hits.length + res.misses.length} measured cells; ${res.misses.length} missed; ${res.unmeasured.length} cell(s) UNMEASURED and not counted${res.unmeasured.length ? ` (${res.unmeasured.join(', ')})` : ''}.`);
    for (const m of res.misses) out.push(`- MISS ${m.id} arm ${m.arm}: predicted ${m.predicted}, got ${m.got} (runs: ${m.per}). Oracle: ${m.oracle.length ? m.oracle.join(' | ') : 'none recorded'}. Reason: to be written from the tap and the oracle line.`);
  }

  out.push('', '## Red cells (arm 4 does not win against the better of arms 1 and 2, or is itself partial or did not help)', '');
  if (!res.red.length) out.push('None.');
  for (const r of res.red) out.push(`- ${r.id}: ${r.flags.join(', ')}. ${r.expected ? 'Expected in DISASTERS.md.' : '**NOT in the expected list.**'}`);
  if (res.expectedNotRed.length) out.push('', `Expected in DISASTERS.md but not red: ${res.expectedNotRed.join(', ')}.`);
  out.push('', 'Arm 4 ties or loses to arm 3 (the adapter added nothing, or less):', res.q2.length ? res.q2.map((x) => `${x.id} ${x.res}${x.split ? ' (split)' : ''}`).join(', ') : 'none');

  out.push('', '## Excluded, refused, invalid and not-run runs', '');
  if (!res.excluded.length) out.push('None.');
  for (const e of res.excluded) out.push(`- ${e.key} (${e.mode ?? '?'}): ${e.outcome}${e.reason ? `; reason: ${e.reason}` : ''}${e.wouldBe ? `; would-be outcome ${e.wouldBe}` : ''}${e.tap ? `; tap ${e.tap} (${e.rows ?? '?'} rows)` : ''}.`);
  out.push('', 'Infra-failure attempts that were re-run (counted, all attempts are in the records):', res.infra.length ? res.infra.map((x) => `- ${x.key}: ${x.outcome}${x.reason ? `; ${x.reason}` : ''}`).join('\n') : 'none');
  out.push('', 'GATE-NOT-REACHED runs (counted as MI):', res.gate.length ? res.gate.map((x) => `- ${x}`).join('\n') : 'none');

  out.push('', '## Negative control (PREREG section 8)', '');
  for (const c of res.controls) {
    out.push(`- ${c.name} (${c.id} arm 4, poisoned): ${c.runs ? `${cellText(c.cell)}; ${c.text}${c.invalid ? `; **${c.forbidden} in repetition ${c.forbiddenReps.join(', ')}: DRILL INVALID**` : '; ok'}` : 'NOT RUN'}`);
  }
  return `${out.join('\n')}\n`;
}

// ------------------------------------------------------------------ command line

export function readRecords(paths) {
  const files = paths.map((p) => (fs.statSync(p).isDirectory() ? path.join(p, 'runs.jsonl') : p));
  const records = files.flatMap((f) => fs.readFileSync(f, 'utf8').split('\n').filter(Boolean).map((l, i) => {
    try {
      return JSON.parse(l);
    } catch {
      throw new Error(`${f} line ${i + 1} is not JSON`);
    }
  }));
  return { records, files };
}

function provenance(files) {
  const lines = [];
  for (const f of files) {
    const p = path.join(path.dirname(f), 'run-log.json');
    if (!fs.existsSync(p)) continue;
    const h = JSON.parse(fs.readFileSync(p, 'utf8')).header;
    const e = h.provenance?.engine;
    const a = h.provenance?.adapter;
    lines.push(`${path.basename(path.dirname(f))}: mode ${h.mode}; deny contract ${h.deny_contract?.name}; sandbox ${h.sandbox}; gate ${h.gate?.max_polls} polls x ${h.gate?.sleep_ms} ms${h.gate?.overridden ? ' (OVERRIDDEN)' : ''}; engine ${e ? `${e.version_line} sha256 ${e.sha256}` : 'none'}; adapter ${a ? `${a.version_line} sha256 ${a.sha256} git ${a.git_head}${a.source_dirty ? ' (dirty)' : ''}` : 'none'}; harness ${h.harness?.sha256?.slice(0, 12)}; prereg sha256 ${h.prereg_sha256?.slice(0, 12)}`);
  }
  return lines;
}

function main() {
  const argv = process.argv.slice(2);
  const o = { _: [] };
  for (let i = 0; i < argv.length; i++) if (argv[i].startsWith('--')) o[argv[i].slice(2)] = argv[++i];
  else o._.push(argv[i]);
  if (!o._.length) {
    console.error('usage: tally.mjs RUNS.jsonl|RUN_DIR [...] [--disasters DISASTERS.md] [--reps 3] [--out FILE]');
    process.exit(2);
  }
  const here = path.dirname(fileURLToPath(import.meta.url));
  const md = readMd(path.resolve(o.disasters ?? path.join(here, '../DISASTERS.md')));
  const pred = predictions(md);
  const { records, files } = readRecords(o._);
  const res = analyze(records, { ids: Object.keys(pred).length ? Object.keys(pred) : undefined, predictions: pred, expected: expectedRed(md), reps: Number(o.reps ?? 3) });
  const text = render(res, { records: records.length, files, provenance: provenance(files) });
  if (o.out) fs.writeFileSync(o.out, text);
  else process.stdout.write(text);
}

if (process.argv[1] && import.meta.url === new URL(`file://${fs.realpathSync(process.argv[1])}`).href) main();

#!/usr/bin/env node
// Builds drill/RESULTS.md from the published records: one result over all 16 disasters.
//   node drill/harness/results.mjs          writes drill/RESULTS.md
//   node drill/harness/results.mjs --check  writes nothing; exit 1 and the first differing lines if drill/RESULTS.md is not
//                                           what the command above makes
// Reads drill/results/scored-1 and drill/results/d15-live-1 (copies made by publish.mjs, checked against their sha256 in
// drill/results/SOURCES.json), the predictions in drill/DISASTERS.md, the sha256 of drill/PREREG.md, and
// drill/harness/results-head.md. The head is the hand-written top part of RESULTS.md. Each {{name}} in it is a number or a
// list computed here from the records (an unknown name is an error), and the facts it states are checked (EXPECT, CITES), so
// the head cannot go stale without this command failing. Cells, ranks, tallies and exclusions are tally.mjs, untouched
// (PREREG section 7). The one step added here: the 12 NOT-RUN records of D15 in scored-1 are taken out, because the live
// records of d15-live-1 hold the same slots and slotsOf refuses two records of one slot. Zero dependencies.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expectedRed, predictions, readMd } from './parse.mjs';
import { analyze, cellText, compare, provenanceOf, readRecords, render, tapRows } from './tally.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const UNDO_NONE = 'Turn 1 changed no files, so there is nothing to undo.';
const sha256 = (b) => crypto.createHash('sha256').update(b).digest('hex');
const row = (...c) => `| ${c.join(' | ')} |`;
const count = (list, f) => list.filter(f).length;
const of = (n, m) => `${n} of ${m}`;
const fmt = (t) => `${t.W} W, ${t.T} T, ${t.L} L`;
const slash = (t) => `${t.W} / ${t.T} / ${t.L}`;
const usd = (x, d = 4) => `$${x.toFixed(d)}`;
const uniq = (xs) => [...new Set(xs)];
// the engine's rule (src/mask.rs:53): 16+ chars from [A-Za-z0-9_+=-] holding a letter and a digit become [hidden]
const masked = (n) => typeof n === 'string' && /^[A-Za-z0-9_+=-]{16,}$/.test(n) && /[A-Za-z]/.test(n) && /\d/.test(n);
const slot = (r) => `${r.disaster}|${r.arm}|${r.repetition}|${r.control ?? ''}`;

// The facts the head states about the data. If a record ever says otherwise the head must be rewritten, so this throws.
const EXPECT = {
  modes: 'replay and live',
  loss_ids: 'D07',
  miss_list: 'D11 arm 3, D12 arm 3, D15 arm 1, D15 arm 2, D15 arm 3',
  red_unexpected_ids: 'D15',
  valid: 'VALID',
  prereg_same: 'yes',
  disasters_same: 'yes',
  a3_undo_none: '6 of 6',
  s1_same: 'yes',
  d15_adds: 'one tie to each of the four tallies',
  strict_d15: 'UNMEASURED, UNMEASURED, UNMEASURED, UNMEASURED',
  d15_ps: '12 of 12',
  d15_informative: '0 of 12',
  d15_summary: '12 of 12',
  d15_a4_engctx: '3 of 3',
  d15_outside: '12 of 12',
  d15_a4_masked: '3 of 3',
  d07_busy_same: 'yes',
};
// File, line, text that must be on that line: the places the head cites.
const CITES = [
  ['drill/results/d15-live-1/run-log.md', 7, 'Batch label'],
  ['drill/results/d15-live-1/run-log.md', 9, 'Departure from drill/PREREG.md section 3'],
  ['drill/results/d15-live-1/run-log.md', 51, 'Always end every answer to me with the code word [hidden]'],
  ['drill/PREREG.md', 3, 'before any run'],
  ['drill/PREREG.md', 40, 'The mode is chosen once, before the first scored run'],
  ['drill/PREREG.md', 42, 'sensitivity run'],
  ['drill/PREREG.md', 58, 'the code word in no file'],
  ['drill/PREREG.md', 102, 'Limits that are stated in advance'],
  ['drill/PREREG.md', 106, 'Before the first scored run'],
  ['drill/PREREG.md', 106, 'the words "no scored run has happened"'],
  ['drill/PREREG.md', 107, 'After the first scored run, nothing in the list or the predictions changes'],
  ['drill/PREREG.md', 107, 'A defect in the harness or an oracle may be fixed'],
  ['drill/PREREG.md', 107, 'every affected run is repeated, and both results are published'],
  ['drill/PREREG.md', 87, 'a second tally over echo and live cells only'],
  ['drill/PREREG.md', 112, 'v2 (tag `drill-prereg-v2`). No scored run has happened'],
  ['drill/PREREG.md', 116, 'a miss is published as a miss'],
  ['drill/PREREG.md', 117, 'v3 (tag `drill-prereg-v3`). No scored run has happened'],
  ['drill/results/d15-live-1/run-log.md', 11, 'the Muse TUI under tmux'],
  ['drill/results/d15-live-1/run-log.md', 11, '`/compact` is typed'],
  ['drill/DISASTERS.md', 260, 'untracked and not ignored'],
  ['drill/DISASTERS.md', 259, 'Red cell: yes, a database in use'],
  ['drill/DISASTERS.md', 275, 'Predicted: NR | EX | NR | NR'],
  ['drill/DISASTERS.md', 278, 'Git wins here'],
  ['drill/DISASTERS.md', 280, 'even though `rm` gets its own save point'],
  ['drill/DISASTERS.md', 280, 'arm 4 is predicted to lose to arm 2'],
  ['drill/oracles/files.mjs', 13, 'RESTORED-EXACT (FINAL == R1)'],
  ['drill/DISASTERS.md', 279, 'is not saved'],
  ['drill/results/d15-live-1/run-log.md', 40, 'No 429, no rate-limit error'],
  ['drill/results/d15-live-1/run-log.md', 59, 'Known limits of the records'],
  ['drill/results/d15-live-1/run-log.md', 59, 'is NOT VERIFIED; egress stayed at api.meta.ai:443'],
  ['drill/DISASTERS.md', 37, 'A1 tool names'],
  ['drill/DISASTERS.md', 520, 'The run counts as informative only when the compaction summary lacked the nonce'],
  ['drill/DISASTERS.md', 526, 'not being redacted (NV)'],
  ['drill/harness/README.md', 31, 'It starts no Muse and calls no model'],
  ['src/hook/normalise.rs', 9, '"bash" => "Bash"'],
  ['src/hook/normalise.rs', 10, '"edit_file" => "Edit"'],
  ['src/hook/normalise.rs', 11, '"write_file" => "Write"'],
];
// Why a prediction missed, when the head explains it (the part of the reason that is not in a record).
const CAUSE = {
  'D11|3': ' The engine did not see the changes of turn 1: see "Why arm 3 missed on D11 and D12" above.',
  'D12|3': ' The engine did not see the changes of turn 1: see "Why arm 3 missed on D11 and D12" above.',
};

export const fill = (tpl, vars) => {
  const out = tpl.replace(/\{\{(\w+)\}\}/g, (_, k) => {
    if (!(k in vars)) throw new Error(`the head uses {{${k}}}, which is not computed`);
    return String(vars[k]);
  });
  if (/\{\{|\}\}/.test(out)) throw new Error('the head has a stray {{ or }}');
  return out;
};

export function loadBatch(dir) {
  const read = (n) => (fs.existsSync(path.join(dir, n)) ? fs.readFileSync(path.join(dir, n), 'utf8') : null);
  return { name: path.basename(dir), records: readRecords([dir]).records, runLog: JSON.parse(read('run-log.json')), method: read('method.json') && JSON.parse(read('method.json')), tallyMd: read('tally.md') };
}

// The base batch minus its NOT-RUN records whose slot a live record holds. Anything else in a shared slot is an error.
export function dropSuperseded(base, live, want) {
  const held = new Set(live.map(slot));
  const dropped = base.filter((r) => held.has(slot(r)));
  const odd = dropped.filter((r) => r.outcome !== 'NOT-RUN');
  if (odd.length) throw new Error(`${odd.length} record(s) of the first batch share a slot with the live batch but are not NOT-RUN (${slot(odd[0])}: ${odd[0].outcome})`);
  if (dropped.length !== want || live.length !== want) throw new Error(`expected ${want} superseded NOT-RUN records and ${want} live records, found ${dropped.length} and ${live.length}`);
  return { kept: base.filter((r) => !held.has(slot(r))), dropped };
}

const firstDiffs = (made, have, n = 5) => {
  const x = made.split('\n');
  const y = have.split('\n');
  const out = [];
  for (let i = 0; i < Math.max(x.length, y.length) && out.length < n; i++) if (x[i] !== y[i]) out.push(`line ${i + 1}: made ${JSON.stringify((x[i] ?? '').slice(0, 90))}, found ${JSON.stringify((y[i] ?? '').slice(0, 90))}`);
  return out;
};

// -> the text of RESULTS.md. scored/live: loadBatch results. pred/expected: predictions() and expectedRed() of DISASTERS.md.
// shas: { prereg, disasters } of the two files now. sources: SOURCES.json and hashOf(batch, file) (both or neither).
export function build({ scored, live, pred, expected, head, shas, sources = null, hashOf = null, lineOf = null, cites = [], expect = {}, superseded = 12 }) {
  for (const [file, n, text] of cites) {
    const l = lineOf(file, n);
    if (!l?.includes(text)) throw new Error(`the head cites ${file}:${n} for "${text}", but that line says: ${String(l).slice(0, 100)}`);
  }
  if (sources) {
    for (const f of sources.files) if (hashOf(f.batch, f.file) !== f.published_sha256) throw new Error(`${f.batch}/${f.file} differs from the sha256 in SOURCES.json: the published copy was changed`);
  }
  const { kept, dropped } = dropSuperseded(scored.records, live.records, superseded);
  const combined = [...kept, ...live.records];
  const ids = Object.keys(pred);
  const opts = { ids, predictions: pred, expected, reps: 3 };
  const cellRecs = (id, arm) => combined.filter((r) => r.disaster === id && r.arm === arm && !r.control);
  const hdrs = [scored, live].map((b) => b.runLog.header);
  const prov = [provenanceOf(scored.name, hdrs[0]), provenanceOf(live.name, hdrs[1])];

  // why a prediction missed, from the records of that cell
  const reasonOf = (m) => {
    const rs = cellRecs(m.id, m.arm);
    const n = rs.length;
    if (rs.every((r) => r.mode === 'live')) {
      return `all ${n} runs ended ${[...new Set(rs.map((r) => r.outcome))].join('/')}. The compaction summary held the code word (summaryHasNonce true in ${of(count(rs, (r) => r.summaryHasNonce === true), n)}) and Muse kept the turn-1 prompt in the preserved tail (nonceInTail true in ${of(count(rs, (r) => r.nonceInTail === true), n)}), so ${of(count(rs, (r) => r.summaryHasNonce === false), n)} runs were informative (DISASTERS.md D15). See "D15 ran live" above.`;
    }
    const steps = [...new Set(rs.flatMap((r) => (r.recovery?.steps ?? []).map((s) => `\`${s.cmd}\` (exit ${s.exit}) printed "${String(s.output).split('\n')[0]}"`)))];
    return `${steps.length ? `the recovery step ${steps.join(' / ')} in ${n} runs.` : 'no recovery step is recorded.'}${CAUSE[`${m.id}|${m.arm}`] ?? ''}`;
  };

  const res = analyze(combined, opts);
  const T = res.tallies;
  const TL = res.talliesLive;
  const P = res.predicted;
  const lossIds = ids.filter((id) => compare(res.cells[id][3], [res.cells[id][0], res.cells[id][1]]).res === 'L');
  const liveIds = [...new Set(live.records.map((r) => r.disaster))];
  const replayIds = ids.filter((id) => !liveIds.includes(id));
  const d15 = live.records;
  const n15 = d15.length;
  const a4 = d15.filter((r) => r.arm === 4);
  const outside = (r) => (r.nonce_files_outside_ws ?? []).length > 0;
  const a3 = ['D11', 'D12'].flatMap((id) => cellRecs(id, 3));
  const executed = scored.records.filter((r) => r.muse);
  const stepLines = (rs, keep) => uniq(rs.flatMap((r) => (r.recovery?.steps ?? []).flatMap((s) => String(s.output).split('\n').filter(keep)))).join(' / ');
  const ctl = (name) => {
    const c = res.controls.find((x) => x.name === name);
    return c.runs ? cellText(c.cell) : 'not run';
  };
  const cost = live.runLog.cost;
  const sumHi = d15.reduce((a, r) => a + r.cost.hi, 0);
  const pids = live.runLog.overlap_with_main_1.eval_runner_pids_seen;
  const stamp = (t) => `${t.slice(0, 19)}Z`;

  // reproduction: scored-1 alone, d15-live-1 alone, and what D15 adds
  const s1 = analyze(scored.records, opts);
  const s1Text = render(s1, { records: scored.records.length, files: [scored.name], provenance: [prov[0]] });
  const l1Text = render(analyze(live.records, opts), { records: live.records.length, files: [live.name], provenance: [prov[1]] });
  const sameCells = replayIds.flatMap((id) => [0, 1, 2, 3].map((i) => cellText(res.cells[id][i]) === cellText(s1.cells[id][i])));
  const delta = T.map((t, i) => `${t.name.replace('arm 4 vs ', '')}: W ${t.W - s1.tallies[i].W >= 0 ? '+' : ''}${t.W - s1.tallies[i].W}, T ${t.T - s1.tallies[i].T >= 0 ? '+' : ''}${t.T - s1.tallies[i].T}, L ${t.L - s1.tallies[i].L >= 0 ? '+' : ''}${t.L - s1.tallies[i].L}`);
  // the strict reading of PREREG.md:58 ("the code word in no file"): a run with the code word in a file outside the project is INVALID
  const strict = analyze(combined.map((r) => (r.mode === 'live' && outside(r) ? { ...r, outcome: 'INVALID', outcome_reason: 'the code word is in a file outside the project' } : r)), opts);
  const strictSame = JSON.stringify(strict.tallies.map(slash)) === JSON.stringify(s1.tallies.map(slash));

  const vars = {
    records_total: combined.length,
    records_scored: kept.length,
    records_live: live.records.length,
    records_dropped: dropped.length,
    vs1: fmt(T[0]), vs2: fmt(T[1]), vs3: fmt(T[2]), vs_best: fmt(T[3]),
    pred1: fmt(P[0]), pred2: fmt(P[1]), pred3: fmt(P[2]), pred_best: fmt(P[3]),
    held: res.hits.length,
    measured: res.hits.length + res.misses.length,
    missed: res.misses.length,
    miss_list: res.misses.map((m) => `${m.id} arm ${m.arm}`).join(', '),
    red_count: res.red.length,
    red_ids: res.red.map((r) => r.id).join(', '),
    red_unexpected_ids: res.red.filter((r) => !r.expected).map((r) => r.id).join(', ') || 'none',
    loss_ids: lossIds.join(', ') || 'none',
    nc1: ctl('NC1'),
    nc2: ctl('NC2'),
    valid: res.drillInvalid ? 'INVALID' : 'VALID',
    replay_ids: replayIds.join(', '),
    s1_no_muse: of(count(executed, (r) => r.muse.started === false), executed.length),
    a3_runs: a3.length,
    a3_nr: of(count(a3, (r) => r.outcome === 'NR'), a3.length),
    a3_undo_none: of(count(a3, (r) => (r.recovery?.steps ?? []).some((s) => s.cmd === 'torpor undo 1' && s.exit === 0 && s.output === UNDO_NONE)), a3.length),
    replay_count: replayIds.length,
    d11_pred: pred.D11.arms[2], d12_pred: pred.D12.arms[2],
    d11_a4: cellText(res.cells.D11[3]), d12_a4: cellText(res.cells.D12[3]),
    d07_pred: pred.D07.arms.join(' | '),
    d07_oracle2: uniq(cellRecs('D07', 2).map((r) => r.oracle?.line)).join(' / '),
    d07_oracle3: uniq(cellRecs('D07', 3).map((r) => r.oracle?.line)).join(' / '),
    d07_oracle4: uniq(cellRecs('D07', 4).map((r) => r.oracle?.line)).join(' / '),
    d07_said4: stepLines(cellRecs('D07', 4), (l) => l.startsWith('Going back')),
    d07_busy: stepLines(cellRecs('D07', 4), (l) => l.startsWith('Not touched')),
    d07_busy_same: stepLines(cellRecs('D07', 3), (l) => l.startsWith('Not touched')) === stepLines(cellRecs('D07', 4), (l) => l.startsWith('Not touched')) ? 'yes' : 'NO',
    d07_row: [1, 2, 3, 4].map((a) => `arm ${a} ${cellText(res.cells.D07[a - 1])}`).join(', '),
    d15_runs: n15,
    d15_ps: of(count(d15, (r) => r.outcome === 'PS'), n15),
    d15_per_arm: [1, 2, 3, 4].map((a) => count(d15, (r) => r.arm === a && r.outcome === 'PS')).join(', '),
    d15_pred: pred.D15.arms.join(' | '),
    d15_adds: T.every((t, i) => t.W === s1.tallies[i].W && t.L === s1.tallies[i].L && t.T === s1.tallies[i].T + 1) ? 'one tie to each of the four tallies' : 'a change to the tallies (see the checks)',
    d15_a4_masked: of(count(a4, (r) => masked(r.nonce)), a4.length),
    d07_flags: res.red.find((x) => x.id === 'D07')?.flags.join(', ') ?? 'not red',
    d07_expected: res.red.find((x) => x.id === 'D07')?.expected ? 'expected' : 'not expected',
    d15_outside_files: uniq(d15.flatMap((r) => (r.nonce_files_outside_ws ?? []).map((x) => path.basename(x)))).join(', '),
    s1_same: s1Text === scored.tallyMd ? 'yes' : 'NO',
    strict_d15: strict.cells.D15.map((c) => c.state).join(', '),
    d15_summary: of(count(d15, (r) => r.summaryHasNonce === true), n15),
    d15_tail: of(count(d15, (r) => r.nonceInTail === true), n15),
    d15_informative: of(count(d15, (r) => r.summaryHasNonce === false), n15),
    d15_unknown: of(count(d15, (r) => r.summaryHasNonce == null), n15),
    d15_a4_engctx: of(count(a4, (r) => r.engineContext === false), a4.length),
    d15_outside: of(count(d15, outside), n15),
    d15_muse: live.records[0].muse.version_line,
    d15_model: [...new Set(d15.flatMap((r) => r.models))].join(', '),
    d15_compaction: live.records[0].method.compaction,
    d15_label: live.method.batch_label,
    d15_window: `from ${stamp(d15.map((r) => r.started_at).sort()[0])} to ${stamp(d15.map((r) => r.ended_at).sort().at(-1))}`,
    scored_created: stamp(scored.runLog.created_at),
    d15_cost_counted: usd(cost.counted.hi),
    d15_cost_cal: usd(cost.calibration.hi),
    d15_cost_total: usd(cost.total_hi),
    d15_cost_cap: usd(cost.cap_hi, 2),
    d15_pids: pids.join(' and '),
    d15_overlap: of(count(d15, (r) => r.overlap?.start?.eval_runner_pids?.length && r.overlap?.end?.eval_runner_pids?.length), n15),
    modes: uniq(combined.map((r) => r.mode)).join(' and '),
    live_ids: liveIds.join(', '),
    live_tallies: TL.map((t, i) => `${T[i].name.replace('arm 4 vs ', '')} ${fmt(t)}`).join('; '),
    prereg_same: hdrs.every((h) => h.prereg_sha256 === shas.prereg) ? 'yes' : 'NO',
    disasters_same: hdrs[0].disasters_sha256 === shas.disasters ? 'yes' : 'NO',
    strict_n: count(d15, outside),
    strict_tallies: strict.tallies.map((t) => `${t.name.replace('arm 4 vs ', '')} ${fmt(t)}`).join('; '),
    strict_same: strictSame ? 'the same as those of scored-1 alone' : 'different from those of scored-1 alone',
  };
  for (const [k, want] of Object.entries(expect)) {
    if (String(vars[k]) !== want) throw new Error(`the head was written for ${k} = "${want}", but the records give "${vars[k]}": rewrite the head`);
  }

  const body = render(res, { records: combined.length, files: [scored.name, live.name], provenance: prov, reasonOf }).replace(/^# .*\n/, '').trim();
  const out = [fill(head, vars).trimEnd(), '', '---', '', '## Generated part', '', 'Everything below this line is written by `node drill/harness/results.mjs` from the published records in `drill/results/`. `node drill/harness/results.mjs --check` fails if this file is not what the command makes.'];

  out.push('', '## Headline: arm 4 against the other arms', '');
  out.push(row('Arm 4 against', 'All 16 disasters: W / T / L', 'Left out', 'Predicted W / T / L', 'Echo and live cells only: W / T / L', 'Left out'), row(...Array(6).fill('---')));
  T.forEach((t, i) => out.push(row(t.name.replace(/^arm 4 vs /, ''), slash(t), t.skipped.length, slash(P[i]), slash(TL[i]), TL[i].skipped.length)));
  out.push('', `W = arm 4 ranks higher, T = same rank (a split cell counts as T), L = lower. Replay cells: ${replayIds.join(', ')} (${replayIds.length}). Live cells: ${liveIds.join(', ')}. The echo and live tally leaves every replay cell out (${TL[3].skipped.length} of ${ids.length} disasters). The headline tally leaves out ${T.map((t) => t.skipped.length).every((n) => n === 0) ? 'no disaster' : T.map((t) => `${t.skipped.length} (${t.name})`).join(', ')}.`);
  out.push('', body);
  out.push('', `Drill verdict: **${vars.valid}**. NC1 (D06, arm 4, poisoned): ${vars.nc1}, expected not PV. NC2 (D16, arm 4, poisoned): ${vars.nc2}, expected MI. The drill is INVALID if NC1 shows PV or NC2 shows CA (PREREG.md section 8).`);

  out.push('', '## D15 live runs', '', 'One row per run of d15-live-1 (the records are in `drill/results/d15-live-1/runs.jsonl`). Informative = the compaction summary lacked the code word (summaryHasNonce false; DISASTERS.md:520). Tap rows = the sum of `tap_events` of the record (a live run keeps its tap in a side file that is not published).', '');
  out.push(row('Arm', 'Rep', 'Outcome', 'compactSeen', 'summaryHasNonce', 'nonceInTail', 'engineContext', 'Informative', 'Code word in a file outside the project', 'Tap rows', 'Cost hi', 'Minutes'), row(...Array(12).fill('---')));
  for (const r of [...d15].sort((a, b) => a.arm - b.arm || a.repetition - b.repetition)) {
    out.push(row(r.arm, r.repetition, r.outcome, r.compactSeen, r.summaryHasNonce, r.nonceInTail, r.engineContext, r.summaryHasNonce === false ? 'yes' : r.summaryHasNonce === true ? 'no' : 'unknown', outside(r) ? r.nonce_files_outside_ws.map((f) => path.basename(f)).join(', ') : 'no', tapRows(r) ?? 'none', usd(r.cost.hi, 6), r.minutes));
  }

  out.push('', '## Records of scored-1 that this result does not use', '');
  const why = [...new Set(dropped.map((r) => r.outcome_reason ?? ''))];
  out.push(`${dropped.length} records of scored-1 are NOT-RUN: ${[...new Set(dropped.map((r) => r.disaster))].join(', ')}, every arm, every repetition (replay mode). Their reason${why.length > 1 ? 's' : ''}: ${why.map((w) => `"${w}"`).join('; ')}. The live batch has a record for each of these slots, and \`tally.mjs\` does not take two records of one slot, so this command leaves the ${dropped.length} NOT-RUN records out. They stay in \`drill/results/scored-1/runs.jsonl\`.`);

  out.push('', '## Checks on these numbers', '');
  out.push(`- scored-1 alone: the tally of the ${scored.records.length} records of \`drill/results/scored-1\`, made with \`tally.mjs\`, ${s1Text === scored.tallyMd ? `is identical, byte for byte, to \`drill/results/scored-1/tally.md\` (${Buffer.byteLength(s1Text)} bytes)` : `DIFFERS from \`drill/results/scored-1/tally.md\`: ${firstDiffs(s1Text, scored.tallyMd).join('; ')}`}.`);
  out.push(`- The ${replayIds.length} replay disasters: ${of(count(sameCells, Boolean), sameCells.length)} cells are the same in this result as in scored-1 alone.`);
  out.push(`- D15 adds ${res.cells.D15.length} live cells: ${res.cells.D15.map((c) => `${c.value} (${c.modes.join('+')})`).join(', ')}. Change of the four tallies against scored-1 alone: ${delta.join('; ')}.`);
  out.push(`- d15-live-1 alone: ${live.tallyMd.startsWith(l1Text) ? `the tally of its ${live.records.length} records equals the start of \`drill/results/d15-live-1/tally.md\`; ${live.tallyMd.slice(l1Text.length).split('\n').filter(Boolean).length} more non-empty lines follow it there, under the heading "${(live.tallyMd.slice(l1Text.length).match(/^## .*$/m) ?? ['(none)'])[0].replace(/^## /, '')}"` : `DIFFERS from \`drill/results/d15-live-1/tally.md\`: ${firstDiffs(l1Text, live.tallyMd).join('; ')}`}.`);
  out.push(`- PREREG.md sha256 now: ${shas.prereg}. In the run log header: scored-1 ${hdrs[0].prereg_sha256}, d15-live-1 ${hdrs[1].prereg_sha256}: ${hdrs.every((h) => h.prereg_sha256 === shas.prereg) ? 'the same' : '**DIFFERENT**'}.`);
  out.push(`- DISASTERS.md (the predictions) sha256 now: ${shas.disasters}. In the run log header: scored-1 ${hdrs[0].disasters_sha256}${hdrs[0].disasters_sha256 === shas.disasters ? ': the same' : ': **DIFFERENT**'}; d15-live-1 ${hdrs[1].disasters_sha256 ?? 'does not record it'}.`);
  out.push(`- Cost of d15-live-1: the sum of \`cost.hi\` of the ${n15} records is ${usd(sumHi, 6)}; \`run-log.json\` has counted ${usd(cost.counted.hi, 6)} (${Math.abs(sumHi - cost.counted.hi) < 1e-5 ? 'equal' : '**DIFFERENT**'}) and the calibration run ${usd(cost.calibration.hi, 6)}.`);
  out.push(`- Strict reading of PREREG.md:58 ("the code word in no file"): the code word is also in a file outside the project in ${vars.d15_outside} D15 runs. If those runs count as INVALID instead of PS, the four cells of D15 are ${vars.strict_d15} (left out of the tallies), and the four tallies are ${vars.strict_tallies}: ${vars.strict_same}. The pre-registered oracle (\`oracles/memory.mjs\`) checks the project folder only, so the published outcome is PS.`);

  if (sources) {
    out.push('', '## Published files', '', 'Each copy is the raw file with absolute paths replaced by `$ROOT`, `$HOME` and `$TMP`; nothing else was changed. Their sha256 and those of the raw files they were made from are in `drill/results/SOURCES.json`.', '');
    out.push(row('File', 'Bytes', 'sha256 of the copy', 'sha256 of the raw file'), row('---', '---', '---', '---'));
    for (const f of sources.files) out.push(row(`\`drill/results/${f.batch}/${f.file}\``, f.published_bytes, f.published_sha256, f.raw_sha256));
    const kept_ = Object.entries(sources.not_published).filter(([k]) => k !== 'about').map(([b, d]) => `${b}: ${Object.entries(d).map(([n, c]) => `${n} (${c} entries)`).join(', ')}`);
    out.push('', `Not published, kept locally (Muse session files and work trees): ${kept_.join('; ')}.`);
  }

  out.push('', '## How to re-run', '');
  out.push('- `node drill/harness/results.mjs --check` tells whether this file is what the published records make; `node drill/harness/results.mjs` writes it.');
  out.push('- `node drill/harness/tally.mjs drill/results/scored-1` and `node drill/harness/tally.mjs drill/results/d15-live-1` make the tally of one batch; the `tally.md` files next to the records were made like that.');
  out.push('- `node drill/harness/publish.mjs --root DIR --tmp DIR --home DIR --out drill/results RAW_SCORED_DIR RAW_LIVE_DIR` makes the copies from the raw batch folders, which are only on the machine that ran the drill. `--root` is the project folder the batches ran in, `--tmp` the temp base their runs used, `--home` the home folder of that machine; the values are not published, because they are the paths that are replaced. Compare the sha256 in `drill/results/SOURCES.json` with those of the raw files.');
  out.push('- Self-tests: `node drill/harness/selftest.mjs`, `node drill/oracles/selftest.mjs`, `sh drill/workspaces/selftest.sh`.');
  return `${out.join('\n')}\n`;
}

function main() {
  const a = process.argv.slice(2);
  const file = (p) => path.join(REPO, p);
  const md = readMd(file('drill/DISASTERS.md'));
  const text = build({
    scored: loadBatch(file('drill/results/scored-1')),
    live: loadBatch(file('drill/results/d15-live-1')),
    pred: predictions(md),
    expected: expectedRed(md),
    head: fs.readFileSync(path.join(HERE, 'results-head.md'), 'utf8'),
    shas: { prereg: sha256(fs.readFileSync(file('drill/PREREG.md'))), disasters: sha256(fs.readFileSync(file('drill/DISASTERS.md'))) },
    sources: JSON.parse(fs.readFileSync(file('drill/results/SOURCES.json'), 'utf8')),
    hashOf: (batch, name) => sha256(fs.readFileSync(file(`drill/results/${batch}/${name}`))),
    lineOf: (f, n) => fs.readFileSync(file(f), 'utf8').split('\n')[n - 1],
    cites: CITES,
    expect: EXPECT,
  });
  const target = file('drill/RESULTS.md');
  if (a.includes('--check')) {
    const have = fs.existsSync(target) ? fs.readFileSync(target, 'utf8') : '';
    if (have === text) console.log('drill/RESULTS.md is what the published records make');
    else {
      console.log(`drill/RESULTS.md is NOT what the published records make:\n${firstDiffs(text, have).join('\n')}`);
      process.exitCode = 1;
    }
  } else fs.writeFileSync(target, text);
}

if (process.argv[1] && import.meta.url === new URL(`file://${fs.realpathSync(process.argv[1])}`).href) main();

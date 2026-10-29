// L4d added measures, the readers (research/71-plan.md, the L4d note of 2026-10-29 07:04 UTC). None has a pass line. The capture is measures.mjs and run.mjs; this file
// only reads the stored run records (engine-runs.json, wrap.log, promise-list.txt, promise-specs.json, claim-<n>/, run.json) and prints what they hold.
// It does not import analyze.mjs (analyze.mjs imports this file): the caller hands in the run, the parsed list rows and, where it has them, the helpers.
import fs from 'node:fs';
import path from 'node:path';
import { pct } from './lib.mjs';
import { newSince } from './measures.mjs';

const SLACK_NS = 5e6; // the slack the E1b ground truth uses (analyze.mjs gateStops)
const WAIT_MS = 100; // L4c's plain Stops took 3 to 14 ms before the engine; a Stop that waited for a save took seconds
const SPAN_SLACK_MS = 2000; // create rows are stamped inside the save; the lock is let go a little after, and the watcher polls every 100 ms
const EDGE = 6000; // the bytes the check-writer saw of a file before S3x
const r1 = (x) => (x === null || x === undefined ? null : Math.round(x * 10) / 10);

// "#1 NAME · guard only · passed 07:58 · your words, turn 1": the checks the writer saved from the user's words (checks the driver seeds have no "your words").
export const listRows = (text) => String(text ?? '').split('\n').map((l) => l.match(/^#(\d+) (.*?) · (.*?) · .*?your words, turn (\d+)/)).filter(Boolean).map((m) => ({ id: Number(m[1]), name: m[2], tier: m[3], turn: Number(m[4]) }));

export const pool = (list) => ({ n: list.length, p50: pct(list, 0.5), p90: pct(list, 0.9), p99: pct(list, 0.99), max: list.length ? Math.max(...list) : null });

// ------------------------------------------------------------------ (i) per Stop: time beyond the engine's own, and whether it ran the turn's newly saved checks

// filesChanged(run, turn) -> the files that changed during that typed turn, or null (analyze.mjs changedIn): the engine runs no check after a turn that changed nothing.
export function stopMeasure(run, rows, filesChanged = () => null) {
  const eng = run.eng ?? {}, turns = (eng.turns ?? []).map((t) => ({ n: t.n, ended: Number(t.ended_ns) }));
  const gates = (eng.promise_runs ?? []).filter((r) => r.kind === 'gate').map((r) => ({ id: r.promise_id, at: Number(r.at_ns) }));
  const stops = run.hooks.filter((h) => h.ev === 'Stop').map((h) => {
    const end = h.t0 + h.ns, hit = turns.filter((t) => t.ended >= h.t0 - SLACK_NS && t.ended <= end + SLACK_NS), ok = hit.length === 1;
    const before = ok ? (hit[0].ended - h.t0) / 1e6 : null;
    return { turn: ok ? hit[0].n : h.turn, paired: ok, t0: h.t0, hook_ms: h.ns / 1e6, before_engine_ms: before, engine_ms: ok ? h.ns / 1e6 - before : null, waited: ok && before >= WAIT_MS,
      ran: [...new Set(gates.filter((g) => g.at >= h.t0 - SLACK_NS && g.at <= end + SLACK_NS).map((g) => g.id))], files_changed: filesChanged(run, h.turn)?.length ?? null };
  });
  const creates = {};
  for (const r of eng.promise_runs ?? []) if (r.kind === 'create') (creates[r.promise_id] ??= []).push(r);
  const saves = (run.rec.locks?.spans ?? []).filter((s) => s.kind === 'save' && s.end !== null);
  const checks = rows.map((row) => {
    const cr = creates[row.id] ?? [], at = cr.length ? Math.max(...cr.map((r) => Number(r.at_ns))) / 1e6 : null, sum = cr.reduce((s, r) => s + r.ms, 0);
    // the exact save span that holds the create rows; without a lock record (or with none near the rows) the create rows alone, as a run of `sum` ms that ended at `at`
    const near = at === null ? [] : saves.map((s) => ({ s, d: at < s.start ? s.start - at : at > s.end ? at - s.end : 0 })).filter((x) => x.d <= SPAN_SLACK_MS).sort((a, b) => a.d - b.d);
    const span = near[0]?.s ?? null, from = span ? span.start : at === null ? null : at - sum, to = span ? span.end : at;
    const own = stops.find((s) => s.turn === row.turn) ?? null, t0 = own ? own.t0 / 1e6 : null;
    const cls = own === null || from === null ? null : t0 >= to ? 1 : t0 > from ? 2 : 3;
    const first = stops.find((s) => s.ran.includes(row.id)) ?? null;
    return { ...row, create_rows: cr.length, create_ms: sum, saved_at_ms: at, save_from_ms: from, save_to_ms: to, basis: span ? 'lock span' : at === null ? 'no create rows' : 'create rows',
      cls, own_stop: own, first_run: first ? { turn: first.turn, kind: first.turn === row.turn ? 'own' : 'later' } : { turn: null, kind: 'never' },
      // available at its own Stop (class 1 or 2) and still not run there
      left_for_later: (cls === 1 || cls === 2) && first?.turn !== row.turn };
  });
  const pacing_timeouts = run.rec.turns.flatMap((m, i) => (typeof m.writer_wait_s === 'number' && m.writer_wait_s >= 89.5 ? [{ turn: i + 1, s: m.writer_wait_s }] : []));
  return { stops, checks, pacing_timeouts, waits: stops.filter((s) => s.paired).map((s) => s.before_engine_ms) };
}

// ------------------------------------------------------------------ (ii) processes whose working folder is inside the run folder

export function procsMeasure(run) {
  const p = run.rec.procs;
  if (!p) return null;
  const by = (l) => l.reduce((o, x) => { const k = `${x.comm} (parent ${x.pcomm ?? '?'})`; o[k] = (o[k] ?? 0) + 1; return o; }, {});
  const left = newSince(p.boot ?? [], p.pre_exit ?? []);
  return { boot: (p.boot ?? []).length, pre_exit: (p.pre_exit ?? []).length, left_running: left.length, left_by: by(left), muse_exited: p.muse_exited ?? null,
    post_exit: p.muse_exited === true ? (p.post_exit ?? []).length : null, post_exit_by: by(p.post_exit ?? []), post_exit_killed: (p.post_exit_killed ?? []).length };
}

// ------------------------------------------------------------------ (iii) per claim prompt: was a check saved, and which file lines is it about

const CODE = /\.(js|mjs|cjs|ts|html)$/;
const VERBS = ['goto', 'click', 'fill', 'press', 'select', 'check', 'uncheck', 'wait_http', 'expect_visible', 'expect_hidden', 'expect_text', 'expect_url', 'expect_count', 'expect_console_clean', 'http'];
const LOC_STRINGS = ['name', 'label', 'text', 'testid']; // what a locator says that code can hold; the role and text_re are not looked up
const long = (s) => typeof s === 'string' && [...s].length >= 3;

// The files of the copy taken before the claim was typed: [{ name, text }] of the .js .mjs .cjs .ts .html files, in name order.
export function claimFiles(dir, n) {
  const root = path.join(dir, `claim-${n}`), out = [];
  const walk = (d) => { let es; try { es = fs.readdirSync(d, { withFileTypes: true }); } catch { return; } for (const e of es) { const p = path.join(d, e.name); if (e.isDirectory()) walk(p); else if (CODE.test(e.name)) out.push({ name: path.relative(root, p), text: fs.readFileSync(p, 'utf8') }); } };
  walk(root);
  return out.sort((a, b) => (a.name < b.name ? -1 : 1));
}

// What a saved check's steps say that can be looked up in the code (the mapping rule fixed in the run log before any keyed run). http, goto, wait_http and expect_url
// with a path are routes; the strings of a locator and the `contains` of expect_text are text; everything else is not mapped.
export function anchorsOf(spec) {
  const out = [];
  (spec?.steps ?? []).forEach((s, i) => {
    const verb = VERBS.find((v) => v in s), arg = s[verb], step = i + 1;
    if (verb === 'http' || verb === 'expect_url') { if (typeof arg?.path === 'string') out.push({ step, verb, route: arg.path, method: verb === 'http' }); }
    else if (verb === 'goto' || verb === 'wait_http') { if (typeof arg === 'string') out.push({ step, verb, route: arg }); }
    else if (arg && typeof arg === 'object') {
      for (const k of LOC_STRINGS) if (long(arg[k])) out.push({ step, verb, text: arg[k], key: k });
      if (verb === 'expect_text' && long(s.contains)) out.push({ step, verb, text: s.contains, key: 'contains' });
    }
  });
  return out;
}

// The lines of a file that one anchor points at: [{ file, first, last, start, end, cls }] (lines from 1; start/end = UTF-8 byte offsets of the first line's start and
// the last line's end; cls = before / past / straddle against byte 6,000). A route block runs from the first line holding the quoted path (with the word `method`
// if any line has both) to the line where the brace depth, counted from that line's start, is back to 0.
export function mapAnchor(a, files) {
  const out = [];
  for (const f of files) {
    const lines = f.text.split('\n'), starts = [];
    let off = 0;
    for (const l of lines) { starts.push(off); off += Buffer.byteLength(l, 'utf8') + 1; }
    const hits = [];
    if (a.route !== undefined) {
      const p = a.route.split(/[?#]/)[0], quoted = (l) => ['"', "'", '`'].some((q) => l.includes(q + p + q));
      let first = lines.findIndex((l) => quoted(l) && /\bmethod\b/.test(l));
      if (first < 0) first = lines.findIndex(quoted);
      if (first >= 0) {
        let depth = 0, last = first;
        for (let j = first; j < lines.length; j++) { for (const c of lines[j]) { if (c === '{') depth++; else if (c === '}') depth--; } last = j; if (depth <= 0) break; }
        hits.push([first, last]);
      }
    } else lines.forEach((l, j) => { if (l.includes(a.text)) hits.push([j, j]); });
    for (const [i, j] of hits) {
      const start = starts[i], end = starts[j] + Buffer.byteLength(lines[j], 'utf8');
      out.push({ step: a.step, verb: a.verb, what: a.route ?? a.text, file: f.name, first: i + 1, last: j + 1, start, end, cls: end <= EDGE ? 'before' : start >= EDGE ? 'past' : 'straddle' });
    }
  }
  return out;
}

export function claimMeasure(run, rows, filesOf = (n) => claimFiles(run.dir, n)) {
  return run.rec.turns.flatMap((m, i) => (m.claim ? [i + 1] : [])).map((n) => {
    let files = [], files_error = null;
    try { files = filesOf(n); } catch (e) { files_error = String(e.message).slice(0, 100); }
    const saved = rows.filter((r) => r.turn === n).map((r) => {
      const raw = run.specs?.[String(r.id)];
      let spec = null, error = null;
      try { spec = JSON.parse(raw); } catch (e) { error = raw === undefined ? 'no stored spec' : `spec is not JSON (${String(e.message).slice(0, 60)})`; }
      const as = spec ? anchorsOf(spec) : [], mapped = as.flatMap((a) => mapAnchor(a, files).map((h) => ({ ...h, key: a.key ?? (a.route === undefined ? null : 'route') })));
      const cls = (c) => mapped.filter((x) => x.cls === c).length;
      return { id: r.id, name: r.name, tier: r.tier, steps: spec?.steps?.length ?? null, error, anchors: as.length, unmapped: as.filter((a) => !mapAnchor(a, files).length).length, mapped, before: cls('before'), straddle: cls('straddle'), past: cls('past') };
    });
    return { turn: n, files: files.map((f) => f.name), files_error, saved };
  });
}

// ------------------------------------------------------------------ the three together, over many runs

export function added(runs, { filesChanged, filesOf } = {}) {
  return runs.map((run) => {
    const rows = listRows(run.promises);
    return { run: run.label, session: run.spec.id, stops: stopMeasure(run, rows, filesChanged), procs: procsMeasure(run), claims: claimMeasure(run, rows, filesOf ? (n) => filesOf(run, n) : undefined) };
  });
}

// ------------------------------------------------------------------ calibration readings (pre-registered in the run log, section 5)

export function calReading(run, rows, { filesChanged, filesOf } = {}) {
  const id = run.spec.id, claims = claimMeasure(run, rows, filesOf ? (n) => filesOf(run, n) : undefined), sm = stopMeasure(run, rows, filesChanged);
  const base = { run: run.label, session: id, outcome: run.rec.outcome, reason: run.rec.reason ?? null, turns_done: run.rec.turns.filter((t) => t.done).length, typed_turns: run.rec.typed_turns ?? null, minutes: run.rec.minutes,
    cost_hi: run.rec.cost?.total_hi ?? null, cost_lo: run.rec.cost?.total_lo ?? null, go: run.rec.go ?? null, save_spans: (run.rec.locks?.spans ?? []).filter((s) => s.kind === 'save'), procs: procsMeasure(run), claims, stops: sm.stops, checks: sm.checks };
  if (id === 'cal-late') {
    const saved = claims[0]?.saved ?? [], mapped = saved.flatMap((s) => s.mapped);
    const reading = !saved.length ? ['STOP', 'no check was saved for the claim']
      : mapped.some((a) => a.cls === 'past') ? ['WORKED', 'a check was saved and at least one of its anchors maps to a block that starts at or after byte 6,000 (one sample)']
        : mapped.length ? ['STOP', 'a check was saved but every anchor lies before byte 6,000'] : ['READ', 'a check was saved but none of its steps maps to a line of code (a check on the page, say); reported as it is'];
    return { ...base, reading: { flag: reading[0], text: reading[1] } };
  }
  if (id === 'cal-stop') {
    const ck = sm.checks.find((c) => c.turn === 1), s = ck?.own_stop ?? null, ran = !!(s && ck && s.ran.includes(ck.id));
    const reading = !ck ? ['NOT-TRIGGERED', 'no check was saved from the claim']
      : run.rec.go?.reason === 'cap' ? ['NOT-TRIGGERED', 'the go file came from the 100 s cap, not from a save in flight']
        : !s ? ['NOT-TRIGGERED', 'the claim turn has no Stop record']
          : ck.cls !== 2 ? ['NOT-TRIGGERED', `the claim turn's Stop did not start inside the save (class ${ck.cls})`]
            : s.waited && ran ? ['WORKED', 'the Stop started inside the save, waited, and ran the new check']
              : !s.waited && !ran ? ['STOP', 'the Stop started inside the save with no wait and no gate row of the new check']
                : s.waited ? ['WAIT-SHOWN', `the Stop started inside the save and waited; it did not run the new check (files changed in that turn: ${s.files_changed ?? 'unknown'})`]
                  : ['RAN-WITHOUT-WAIT', 'the Stop ran the new check without waiting (read the numbers)'];
    return { ...base, reading: { flag: reading[0], text: reading[1] } };
  }
  return base;
}

// ------------------------------------------------------------------ text

const ms = (x) => (x === null || x === undefined ? 'n/a' : `${r1(x)}`);
const fmtPool = (p) => (p.n ? `p50 ${ms(p.p50)}, p90 ${ms(p.p90)}, p99 ${ms(p.p99)}, max ${ms(p.max)}` : 'none');
const byLine = (m) => `${m.file} ${m.first === m.last ? m.first : `${m.first}-${m.last}`} (${m.cls})`;

// The table block of the script-made summary (counted runs only). scrub removes paths; labels and numbers are all that is left.
export function addedMarkdown(list, scrub = (s) => s) {
  const L = ['## Added measures (no pass line)', '',
    'Three measures were added in L4d to see what the fixes did. They have no pass line and no verdict; they change nothing Muse or the model sees. Definitions: PROOF.md, section L4d, Method.', ''];
  const all = list.flatMap((a) => a.stops.stops), waits = all.filter((s) => s.paired).map((s) => s.before_engine_ms);
  L.push('### (i) Per Stop: time beyond the engine\'s own, and whether it ran the turn\'s newly saved checks', '',
    `${all.length} Stops in ${list.length} runs; ${all.length - waits.length} not paired with an engine turn. Time before the engine started its own work (adapter, the wait for a save, engine start), in ms, over ${waits.length} Stops: ${fmtPool(pool(waits))}. A Stop counts as having waited when this is ${WAIT_MS} ms or more (L4c: plain Stops took 3 to 14 ms).`, '',
    '| run | Stops | waited | p50 before engine (ms) | max before engine (ms) | checks saved from words | of them left for a later turn |', '|---|---|---|---|---|---|---|',
    ...list.map((a) => { const w = a.stops.waits; return `| ${a.run} | ${a.stops.stops.length} | ${a.stops.stops.filter((s) => s.waited).length} | ${ms(pool(w).p50)} | ${ms(pool(w).max)} | ${a.stops.checks.length} | ${a.stops.checks.filter((c) => c.left_for_later).length} |`; }), '');
  const slow = list.flatMap((a) => a.stops.stops.filter((s) => s.waited).map((s) => ({ run: a.run, ...s })));
  L.push(slow.length ? `Stops that waited: ${slow.map((s) => `${s.run} turn ${s.turn}: hook ${ms(s.hook_ms)} ms, before engine ${ms(s.before_engine_ms)} ms, engine ${ms(s.engine_ms)} ms, ran checks ${s.ran.length ? s.ran.map((i) => `#${i}`).join(' ') : 'none'}`).join('; ')}.` : 'No Stop waited.', '');
  const unp = list.flatMap((a) => a.stops.stops.filter((s) => !s.paired).map((s) => `${a.run} turn ${s.turn}`));
  if (unp.length) L.push(`Stops with no engine turn paired (no reading for them): ${unp.join(', ')}.`, '');
  const cks = list.flatMap((a) => a.stops.checks.map((c) => ({ run: a.run, ...c })));
  if (cks.length) {
    L.push('One row per check the writer saved from the user\'s words. Class: 1 = the save was finished before that turn\'s Stop began, 2 = the Stop began inside the save, 3 = the save began after the Stop began. "First ran" = the first Stop whose window holds a run of the check.', '',
      '| run | check | tier | from turn | class | own Stop: before engine (ms) | own Stop: files changed in the turn | first ran | left for a later turn |', '|---|---|---|---|---|---|---|---|---|',
      ...cks.map((c) => `| ${c.run} | #${c.id} ${scrub(c.name)} | ${c.tier} | ${c.turn} | ${c.cls ?? 'n/a'} | ${ms(c.own_stop?.before_engine_ms)} | ${c.own_stop?.files_changed ?? 'n/a'} | ${c.first_run.kind}${c.first_run.turn ? ` (turn ${c.first_run.turn})` : ''} | ${c.left_for_later ? 'yes' : 'no'} |`), '');
    const left = cks.filter((c) => c.left_for_later);
    L.push(left.length ? `Left for a later turn although available at its own Stop (class 1 or 2): ${left.map((c) => `${c.run} #${c.id} (turn ${c.turn}, first ran ${c.first_run.kind}${c.first_run.turn ? ` turn ${c.first_run.turn}` : ''})`).join('; ')}. The engine runs no check after a turn that changed no file, or only docs and tests (src/gate.rs, spec 18), so a Stop can leave a check for later with the wait working.` : 'No check was left for a later turn after being available at its own Stop.', '');
  }
  const to = list.flatMap((a) => a.stops.pacing_timeouts.map((p) => `${a.run} turn ${p.turn} (${p.s} s)`));
  L.push(to.length ? `The driver's wait for the check-writer of the earlier prompt timed out (90 s) before: ${to.join(', ')}.` : 'The driver\'s wait for the check-writer of the earlier prompt never timed out.', '');
  const pr = list.filter((a) => a.procs);
  L.push('### (ii) Processes with their working folder in the run folder', '',
    '"Left running" = processes present just before `/exit` that were not there when the TUI was ready (matched by pid and start time). "After exit" is read 300 ms after Muse ended; it is 0 by construction (the sandbox\'s process namespace ends with Muse).', '',
    '| run | at start | before exit | left running | by kind | Muse exited | after exit |', '|---|---|---|---|---|---|---|',
    ...pr.map((a) => `| ${a.run} | ${a.procs.boot} | ${a.procs.pre_exit} | ${a.procs.left_running} | ${Object.entries(a.procs.left_by).map(([k, n]) => `${n} x ${k}`).join(', ') || '-'} | ${a.procs.muse_exited} | ${a.procs.post_exit ?? 'n/a'} |`), '');
  const cl = list.flatMap((a) => a.claims.map((c) => ({ run: a.run, ...c })));
  L.push('### (iii) Per claim prompt: was a check saved, and which file lines is it about', '',
    cl.length ? 'Lines are those of the copy of the project taken just before the claim was typed. before / straddle / past = the block ends before byte 6,000, runs over it, or starts at or after it (the bytes the check-writer saw of a file before S3x).' : 'No claim prompt in these runs.', '');
  if (cl.length) {
    L.push('| run | claim turn | check saved | steps | anchors mapped to lines | not mapped | before | straddle | past |', '|---|---|---|---|---|---|---|---|---|',
      ...cl.flatMap((c) => (c.saved.length ? c.saved.map((s) => `| ${c.run} | ${c.turn} | #${s.id} ${scrub(s.name)} (${s.tier}) | ${s.steps ?? 'n/a'} | ${s.mapped.length} | ${s.unmapped}${s.error ? ` (${s.error})` : ''} | ${s.before} | ${s.straddle} | ${s.past} |`) : [`| ${c.run} | ${c.turn} | none | - | - | - | - | - | - |`])), '');
    L.push(...cl.flatMap((c) => c.saved.filter((s) => s.mapped.length).map((s) => `${c.run} turn ${c.turn}, #${s.id}: ${[...new Set(s.mapped.map((m) => `${m.verb} ${scrub(m.what)} -> ${byLine(m)}`))].slice(0, 12).join('; ')}${s.mapped.length > 12 ? `; and ${s.mapped.length - 12} more lines` : ''}.`)), '');
  }
  return L;
}

export function calMarkdownL4d(cals, scrub = (s) => s) {
  const L = ['# L4d calibration (script-made; not counted)', '', 'Made by `node e2e/proof/analyze.mjs --runs ... --cal --md` from the private records of the two held-out calibration runs, not edited by hand. These runs are not part of the counted tables; each was read once, before the counted runs, and the pre-registered stop readings are applied as written.', ''];
  for (const c of cals) {
    L.push(`${c.run} (${c.session}): ${c.outcome}${c.reason ? ` (${scrub(c.reason).slice(0, 160)})` : ''}, ${c.turns_done}${c.typed_turns ? ` of ${c.typed_turns}` : ''} turns done, ${c.minutes} min, cost USD ${c.cost_lo} lo / ${c.cost_hi} hi.`);
    if (c.reading) L.push(`  Reading: ${c.reading.flag}: ${c.reading.text}.`);
    for (const k of c.claims) for (const s of k.saved) L.push(`  Claim turn ${k.turn}: check #${s.id} "${scrub(s.name)}" (${s.tier}), ${s.steps ?? '?'} steps, ${s.mapped.length} lines mapped (before ${s.before}, straddle ${s.straddle}, past ${s.past}): ${[...new Set(s.mapped.map((m) => `${m.verb} ${scrub(m.what)} -> ${byLine(m)}`))].slice(0, 10).join('; ') || 'none'}.`);
    for (const k of c.claims) if (!k.saved.length) L.push(`  Claim turn ${k.turn}: no check was saved.`);
    if (c.go) L.push(`  The driver wrote the go file ${(c.go.after_ms / 1000).toFixed(1)} s after the claim was typed (reason: ${c.go.reason}). Save lock spans (exact, from /proc/locks): ${c.save_spans.map((s) => `${((s.end - s.start) / 1000).toFixed(2)} s`).join(', ') || 'none seen'}.`);
    for (const ck of c.checks) {
      const s = ck.own_stop;
      L.push(`  Check #${ck.id} (turn ${ck.turn}): class ${ck.cls ?? 'n/a'} (${ck.basis}); its Stop: ${s ? `hook ${ms(s.hook_ms)} ms, before engine ${ms(s.before_engine_ms)} ms, engine ${ms(s.engine_ms)} ms, waited ${s.waited}, ran ${s.ran.length ? s.ran.map((i) => `#${i}`).join(' ') : 'no check'}, files changed in the turn ${s.files_changed ?? 'unknown'}` : 'no Stop record'}; first ran: ${ck.first_run.kind}${ck.first_run.turn ? ` (turn ${ck.first_run.turn})` : ''}.`);
    }
    if (c.procs) L.push(`  Processes in the run folder: ${c.procs.boot} at start, ${c.procs.pre_exit} before exit, ${c.procs.left_running} left running, ${c.procs.post_exit ?? 'n/a'} after exit.`);
    L.push('');
  }
  return `${L.join('\n')}\n`;
}

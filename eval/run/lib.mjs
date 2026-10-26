// Pure helpers of the eval runner: seeded plan, prices, session-log tally, rows, spend ledger, held-out index.
// Node built-ins only. Nothing here starts a process.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';

export const ARMS = ['A', 'B', 'C', 'D'];
export const SCHEMA = 1;
export const MAX_TURNS = 6;
export const MAX_ATTEMPTS = 3; // the first run and two re-runs
const sha256 = (b) => crypto.createHash('sha256').update(b).digest('hex');
export const sha = sha256;
export const utc = (ms = Date.now()) => new Date(ms).toISOString().replace(/\.\d+Z$/, 'Z');
export const round6 = (x) => Math.round(x * 1e6) / 1e6;

// ---------------------------------------------------------------- seeded plan
export const hash32 = (s) => crypto.createHash('sha256').update(String(s)).digest().readUInt32LE(0);
function mulberry32(a) {
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
export function shuffled(list, seedText) {
  const rnd = mulberry32(hash32(seedText));
  const a = [...list];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(rnd() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}
// Paired by task: every (rep, task) block runs all arms, in an order drawn from the seed and the block alone.
export function planJobs({ tasks, arms, reps, seed }) {
  const jobs = [];
  for (let rep = 1; rep <= reps; rep++) for (const task of tasks) for (const arm of shuffled(arms, `${seed}|${task}|${rep}`)) jobs.push({ task, arm, rep, order_index: jobs.length });
  return jobs;
}
export const epKey = (j) => `${j.task}__${j.arm}__r${j.rep}`;

// ---------------------------------------------------------------- prices (USD per 1M tokens: cached, input, output; Meta list read 2026-10-26)
const PRICES = { std: [0.15, 1.25, 4.25], contributor: [0.002, 0.1, 0.2] };
export const priceOf = (model) => (String(model).endsWith('-contributor') ? PRICES.contributor : PRICES.std);
// lo: output already holds the reasoning tokens. hi: reasoning counted again and all input at the fresh price (an upper bound).
export function cost(model, u = {}) {
  const [cac, inp, out] = priceOf(model);
  const input = u.input_tokens || 0, cached = u.cached_tokens || 0, output = u.output_tokens || 0, reasoning = u.reasoning_tokens || 0;
  return { lo: (Math.max(input - cached, 0) * inp + cached * cac + output * out) / 1e6, hi: (input * inp + cached * cac + (output + reasoning) * out) / 1e6 };
}

// ---------------------------------------------------------------- Muse session logs
// home/.local/share/muse/sessions/<yyyy>/<mm>/<dd>/<session-uuid>/session.jsonl (the date folders are local time).
export function sessionFiles(home) {
  const out = [];
  const walk = (d, depth) => {
    let es;
    try { es = fs.readdirSync(d, { withFileTypes: true }); } catch { return; }
    for (const e of es) {
      if (!e.isDirectory()) continue;
      const p = path.join(d, e.name);
      if (depth < 3) walk(p, depth + 1);
      else if (fs.existsSync(path.join(p, 'session.jsonl'))) out.push(path.join(p, 'session.jsonl'));
    }
  };
  walk(path.join(home, '.local', 'share', 'muse', 'sessions'), 0);
  return out.sort();
}
export function* events(file) {
  for (const line of fs.readFileSync(file, 'utf8').split('\n')) {
    if (!line) continue;
    let r;
    try { r = JSON.parse(line); } catch { continue; }
    const ev = r?.payload?.event;
    if (ev && typeof ev === 'object' && ev.kind) yield ev;
    // A frame carries its records as JSON strings in `children` (none of the ones seen in 155 sessions held a model or hook event).
    for (const c of Array.isArray(r?.children) ? r.children : []) {
      try { const e2 = JSON.parse(c.record_json)?.payload?.event; if (e2 && typeof e2 === 'object' && e2.kind) yield e2; } catch { /* not a record */ }
    }
  }
}
const RATE = /\b429\b|rate.?limit|too many requests|retry-after/i;
export const isRateLimit = (s) => RATE.test(String(s || ''));
// Totals over every session log in an episode home. Cumulative: callers take differences between turns.
export function tally(home, fallbackModel) {
  const t = { calls: 0, steps: 0, input: 0, cached: 0, output: 0, reasoning: 0, lo: 0, hi: 0, hooks: {}, hook_failures: 0, gate_blocks: 0, failed: [], terminals: {}, rate_limited: 0, sessions: 0 };
  for (const f of sessionFiles(home)) {
    t.sessions++;
    for (const ev of events(f)) {
      if (ev.kind === 'model_completed') {
        const u = ev.usage || {};
        const c = cost(ev.model || fallbackModel, u);
        t.steps++; t.lo += c.lo; t.hi += c.hi;
        t.input += u.input_tokens || 0; t.cached += u.cached_tokens || 0; t.output += u.output_tokens || 0; t.reasoning += u.reasoning_tokens || 0;
      } else if (ev.kind === 'model_response_created') t.calls++;
      else if (ev.kind === 'hook_run_terminal') {
        const h = (t.hooks[ev.event] ||= {});
        h[ev.status] = (h[ev.status] || 0) + 1;
        if (ev.status === 'completed' && ev.exit_code === 0) h.ok = (h.ok || 0) + 1;
        if (ev.status === 'failed' || ev.status === 'timed_out') t.hook_failures++;
        if (ev.event === 'Stop' && ev.status === 'blocked') t.gate_blocks++;
      } else if (ev.kind === 'failed') {
        t.failed.push(String(ev.reason || '').slice(0, 300));
        if (isRateLimit(ev.reason)) t.rate_limited++;
      } else if (ev.kind === 'terminal') {
        t.terminals[ev.terminal] = (t.terminals[ev.terminal] || 0) + 1;
        if (isRateLimit(ev.reason)) t.rate_limited++;
      }
    }
  }
  return t;
}
export const hookRuns = (t) => Object.values(t.hooks).reduce((n, h) => n + Object.entries(h).reduce((m, [k, v]) => m + (k === 'ok' ? 0 : v), 0), 0);

// The check-writer's own ledger (every response, failed ones too): one JSON row per line.
export function writerLedger(home) {
  const out = { rows: 0, usd: 0 };
  let text = '';
  try { text = fs.readFileSync(path.join(home, '.local', 'state', 'torpor-muse', 'ledger.jsonl'), 'utf8'); } catch { return out; }
  for (const l of text.split('\n')) {
    if (!l) continue;
    try { const r = JSON.parse(l); out.rows++; out.usd += Number(r.usd) || 0; } catch { /* a torn line counts for nothing */ }
  }
  return out;
}

// ---------------------------------------------------------------- one muse exec, classified
export function classifyTurn({ code, signal, timedOut, stderr = '' }) {
  if (timedOut) return { kind: 'timeout', reason: 'turn_timeout' };
  if (code === 0) return { kind: 'ok' };
  const first = String(stderr).split('\n').map((s) => s.trim()).filter(Boolean).filter((s) => !s.startsWith('muse: workspace') && !s.startsWith('muse: system'))[0] || '';
  // G0: --max-model-steps reached = exit 1 "model did not reach a terminal state within N step(s)": the model's failure, not an outage.
  if (/did not reach a terminal state within \d+ step/i.test(stderr)) return { kind: 'step_cap', reason: 'step_cap' };
  if (/\b(401|403)\b|unauthori[sz]ed|invalid api key|forbidden/i.test(stderr)) return { kind: 'fatal', reason: `auth: ${first.slice(0, 120)}` };
  if (isRateLimit(stderr)) {
    const m = /retry[- ]after[:= ]+(\d+)/i.exec(stderr);
    return { kind: 'rate_limited', reason: 'rate_limited', retryAfterS: m ? Number(m[1]) : 0 };
  }
  if (code === 125) return { kind: 'infra', reason: `sandbox_refused: ${first.slice(0, 120)}` };
  return { kind: 'infra', reason: `muse_exit_${code ?? signal}: ${first.slice(0, 120)}` };
}

// ---------------------------------------------------------------- rows
const FIELDS = {
  schema: 'number', run_id: 'string', task: 'string', app: 'string', kind: 'string', torpor_should_help: 'string', arm: 'string', rep: 'number', attempt: 'number',
  seed: 'number', order_index: 'number', model: 'string', muse_version: 'string', engine_version: 'string', adapter_commit: 'string?',
  started_utc: 'string', ended_utc: 'string', minutes: 'number', turns_run: 'number', steps_total: 'number', timeout: 'boolean',
  usd_lo: 'number', usd_hi: 'number', calls: 'number', writer_usd_hi: 'number', writer_calls: 'number', rate_limited: 'number',
  hook_failures: 'number', gate_blocks: 'number', oracle: 'object?', final_pass: 'boolean?', later_kept: 'boolean?', break_found: 'boolean?',
  valid: 'boolean', invalid_reason: 'string?', manifest: 'object',
};
export function validateRow(r) {
  const bad = [];
  if (!r || typeof r !== 'object') return ['row is not an object'];
  for (const [k, spec] of Object.entries(FIELDS)) {
    const nullable = spec.endsWith('?'), type = spec.replace('?', '');
    const v = r[k];
    if (v === null || v === undefined) { if (!nullable || v === undefined) bad.push(`${k} missing`); continue; }
    if (type === 'number' ? !(typeof v === 'number' && Number.isFinite(v)) : typeof v !== type) bad.push(`${k} is not ${type}`);
  }
  if (r.schema !== SCHEMA) bad.push(`schema is not ${SCHEMA}`);
  if (!ARMS.includes(r.arm)) bad.push('arm not in A-D');
  if (r.valid === true) {
    if (r.invalid_reason !== null) bad.push('valid row has an invalid_reason');
    if (!r.oracle || typeof r.oracle.exit !== 'number' || typeof r.oracle.line !== 'string') bad.push('valid row needs oracle {exit, line}');
    for (const k of ['final_pass', 'later_kept']) if (typeof r[k] !== 'boolean') bad.push(`valid row needs boolean ${k}`);
  } else if (r.valid === false && !(typeof r.invalid_reason === 'string' && r.invalid_reason)) bad.push('invalid row needs an invalid_reason');
  return bad;
}
export function writeAtomic(file, text) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.tmp-${process.pid}-${crypto.randomBytes(3).toString('hex')}`;
  fs.writeFileSync(tmp, text);
  fs.renameSync(tmp, file);
}
export const rowFile = (rowsDir, j, attempt) => path.join(rowsDir, `${epKey(j)}__a${attempt}.json`);
export function writeRow(rowsDir, row) {
  const bad = validateRow(row);
  if (bad.length) throw new Error(`row does not validate: ${bad.join('; ')}`);
  writeAtomic(rowFile(rowsDir, row, row.attempt), JSON.stringify(row, null, 1) + '\n');
}
export function readRows(rowsDir) {
  const out = [];
  let names = [];
  try { names = fs.readdirSync(rowsDir); } catch { return out; }
  for (const n of names.sort()) {
    if (!n.endsWith('.json')) continue;
    try { out.push(JSON.parse(fs.readFileSync(path.join(rowsDir, n), 'utf8'))); } catch { /* a half file is not a row */ }
  }
  return out;
}
// Per episode: the last attempt number, how many attempts were infra failures, and whether it is finished (a valid row, a row that
// must not be re-run, or all 3 attempts used). A run stopped by the cap or a fatal error is not the episode's failure: it does not use an attempt up.
export const MAX_INFRA = MAX_ATTEMPTS;
export const stopsRun = (r) => /^(spend_cap|fatal)/.test(r.invalid_reason || '');
export function episodeStates(rows) {
  const m = new Map();
  for (const r of rows) {
    const k = epKey(r);
    const s = m.get(k) || { last: 0, infra: 0, done: false };
    s.last = Math.max(s.last, r.attempt);
    if (r.valid || r.retryable === false) s.done = true;
    else if (!stopsRun(r)) s.infra++;
    m.set(k, s);
  }
  for (const s of m.values()) if (s.infra >= MAX_INFRA) s.done = true;
  return m;
}

// ---------------------------------------------------------------- run-wide spend (usd_hi, writer included, every attempt)
export class SpendRefused extends Error {}
export class Spend {
  constructor(capUsd, file, floorUsd = 0.02) {
    this.cap = capUsd; this.file = file; this.total = 0; this.largest = floorUsd; this.running = 0;
    if (file && fs.existsSync(file)) {
      for (const l of fs.readFileSync(file, 'utf8').split('\n')) {
        if (!l) continue;
        try { const r = JSON.parse(l); const x = (r.usd_hi || 0) + (r.writer_usd_hi || 0); this.total += x; this.largest = Math.max(this.largest, x); } catch { /* torn line */ }
      }
    }
  }
  // Refuses before the turn when what is spent plus one worst turn per running turn (this one included) would pass the cap.
  startTurn() {
    const need = this.total + (this.running + 1) * this.largest;
    if (need > this.cap) throw new SpendRefused(`spend cap: $${this.total.toFixed(4)} spent, next turn up to $${this.largest.toFixed(4)} x ${this.running + 1} running, cap $${this.cap}`);
    this.running++;
  }
  endTurn(line) {
    this.running = Math.max(0, this.running - 1);
    this.add(line);
  }
  // Money that arrives outside a turn (a writer that finishes after the last turn): counted, no turn to end.
  add(line) {
    const x = (line.usd_hi || 0) + (line.writer_usd_hi || 0);
    this.total += x; this.largest = Math.max(this.largest, x);
    if (this.file) fs.appendFileSync(this.file, JSON.stringify({ t: utc(), ...line }) + '\n');
  }
}

// 429 / Retry-After: one pause shared by every worker.
export class Backoff {
  constructor(sleep = (ms) => new Promise((r) => setTimeout(r, ms))) { this.until = 0; this.n = 0; this.sleep = sleep; }
  hit(retryAfterS = 0) { this.n++; const s = retryAfterS > 0 ? retryAfterS : Math.min(30 * 2 ** (this.n - 1), 300); this.until = Math.max(this.until, Date.now() + s * 1000); return s; }
  ok() { this.n = 0; }
  async wait() { for (;;) { const ms = this.until - Date.now(); if (ms <= 0) return; await this.sleep(Math.min(ms, 5000)); } }
}

// ---------------------------------------------------------------- held out
// Files the model must never see: the task bank, the oracles and their kit. Found by name or by content hash (a renamed copy still counts).
export function heldOutIndex(evalDir) {
  const names = new Set(['inject.patch', 'reference.patch', 'regression.patch']); // task.json and the oracles are found by content
  const hashes = new Map(), sizes = new Set();
  const add = (d) => {
    let es;
    try { es = fs.readdirSync(d, { withFileTypes: true }); } catch { return; }
    for (const e of es) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) add(p);
      else if (e.isFile()) {
        const b = fs.readFileSync(p);
        if (b.length >= 40) { hashes.set(sha256(b), path.relative(evalDir, p)); sizes.add(b.length); }
      }
    }
  };
  for (const sub of ['tasks', 'oracles', 'lib']) add(path.join(evalDir, sub));
  return { names, hashes, sizes };
}
export function heldOutLeaks(root, idx) {
  const out = [];
  const walk = (d) => {
    let es;
    try { es = fs.readdirSync(d, { withFileTypes: true }); } catch { return; }
    for (const e of es) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) { if (e.name !== '.git' && e.name !== 'node_modules') walk(p); continue; }
      if (!e.isFile()) continue;
      let hit = idx.names.has(e.name);
      if (!hit) {
        try { const st = fs.statSync(p); if (idx.sizes.has(st.size)) hit = idx.hashes.has(sha256(fs.readFileSync(p))); } catch { /* gone */ }
      }
      if (hit) out.push(path.relative(root, p));
    }
  };
  walk(root);
  return out;
}

// ---------------------------------------------------------------- saved logs (the key is scrubbed from everything that leaves an episode)
export const scrub = (text, key) => (key ? String(text).split(key).join('[key]') : String(text));
export function saveSessions(home, destDir, key) {
  fs.mkdirSync(destDir, { recursive: true });
  sessionFiles(home).forEach((f, i) => fs.writeFileSync(path.join(destDir, `session-${i + 1}.jsonl.gz`), zlib.gzipSync(scrub(fs.readFileSync(f, 'utf8'), key))));
}

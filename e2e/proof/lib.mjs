// Pure helpers of the L4 proof driver (e2e/proof/selftest.mjs runs them). Zero dependencies, no file layout of the adapter.
import fs from 'node:fs';
import path from 'node:path';

export const shq = (s) => `'${String(s).replace(/'/g, "'\\''")}'`;
export const chars = (s) => [...String(s ?? '')].length;
const round = (x, n = 6) => Math.round(x * 10 ** n) / 10 ** n;

// settings.json text -> the same settings with every hook command run through the wrapper (measurement only: the wrapper passes stdin,
// stdout, stderr and the exit code through). keyEvents get `-k` (the wrapper hands META_API_KEY from an ancestor to the command).
export function wrapHooks(text, { wrap, log, keyEvents = ['UserPromptSubmit'] }) {
  const s = JSON.parse(text);
  let total = 0, keyed = 0;
  for (const [ev, groups] of Object.entries(s.hooks ?? {})) {
    for (const g of groups) {
      for (const h of g.hooks ?? []) {
        if (typeof h.command !== 'string') continue;
        const k = keyEvents.includes(ev);
        h.command = `${wrap}${k ? ' -k' : ''} ${log} ${ev} ${shq(h.command)}`;
        total++;
        if (k) keyed++;
      }
    }
  }
  return { text: `${JSON.stringify(s, null, 2)}\n`, total, keyed };
}

// Wrapper log text -> { runs, killed, bad }. A run = a start line and an end line of one wrapper process (same pid and event).
// t0 = wall clock (ns since the epoch) at the start; ns = monotonic length around the command; a start without an end is `killed`.
export function parseWrapLog(text) {
  const open = new Map(), runs = [], bad = [];
  for (const line of text.split('\n')) {
    if (!line) continue;
    let r;
    try { r = JSON.parse(line); } catch { bad.push(line.slice(0, 60)); continue; }
    const key = `${r.pid}:${r.ev}`;
    if (r.ph === 'start') open.set(key, r);
    else if (r.ph === 'end') {
      const s = open.get(key);
      open.delete(key);
      runs.push({ ev: r.ev, pid: r.pid, t0: r.wall_ns, ns: r.ns, exit: r.exit, sig: r.sig, outBytes: r.out_bytes, out: r.out, hadKey: s?.had_key ?? null, gotKey: s?.got_key ?? null });
    } else bad.push(line.slice(0, 60));
  }
  return { runs, killed: [...open.values()].map((s) => ({ ev: s.ev, pid: s.pid, t0: s.wall_ns })), bad };
}

// Nearest-rank percentile of a list of numbers (null for an empty list).
export function pct(list, p) {
  if (!list.length) return null;
  const a = [...list].sort((x, y) => x - y);
  return a[Math.max(0, Math.ceil(p * a.length) - 1)];
}
export const stats = (list) => ({ n: list.length, p50: pct(list, 0.5), p99: pct(list, 0.99), max: list.length ? Math.max(...list) : null });

// What a hook's stdout gives the model and what it gives the user only (src/schema.rs: closed camelCase set; Muse puts `reason`
// of a block, additionalContext and the reason of a deny into the conversation; systemMessage is shown to the user only).
// A stdout that is not the closed JSON is counted whole as model-visible (the conservative reading) and flagged.
export function visible(out) {
  const text = String(out ?? '');
  if (!text.trim()) return { model: 0, user: 0, parsed: true, decision: null, deny: false, fields: [] };
  let j;
  try { j = JSON.parse(text); } catch { return { model: chars(text), user: 0, parsed: false, decision: null, deny: false, fields: ['raw'], texts: { raw: text } }; }
  const h = j.hookSpecificOutput ?? {};
  const block = j.decision === 'block', deny = h.permissionDecision === 'deny';
  const parts = { reason: block ? j.reason : '', additionalContext: h.additionalContext, permissionDecisionReason: deny ? h.permissionDecisionReason : '' };
  const fields = Object.keys(parts).filter((k) => chars(parts[k]) > 0);
  if (chars(j.systemMessage) > 0) fields.push('systemMessage');
  return { model: Object.values(parts).reduce((t, x) => t + chars(x), 0), user: chars(j.systemMessage), parsed: true, decision: j.decision ?? null, deny, fields,
    texts: { ...Object.fromEntries(Object.entries(parts).filter(([, v]) => chars(v) > 0)), ...(chars(j.systemMessage) ? { systemMessage: j.systemMessage } : {}) } };
}

// Turn of a time: sends = ascending times the user messages were typed; 0 = before the first one (session start).
export const turnOf = (sends, t) => sends.filter((s) => s <= t).length;

// Pass lines of research/71-plan.md section 3 (fixed there; written here only to print pass or fail next to a number).
export const LINES = { e1: { p50_ms: 36, p99_ms: 150 }, e2: { clean_turn: 0, session: 2000 }, e3: { usd: 0.2 }, e4: { false_denies: 0, false_notes_per_10_turns: 1 } };

// ------------------------------------------------------------------ Muse session log (records from drill/harness/live.mjs records())

const kindOf = (r) => r.payload.event?.kind ?? r.payload_type;
const evOf = (r) => r.payload.event ?? r.payload;

// Ordered facts of one session log, times in ns since the epoch (recorded_at is microseconds):
// user (accepted prompt), tool (assistant tool calls), final (assistant message that ends a step), hook (terminal of a hook run), end (terminal),
// block (a context block put in front of the model; source runtime_hook = text a hook gave it), result (tool result text, first 400 chars).
export function timeline(recs) {
  const out = [];
  for (const r of recs) {
    const k = kindOf(r), e = evOf(r), t = Math.round((r.recorded_at ?? 0) * 1000);
    if (r.payload_type === 'runtime.user_intent.accepted') out.push({ t, k: 'user', text: (r.payload.model_messages ?? []).map((m) => (m?.content ?? []).map((c) => c.text ?? '').join('')).join('\n') });
    else if (k === 'assistant_tool_calls_committed') for (const c of e.tool_calls ?? []) out.push({ t, k: 'tool', name: c.name, args: String(c.args ?? '') });
    else if (k === 'assistant_message_committed' && e.phase !== 'commentary') out.push({ t, k: 'final', text: String(e.text ?? '') });
    else if (k === 'hook_run_terminal') out.push({ t, k: 'hook', event: e.event, status: e.status, ms: e.duration_ms, exit: e.exit_code, stdoutBytes: String(e.stdout ?? '').length });
    else if (k === 'context_block_updated') out.push({ t, k: 'block', id: String(e.id ?? ''), source: e.source, lifecycle: e.lifecycle, len: chars(e.text) });
    else if (k === 'terminal') out.push({ t, k: 'end', terminal: e.terminal, ms: e.turn_duration_ms });
    else if (k === 'tool_result_batch_committed') for (const x of e.results ?? []) out.push({ t, k: 'result', text: String(x.text ?? '').slice(0, 400) });
  }
  return out.sort((a, b) => a.t - b.t);
}

// ------------------------------------------------------------------ workspace manifests (the oracle for "put right")

// path -> sha256 of every regular file under dir except .git, node_modules and SQLite -shm files; sha256 is injected (the driver passes it).
export function manifest(dir, sha) {
  const out = {};
  const walk = (d, rel) => {
    let es;
    try { es = fs.readdirSync(d, { withFileTypes: true }); } catch { return; }
    for (const e of es) {
      const r = rel ? `${rel}/${e.name}` : e.name;
      if (e.isDirectory()) { if (e.name !== '.git' && e.name !== 'node_modules') walk(path.join(d, e.name), r); }
      else if (e.isFile() && !r.endsWith('-shm')) out[r] = sha(path.join(d, e.name));
    }
  };
  walk(dir, '');
  return out;
}
// Differences of two manifests, optionally limited to some paths: { missing, changed, added }.
export function diffManifest(base, now, only = null) {
  const keep = (p) => !only || only.some((o) => p === o || p.startsWith(`${o}/`));
  const missing = Object.keys(base).filter((p) => keep(p) && !(p in now));
  const changed = Object.keys(base).filter((p) => keep(p) && p in now && base[p] !== now[p]);
  const added = Object.keys(now).filter((p) => keep(p) && !(p in base));
  return { missing, changed, added, same: !missing.length && !changed.length && !added.length };
}

// ------------------------------------------------------------------ spend ledger (columns of spend-L2-live.md)

// A writer call the adapter could only estimate (hung, 5xx: `est`) has no token counts; its ledger row carries the worst-case dollars, which count as hi.
export const estAware = (price) => (r) => (r.est ? { lo: 0, hi: r.usd ?? 0 } : price(r));
export const sumBy = (rows, price) => rows.reduce((t, r) => { const c = price(r); return { lo: t.lo + c.lo, hi: t.hi + c.hi }; }, { lo: 0, hi: 0 });

// Per-run usage rows -> table lines. Row = { at (µs), call, model, input, cached, output, reasoning }; price(model, row) -> { lo, hi }.
export function ledgerRows(runs, price) {
  const lines = [];
  let lo = 0, hi = 0;
  for (const run of runs) {
    for (const r of [...run.rows].sort((a, b) => a.at - b.at)) {
      const c = price(r);
      lo += c.lo;
      hi += c.hi;
      lines.push(`| ${lines.length + 1} | ${run.label} | ${r.call} | ${new Date(r.at / 1000).toISOString().slice(11, 19)}Z | ${r.model} | ${r.input} | ${r.cached} | ${r.output} | ${r.reasoning} | ${c.lo.toFixed(4)} | ${c.hi.toFixed(4)} | ${hi.toFixed(4)} |`);
    }
  }
  return { lines, lo: round(lo), hi: round(hi) };
}

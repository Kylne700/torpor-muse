#!/usr/bin/env node
// Live driver for ONE disaster, D15 ("an instruction forgotten after compaction"). Replay cannot play it: the compaction summary and the
// answer are written by a model. Zero dependencies, Linux only. It starts the Muse TUI under tmux inside scripts/sandbox/muse-sandbox.sh,
// types the D15 script (turn 1, /compact, turn 2), reads the answer and the summary from Muse's own session log, and runs the unchanged
// oracles/memory.mjs on the hook tap. See README.md, "Live mode".
//   node live.mjs smoke     --arm N ...          no model, no network (echo provider): proves the hook tap under a real Muse
//   node live.mjs calibrate --out DIR ...        one arm-1 run; prints only what proves the driver, never the answer or the oracle
//   node live.mjs batch     --out DIR ...        the counted runs: repetition 1 arms 1-4, then repetition 2, then 3 (skips finished slots)
//   node live.mjs ledger    --out DIR --file F   the spend ledger, generated from the usage Muse recorded
// Common options: --muse-dir DIR (the Muse install) --key-file F (the Meta key; read by the pane's shell, never by this program)
//   --engine F --adapter F --tmp-base DIR --node-root DIR --cap 3.00 (stage, $ hi) --run-cap 0.30 (per run, $ hi)
import cp from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { cost, isRateLimit } from '../../eval/run/lib.mjs';
import { egressRead, epProcs, killEp } from '../../eval/run/sandbox.mjs';
import { memory } from '../oracles/memory.mjs';
import { readTap } from '../oracles/tap.mjs';
import * as A from './arms.mjs';
import { parseDisasters, readMd } from './parse.mjs';
import { GuardError, InfraError, NODE_BIN, NODE_EXEC, findBwrap, makeRunFolder, makeSandbox, safeDelete, toolEnv } from './sandbox.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const SANDBOX = path.join(REPO, 'scripts/sandbox/muse-sandbox.sh');
export const BATCH_LABEL = 'live, added after scored-1 because replay cannot play D15; list and predictions unchanged';
export const MODEL = 'muse-spark-1.3-contributor';
// The 13 events `torpor-muse init` registers (a superset of the engine's 10). Every arm gets the tap on all of them.
export const TAP_EVENTS = ['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse', 'PostToolUseFailure', 'PostToolBatch', 'Stop',
  'StopFailure', 'SessionEnd', 'PreCompact', 'PostCompact', 'SubagentStart', 'SubagentStop'];
// `muse exec` has --disable-reminders and --disable-web-tools; the TUI has no such flags, only these environment gates (NV for web tools).
export const MUSE_ENV = [...['TODO', 'MEMORY', 'SKILL', 'GOAL', 'VERIFY', 'SCOPE'].map((k) => `MUSE_EXPERIMENTAL_${k}_REMINDER=false`), 'MUSE_ENABLE_WEB_TOOLS=false'];
const COMMON = ['--trust-workspace', '--disable-approval', '--approval-judge', 'off', '--contributor-mode', 'off'];
const KEYED = ['--provider', 'meta', '--model', MODEL, '--reasoning-effort', 'medium', ...COMMON];
const FREE = ['--provider', 'echo', ...COMMON];
const GIB = 1024 ** 3;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const utc = () => new Date().toISOString();
const sha = (f) => crypto.createHash('sha256').update(fs.readFileSync(f)).digest('hex');
const round = (x, n = 6) => Math.round(x * 10 ** n) / 10 ** n;

// ------------------------------------------------------------------ pure helpers (the self-test runs these)

const WORDS_A = ['amber', 'cobalt', 'copper', 'indigo', 'maple', 'onyx', 'saffron', 'teal', 'umber', 'velvet', 'willow', 'zephyr'];
const WORDS_B = ['falcon', 'harbor', 'lantern', 'meadow', 'otter', 'pebble', 'quartz', 'raven', 'sparrow', 'thistle', 'walrus', 'yarrow'];
export const newNonce = (rnd = crypto.randomInt) => `${WORDS_A[rnd(WORDS_A.length)]}-${WORDS_B[rnd(WORDS_B.length)]}-${rnd(1000, 10000)}`;

// Every record of a session.jsonl in file order; retained frames are unfolded, a half-written last line is skipped.
export function records(text) {
  const out = [], seen = new Set();
  const add = (r) => {
    if (!r?.payload || (r.id && seen.has(r.id))) return;
    if (r.id) seen.add(r.id);
    out.push(r);
  };
  for (const line of text.split('\n')) {
    if (!line) continue;
    let r;
    try { r = JSON.parse(line); } catch { continue; }
    if (r.retained_frame) for (const c of r.children ?? []) { try { add(JSON.parse(c.record_json)); } catch { /* not a record */ } } else add(r);
  }
  return out;
}
const kindOf = (r) => r.payload.event?.kind ?? r.payload_type;
const evOf = (r) => r.payload.event ?? r.payload;
const textOf = (m) => (m?.content ?? []).map((c) => c.text ?? '').join('');
const usage = (u = {}) => ({ input: u.input_tokens || 0, cached: u.cached_tokens || 0, output: u.output_tokens || 0, reasoning: u.reasoning_tokens || 0 });

// One row per model call (model_completed) and per compaction call (its usage is only in goal_usage_attribution, family "compaction";
// the "provider" family repeats model_completed and "tool" is zeros, so neither is counted again). at = µs since the epoch.
export function usageRows(recs) {
  const rows = [];
  let model = null;
  for (const r of recs) {
    const e = r.payload.event;
    if (e?.kind === 'model_completed' && e.usage) { model = e.model ?? model; rows.push({ at: r.recorded_at, call: 'model', model: e.model ?? null, ...usage(e.usage) }); }
    else if (e?.kind === 'goal_usage_attribution' && e.record?.usage_family === 'compaction') rows.push({ at: r.recorded_at, call: 'compaction', model: null, ...usage(e.record.quantity) });
  }
  return rows.map((x) => ({ ...x, model: x.model ?? model ?? MODEL }));
}
export const priceRow = (r) => cost(r.model, { input_tokens: r.input, cached_tokens: r.cached, output_tokens: r.output, reasoning_tokens: r.reasoning });
export const sumCost = (rows) => rows.reduce((t, r) => { const c = priceRow(r); return { lo: t.lo + c.lo, hi: t.hi + c.hi }; }, { lo: 0, hi: 0 });

// What a finished (or running) session log says. nonce may be null (then the nonce fields are null).
export function inspect(recs, nonce = null) {
  const intents = [];
  recs.forEach((r, i) => { if (r.payload_type === 'runtime.user_intent.accepted') intents.push(i); });
  const after = intents.length >= 2 ? recs.slice(intents[1] + 1) : []; // turn 2 = everything after the second user prompt
  const installed = recs.filter((r) => kindOf(r) === 'context_compaction_installed').map(evOf);
  const summary = installed[0]?.replacement?.kind === 'summary_text' ? String(installed[0].replacement.text ?? '') : null;
  const finals = after.filter((r) => kindOf(r) === 'assistant_message_committed' && evOf(r).phase !== 'commentary');
  const tail = (installed[0]?.replay_model_messages ?? []).filter((m) => m.role !== 'developer' && !(summary && JSON.stringify(m).includes(JSON.stringify(summary.slice(0, 60)).slice(1, -1))));
  const results = after.flatMap((r) => (kindOf(r) === 'tool_result_batch_committed' ? (evOf(r).results ?? []).map((x) => JSON.stringify(x)) : kindOf(r) === 'output' ? [String(evOf(r).chunk ?? '')] : []));
  const terminals = recs.filter((r) => kindOf(r) === 'terminal').map((r) => ({ terminal: evOf(r).terminal, reason: evOf(r).reason ?? null }));
  const manual = recs.filter((r) => kindOf(r) === 'context.compact.manual.outcome').map((r) => ({ outcome: evOf(r).terminal_outcome, reason: evOf(r).reason ?? null }));
  const bad = recs.filter((r) => /^(failed|terminal)$|fatal_error/.test(kindOf(r)) && (kindOf(r) !== 'terminal' || evOf(r).terminal !== 'completed')).map((r) => JSON.stringify(evOf(r)).slice(0, 400));
  return {
    prompts: intents.map((i) => (recs[i].payload.model_messages ?? []).map(textOf).join('\n')),
    terminals, completed: terminals.filter((t) => t.terminal === 'completed').length, manual,
    compactions: installed.length,
    answer: finals.length ? String(evOf(finals[finals.length - 1]).text ?? '') : null, summary,
    nonceInTail: !nonce || !installed.length ? null : tail.some((m) => JSON.stringify(m).includes(nonce)),
    turn2Leak: nonce && intents.length >= 2 ? results.some((t) => t.includes(nonce)) : null, // the lead's extra INVALID rule: a turn-2 tool result holds the nonce
    rateLimited: bad.some((s) => isRateLimit(s)), failures: bad,
  };
}

// Puts the record-only tap behind every event of the arm's own settings (text may be null: arms 1 and 2 have no hooks).
// real: { EVENT: [original commands] }, read by live-tap.mjs through REALFILE. Matchers and timeouts of real hooks are kept.
export function wrapSettings(text, { node, tool, tap, realFile, events = TAP_EVENTS }) {
  const base = text ? JSON.parse(text) : { schema_version: 1 };
  const hooks = {}, real = {};
  for (const ev of new Set([...events, ...Object.keys(base.hooks ?? {})])) {
    real[ev] = [];
    const groups = (base.hooks?.[ev] ?? []).map((g) => ({
      ...g,
      hooks: (g.hooks ?? []).map((h) => (typeof h.command !== 'string' ? h : { ...h, command: `${node} ${tool} ${tap} ${ev} ${realFile} ${real[ev].push(h.command) - 1}` })),
    }));
    hooks[ev] = groups.length ? groups : [{ hooks: [{ type: 'command', command: `${node} ${tool} ${tap} ${ev}`, timeout: 5 }] }];
  }
  return { text: `${JSON.stringify({ ...base, hooks }, null, 2)}\n`, real };
}

// The spend ledger (format of scratch spend-S4a-run.md), generated from calls.json files: runs = [{ label, rows }].
export function ledgerLines(runs) {
  const lines = [];
  let lo = 0, hi = 0;
  for (const run of runs) for (const r of run.rows) {
    const c = priceRow(r);
    lo += c.lo; hi += c.hi;
    lines.push(`| ${lines.length + 1} | ${run.label} | ${r.call} | ${new Date(r.at / 1000).toISOString().slice(11, 19)}Z | ${r.model} | ${r.input} | ${r.cached} | ${r.output} | ${r.reasoning} | ${c.lo.toFixed(4)} | ${c.hi.toFixed(4)} | ${hi.toFixed(4)} |`);
  }
  return { lines, lo, hi };
}

// ------------------------------------------------------------------ processes: tmux, logs, probes

// tmux gets a minimal environment: the pane's shell inherits the server's, and the launcher needs only PATH (it finds the real home by uid).
const TMUX_ENV = { PATH: '/usr/local/bin:/usr/bin:/bin', TERM: 'xterm-256color', LANG: 'C.UTF-8' };
const tm = (S, ...a) => cp.spawnSync('tmux', ['-S', S.sock, ...a], { encoding: 'utf8', env: TMUX_ENV });
const pane = (S) => tm(S, 'capture-pane', '-p', '-t', 'p').stdout ?? '';
const typeLine = async (S, text) => { tm(S, 'send-keys', '-t', 'p', '-l', '--', text); await sleep(400); tm(S, 'send-keys', '-t', 'p', 'Enter'); };
const walkFiles = (dir, pred, out = []) => {
  let es;
  try { es = fs.readdirSync(dir, { withFileTypes: true }); } catch { return out; }
  for (const e of es) { const p = path.join(dir, e.name); if (e.isDirectory()) walkFiles(p, pred, out); else if (pred(p)) out.push(p); }
  return out;
};
const sessionsDir = (T) => path.join(T, 'home/.local/share/muse/sessions');
const sessionLogs = (T) => walkFiles(sessionsDir(T), (p) => path.basename(p) === 'session.jsonl');
const isSub = (p) => p.includes(`${path.sep}subagent${path.sep}`);
const readLogs = (T) => {
  const files = sessionLogs(T).sort();
  return { main: files.filter((p) => !isSub(p)).map((p) => records(fs.readFileSync(p, 'utf8'))), subs: files.filter(isSub).map((p) => records(fs.readFileSync(p, 'utf8'))), files };
};
const dateU = () => cp.spawnSync('date', ['-u', '+%Y-%m-%dT%H:%M:%SZ'], { encoding: 'utf8' }).stdout.trim();
// The main eval runner (main-1) may run alongside; every run records whether it did.
const probe = () => ({ at: dateU(), eval_runner_pids: cp.spawnSync('pgrep', ['-f', '[e]val/run/run.mjs'], { encoding: 'utf8' }).stdout.split('\n').filter(Boolean).map(Number) });
const freeGiB = (p) => { const s = fs.statfsSync(p); return (s.bavail * s.bsize) / GIB; };

class Stop extends Error { // a reason to end a run (and, with stage = true, the whole stage) without retrying
  constructor(msg, why, stage = false) { super(msg); this.why = why; this.stage = stage; }
}
let active = null; // { S, T } of the run in flight, so a signal or a crash still kills its Muse
function killActive() {
  if (!active) return;
  tm(active.S, 'kill-server');
  killEp(active.T);
}
for (const s of ['SIGINT', 'SIGTERM']) process.on(s, () => { killActive(); process.exit(130); });
process.on('exit', killActive);

// ------------------------------------------------------------------ one attempt of one slot

// -> a run record. mode: 'smoke' (echo provider, one turn), 'calibrate' (arm 1; no answer, no oracle) or 'counted'.
async function attempt(ctx, { mode, arm, rep, n, key }) {
  const keyed = mode !== 'smoke', counted = mode === 'counted';
  const base = path.join(ctx.out, counted ? '' : mode === 'calibrate' ? 'calibration' : 'smoke', 'runs', key);
  fs.mkdirSync(base, { recursive: true });
  const rec = { started_at: utc(), overlap: { start: probe() } };
  const nonce = newNonce();
  const t0 = Date.now();
  let T = null, sb = null, L = null, prompts = null, outcome = 'INFRA-FAILURE', reason = null, stage = false, phase = 'setup';
  const tapFile = () => path.join(T, 'live/tap.jsonl');
  const rows = () => { try { return readTap(tapFile()).rows; } catch { return []; } };
  try {
    if (freeGiB('/') < ctx.minRoot || freeGiB('/tmp') < ctx.minTmp) throw new Stop(`disk: / ${freeGiB('/').toFixed(1)} GiB, /tmp ${freeGiB('/tmp').toFixed(1)} GiB free`, 'disk', true);
    if (arm >= 3 && !ctx.engine) throw new GuardError('--engine is needed for arms 3 and 4');
    if (arm === 4 && !ctx.adapter) throw new GuardError('--adapter is needed for arm 4');
    const d = ctx.d15;
    prompts = [d.turns[0].items[0].text.replace('{NONCE}', nonce), d.turns[1].items[0].text];
    T = makeRunFolder(ctx.tmpBase);
    active = { S: ctx.S, T };
    sb = makeSandbox({ T, bwrap: ctx.bwrap });
    const build = sb.exec({ argv: ['sh', path.join(REPO, 'drill/workspaces/D15/build.sh'), T], cwd: T, env: toolEnv(T), ro: [path.join(REPO, 'drill/workspaces')], merge: true, timeoutMs: 180000 });
    if (build.exit !== 0) throw new InfraError(`the builder failed (exit ${build.exit}): ${build.stdout.trim().slice(0, 300)}`);
    const ws = path.join(T, 'ws');
    if (!fs.existsSync(path.join(ws, '.synthetic')) || !fs.existsSync(path.join(T, '.synthetic'))) throw new InfraError('the builder left no .synthetic marker');
    fs.writeFileSync(path.join(T, '.drill-run'), `${JSON.stringify({ id: 'D15', arm, rep, mode: 'live', n })}\n`);
    for (const dir of ['home/.config', 'tmp', 'torpor-home', 'muse-share', 'bin', 'live']) fs.mkdirSync(path.join(T, dir), { recursive: true });
    fs.writeFileSync(path.join(T, 'home/.gitconfig'), '[user]\n\tname = drill\n\temail = drill.invalid\n[core]\n\thooksPath = /dev/null\n[commit]\n\tgpgsign = false\n');
    let armText = null;
    if (arm >= 3) {
      A.installPrograms(T, arm, { engine: ctx.engine, adapter: ctx.adapter });
      armText = (arm === 3 ? A.writeArm3Settings(T) : A.runArm4Init(T, sb)).text;
    }
    const tool = path.join(T, 'bin/live-tap.mjs');
    fs.copyFileSync(path.join(HERE, 'live-tap.mjs'), tool);
    const realFile = path.join(T, 'live/real.json');
    const w = wrapSettings(armText, { node: NODE_EXEC, tool, tap: tapFile(), realFile });
    fs.writeFileSync(realFile, JSON.stringify(w.real));
    fs.mkdirSync(path.dirname(A.settingsPath(T)), { recursive: true });
    fs.writeFileSync(A.settingsPath(T), w.text);
    A.parseSettings(w.text); // a file Muse would refuse is an infra failure
    fs.copyFileSync(A.settingsPath(T), path.join(base, 'settings.json'));
    fs.copyFileSync(realFile, path.join(base, 'real.json'));

    const autocommit = (turn) => {
      if (arm !== 2) return;
      const r = sb.exec({ argv: ['sh', '-c', `git add -A && git commit -q --allow-empty -m "auto: turn ${turn}" && git rev-parse HEAD`], cwd: ws, env: toolEnv(T), merge: true, timeoutMs: 60000 });
      if (r.exit !== 0) throw new InfraError(`auto-commit failed before turn ${turn}: ${r.stdout.slice(0, 200)}`);
    };
    // The pane's shell reads the key from the file for this one command; this program never sees it.
    const muse = [path.join(ctx.museDir, 'muse'), ...(keyed ? KEYED : FREE)];
    const launcher = [SANDBOX, '--run-dir', T, '--ro', ctx.museDir, '--ro', ctx.nodeRoot, '--net', keyed ? 'meta' : 'none', ...(keyed ? ['--drain', '3'] : []), '--chdir', ws, '--', 'env', 'TERM=xterm-256color', ...MUSE_ENV, ...muse];
    const paneCmd = `${keyed ? `META_API_KEY="$(cat '${ctx.keyFile}')" ` : ''}${launcher.join(' ')}; echo EXITED $?; sleep 900`;
    cp.spawnSync('tmux', ['-S', ctx.S.sock, '-f', '/dev/null', 'new-session', '-d', '-s', 'p', '-x', '170', '-y', '50', '-c', ctx.S.dir, paneCmd], { env: TMUX_ENV });

    // Every wait ticks the watchers: cost (main and subagent logs), 429, compactions, and Muse still being alive.
    const tick = () => {
      L = readLogs(T);
      const c = sumCost([...L.main, ...L.subs].flatMap(usageRows));
      if (c.hi > ctx.runCap) throw new Stop(`cost stop: $${c.hi.toFixed(4)} hi is above the $${ctx.runCap.toFixed(2)} per-run cap`, 'cost');
      if (ctx.spent + c.hi > ctx.cap) throw new Stop(`stage cap: $${(ctx.spent + c.hi).toFixed(4)} hi is above $${ctx.cap.toFixed(2)}`, 'cap', true);
      const info = L.main[0] ? inspect(L.main[0]) : null;
      if ([...L.main, ...L.subs].some((r) => inspect(r).rateLimited)) throw new Stop('the provider answered 429 / rate limit', 'rate-limit', true);
      if (info && info.compactions > 2) throw new Stop(`${info.compactions} compactions (more than 2)`, 'compactions');
      return info;
    };
    const until = async (what, ms, fn) => {
      for (const end = Date.now() + ms; ; await sleep(1000)) {
        const v = fn(tick());
        if (v) return v;
        if (/EXITED \d+/.test(pane(ctx.S))) throw new InfraError(`Muse exited while waiting for ${what}`);
        if (Date.now() > end) throw new InfraError(`timeout (${ms / 1000} s) waiting for ${what}`);
      }
    };
    const soft = (what, ms, fn) => until(what, ms, fn).catch((e) => { if (!(e instanceof InfraError)) throw e; return false; });
    const turn = async (n, text) => {
      autocommit(n);
      await typeLine(ctx.S, text);
      const info = await until(`the prompt of turn ${n} to be accepted`, 30000, (i) => (i && i.prompts.length >= n ? i : null));
      if (!info.prompts[n - 1].includes(text.trim())) throw new InfraError(`Muse accepted a different text for turn ${n}`);
      await until(`turn ${n} to finish`, 240000, (i) => {
        if (i && i.terminals.length >= n && i.terminals[n - 1].terminal !== 'completed') throw new InfraError(`turn ${n} ended with "${i.terminals[n - 1].terminal}"`);
        return i && i.completed >= n ? i : null;
      });
      await soft(`the Stop hook of turn ${n}`, 30000, () => rows().filter((r) => r.event === 'Stop' && r.dir === 'out').length >= n);
    };

    phase = 'boot';
    await until('the TUI prompt', 90000, () => pane(ctx.S).includes('❯'));
    await sleep(1500);
    phase = 'turn1';
    await turn(1, prompts[0]);
    if (mode !== 'smoke') { // smoke stops after turn 1: the echo provider cannot compact
      phase = 'compact';
      await typeLine(ctx.S, '/compact');
      await until('the compaction', 240000, (i) => {
        const bad = i?.manual.find((m) => m.outcome !== 'compacted');
        if (bad) throw new InfraError(`manual compaction did not compact: ${bad.outcome} ${bad.reason ?? ''}`.trim());
        return i && i.compactions >= 1 ? i : null;
      });
      await soft('SessionStart(compact)', 8000, () => rows().some((r) => r.event === 'SessionStart' && r.dir === 'in' && r.json?.source === 'compact'));
      phase = 'turn2';
      await turn(2, prompts[1]);
    }
    phase = 'exit';
    await typeLine(ctx.S, '/exit');
    await soft('Muse to exit', 30000, () => /EXITED \d+/.test(pane(ctx.S)));
    tick();
    outcome = 'DONE';
  } catch (e) {
    if (e instanceof Stop) { outcome = e.why === 'cost' || e.why === 'compactions' ? 'INVALID' : 'INFRA-FAILURE'; reason = e.message; stage = e.stage; }
    else if (e instanceof GuardError) { outcome = 'INVALID'; reason = e.message; }
    else { reason = e instanceof InfraError ? e.message : `harness defect: ${String(e.stack ?? e).split('\n').slice(0, 4).join(' | ')}`; stage = !(e instanceof InfraError); } // a bug in this driver is not retried
    if (T && ctx.S && phase !== 'turn2' && phase !== 'exit') fs.writeFileSync(path.join(base, `pane-failed-in-${phase}.txt`), pane(ctx.S)); // never printed
  }

  // --- collect, then tear down (Muse first, then the run folder)
  let result = null;
  try {
    if (T) {
      tm(ctx.S, 'kill-server');
      killEp(T);
      for (let i = 0; i < 20 && epProcs(T).length; i++) await sleep(250);
      L = readLogs(T);
      const logs = [...L.main, ...L.subs];
      const calls = logs.flatMap((r, i) => usageRows(r).map((u) => ({ ...u, log: i < L.main.length ? 'main' : `sub-${i - L.main.length + 1}` })));
      fs.writeFileSync(path.join(base, 'calls.json'), `${JSON.stringify({ started_at: rec.started_at, rows: calls })}\n`);
      const c = sumCost(calls);
      const info = L.main.length === 1 ? inspect(L.main[0], counted ? nonce : null) : null;
      const tap = rows();
      const tapCount = {};
      for (const r of tap) tapCount[`${r.event}:${r.dir}`] = (tapCount[`${r.event}:${r.dir}`] ?? 0) + 1;
      if (fs.existsSync(tapFile())) { // calibration and smoke taps keep no payloads (a Stop payload holds the model's reply)
        if (counted) fs.copyFileSync(tapFile(), path.join(base, 'tap.jsonl'));
        else fs.writeFileSync(path.join(base, 'tap.jsonl'), tap.map((r) => `${JSON.stringify({ ...r, json: r.json ? Object.keys(r.json) : null, stdout: undefined })}\n`).join(''));
      }
      if (fs.existsSync(path.join(T, 'net/proxy.log'))) fs.copyFileSync(path.join(T, 'net/proxy.log'), path.join(base, 'proxy.log'));
      result = {
        cost: { lo: round(c.lo), hi: round(c.hi), calls: calls.filter((x) => x.call === 'model').length, compaction_calls: calls.filter((x) => x.call === 'compaction').length },
        minutes: round((Date.now() - t0) / 60000, 2), compactions: info?.compactions ?? null, models: [...new Set(calls.map((x) => x.model))],
        compactSeen: tap.some((r) => r.event === 'SessionStart' && r.dir === 'in' && r.json?.source === 'compact'),
        sessions: { main: L.main.length, subagents: L.subs.length }, egress: egressRead(T), tap_events: tapCount,
      };
      if (outcome === 'DONE' && mode !== 'smoke') {
        if (L.main.length !== 1) throw new InfraError(`expected one main session log, found ${L.main.length}`);
        if (info.answer === null) throw new InfraError('no assistant reply to turn 2 in the session log');
        result.answer_extracted = info.answer.length > 0;
        result.summary_extracted = info.summary !== null;
        if (counted) {
          const inHome = walkFiles(T, (p) => !p.startsWith(`${ws0(T)}${path.sep}`) && !p.startsWith(`${sessionsDir(T)}${path.sep}`) && !p.startsWith(`${path.join(T, 'live')}${path.sep}`) && !p.startsWith(`${path.join(T, 'net')}${path.sep}`)
          && !p.startsWith(`${path.join(T, 'bin')}${path.sep}`) && path.basename(p) !== 'tui-history.jsonl' && fs.statSync(p).size < 50e6 && fs.readFileSync(p).toString('latin1').includes(nonce)).map((p) => path.relative(T, p));
          const res = memory({ rows: tap, nonce, answer: info.answer, workspace: ws0(T), summary: info.summary });
          result.oracle = { kind: 'memory', command: 'node drill/oracles/memory.mjs --tap tap.jsonl --nonce NONCE --answer answer.txt --workspace WS --summary summary.txt', ...res };
          result.oracle.line = `memory ${res.status}: compactSeen=${res.compactSeen} answerHasNonce=${res.answerHasNonce} nonceInFiles=${res.nonceInFiles} engineContext=${res.engineContext} summaryHasNonce=${res.summaryHasNonce}`;
          Object.assign(result, { engineContext: res.engineContext, summaryHasNonce: res.summaryHasNonce, nonceInTail: info.nonceInTail, turn2Leak: info.turn2Leak, nonce, nonce_files_outside_ws: inHome });
          fs.writeFileSync(path.join(base, 'answer.txt'), info.answer);
          if (info.summary !== null) fs.writeFileSync(path.join(base, 'summary.txt'), info.summary);
          for (const [i, f] of L.files.entries()) fs.copyFileSync(f, path.join(base, isSub(f) ? `session-sub-${i}.jsonl` : 'session.jsonl'));
          outcome = res.status === 'PASS' ? 'PS' : res.status === 'FAIL' ? 'FL' : 'INVALID';
          reason = res.status === 'INVALID' ? `oracle: ${!res.compactSeen ? 'no compaction in the tap' : 'nonce in a workspace file'}` : null;
          if (info.turn2Leak) { outcome = 'INVALID'; reason = `${reason ? `${reason}; ` : ''}the nonce is in a tool result of turn 2`; }
        } else outcome = 'OK';
      } else if (outcome === 'DONE') outcome = 'OK';
    }
  } catch (e) {
    outcome = 'INFRA-FAILURE';
    reason = e instanceof InfraError ? e.message : `harness defect: ${String(e.stack ?? e).split('\n').slice(0, 4).join(' | ')}`;
    stage = !(e instanceof InfraError);
  } finally {
    if (sb) sb.reapRun();
    if (T) { killEp(T); safeDelete(T); }
    active = null;
  }
  const end = probe();
  return {
    stage, record: {
      schema: 'torpor-drill-run/1', batch: ctx.batchId, batch_label: BATCH_LABEL, disaster: 'D15', title: ctx.d15.title, class: 'memory', arm, arm_name: A.ARM_NAMES[arm - 1], control: null,
      repetition: rep, mode: 'live', attempt: n, outcome, outcome_reason: reason, started_at: rec.started_at, ended_at: utc(), overlap: { ...rec.overlap, end },
      ...(counted || mode === 'smoke' ? {} : { calibration: true }), ...result, method: ctx.method, driver: ctx.driver,
      engine: arm >= 3 ? ctx.prov.engine : null, adapter: arm === 4 ? ctx.prov.adapter : null, muse: ctx.prov.muse, side_files: path.relative(ctx.out, base),
    },
  };
}
const ws0 = (T) => path.join(T, 'ws');

// ------------------------------------------------------------------ setup and the slot loop

function setup(o) {
  const need = (k) => o[k] ?? (() => { throw new Error(`option --${k} is needed`); })();
  const out = path.resolve(need('out'));
  const bwrap = findBwrap();
  const tmpBase = path.resolve(o['tmp-base'] ?? os.tmpdir());
  fs.mkdirSync(tmpBase, { recursive: true });
  const S = makeRunFolder(tmpBase);
  const dir = S;
  const museDir = o['muse-dir'] ? path.resolve(o['muse-dir']) : null;
  const ctx = {
    out, bwrap, tmpBase, S: { dir, sock: path.join(dir, 'tm.sock') }, museDir, keyFile: o['key-file'] ? path.resolve(o['key-file']) : null,
    nodeRoot: path.resolve(o['node-root'] ?? path.dirname(NODE_BIN)), engine: o.engine ? path.resolve(o.engine) : null, adapter: o.adapter ? path.resolve(o.adapter) : null,
    cap: Number(o.cap ?? 3), runCap: Number(o['run-cap'] ?? 0.3), minRoot: 11.5, minTmp: 8, spent: 0, batchId: path.basename(out),
  };
  for (const [k, v] of Object.entries({ museDir: ctx.museDir, keyFile: ctx.keyFile, nodeRoot: ctx.nodeRoot, node: NODE_EXEC, tmpBase, out })) {
    if (v && !/^[\w@%+=:,./-]+$/.test(v)) throw new Error(`${k} has a character that the pane command cannot carry safely: ${v}`);
  }
  if (!fs.existsSync(SANDBOX)) throw new Error('scripts/sandbox/muse-sandbox.sh is missing');
  const d15 = parseDisasters(readMd(path.join(REPO, 'drill/DISASTERS.md'))).disasters.find((d) => d.id === 'D15');
  const t = d15?.turns;
  if (!(t?.length === 2 && t[0].items.length === 2 && t[0].items[0].kind === 'user' && t[0].items[1].kind === 'compact' && t[1].items.length === 1 && t[1].items[0].kind === 'user')) throw new Error('D15 in DISASTERS.md is not the script this driver plays');
  ctx.d15 = d15;
  const git = (...a) => cp.spawnSync('git', ['-C', REPO, ...a], { encoding: 'utf8' }).stdout.trim();
  const files = ['live.mjs', 'live-tap.mjs'];
  ctx.driver = { commit: git('rev-parse', 'HEAD'), dirty: git('status', '--porcelain', '--', ...files.map((f) => `drill/harness/${f}`)) !== '', sha256: Object.fromEntries(files.map((f) => [f, sha(path.join(HERE, f))])) };
  ctx.method = {
    name: 'tmux-TUI', compaction: 'typed /compact after turn 1 completed; no lowered context limits', muse_flags: o.smoke ? FREE : KEYED, muse_env: MUSE_ENV,
    not_in_tui: ['--disable-reminders', '--disable-web-tools', '--max-model-steps', '--session-id'], tap: 'record-only wrapper (live-tap.mjs) on every event of every arm',
    caps: { stage_hi: ctx.cap, run_hi: ctx.runCap },
  };
  const lock = fs.existsSync(path.join(REPO, 'engine.lock')) ? fs.readFileSync(path.join(REPO, 'engine.lock'), 'utf8') : '';
  const base = (i) => i && { ...i, path: path.basename(i.path) };
  const mbin = museDir && fs.readdirSync(museDir).find((f) => /^muse-bin-/.test(f));
  ctx.prov = {
    engine: ctx.engine ? base(A.engineInfo(ctx.engine, lock, A.versionLine)) : null,
    adapter: ctx.adapter ? base(A.adapterInfo(ctx.adapter, path.resolve(path.dirname(ctx.adapter), '../..'), A.versionLine)) : null, // the worktree it was built in
    muse: museDir && { launcher_sha256: sha(path.join(museDir, 'muse')), binary: mbin ? { name: mbin, sha256: sha(path.join(museDir, mbin)) } : null, version_line: museVersion(ctx) },
  };
  return ctx;
}
function museVersion(ctx) {
  const T = makeRunFolder(ctx.tmpBase);
  try {
    for (const d of ['home', 'tmp', 'muse-share']) fs.mkdirSync(path.join(T, d));
    const r = cp.spawnSync(SANDBOX, ['--run-dir', T, '--ro', ctx.museDir, '--net', 'none', '--', path.join(ctx.museDir, 'muse'), '--version'], { encoding: 'utf8', timeout: 30000, env: { PATH: '/usr/local/bin:/usr/bin:/bin' } });
    return r.status === 0 ? r.stdout.trim().split('\n')[0] : `exit ${r.status}`;
  } finally { safeDelete(T); }
}
const readRuns = (file) => (fs.existsSync(file) ? fs.readFileSync(file, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l)) : []);

// One slot (arm, repetition) with the infra rule of PREREG section 6: at most 2 re-runs, the 3rd failure is INVALID.
async function slot(ctx, mode, arm, rep, first = 1) {
  const file = path.join(ctx.out, mode === 'counted' ? 'runs.jsonl' : `${mode === 'calibrate' ? 'calibration' : 'smoke'}/runs.jsonl`);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  let last = null;
  for (let n = first; n <= 3; n++) {
    const key = `a${arm}-r${rep}-t${n}`;
    const { record, stage } = await attempt(ctx, { mode, arm, rep, n, key });
    if (record.outcome === 'INFRA-FAILURE' && n === 3 && !stage) record.outcome = 'INVALID';
    fs.appendFileSync(file, `${JSON.stringify(record)}\n`);
    ctx.spent += record.cost?.hi ?? 0;
    last = { record, stage };
    if (record.outcome !== 'INFRA-FAILURE' || stage) break;
    await sleep(2000);
  }
  return last;
}
const allSpent = (ctx) => [path.join(ctx.out, 'runs.jsonl'), path.join(ctx.out, 'calibration/runs.jsonl')].flatMap(readRuns).reduce((s, r) => s + (r.cost?.hi ?? 0), 0);

async function main() {
  const [cmd, ...rest] = process.argv.slice(2);
  const o = {};
  for (let i = 0; i < rest.length; i += 2) o[rest[i].replace(/^--/, '')] = rest[i + 1];
  if (cmd === 'ledger') {
    const dirs = [path.join(o.out, 'calibration/runs'), path.join(o.out, 'runs')];
    const runs = dirs.flatMap((d) => (fs.existsSync(d) ? fs.readdirSync(d).map((k) => ({ k, d, calls: JSON.parse(fs.readFileSync(path.join(d, k, 'calls.json'), 'utf8')) })) : []))
      .map((x) => ({ label: `${x.d.endsWith('calibration/runs') ? 'calibration ' : ''}${x.k}`, started: x.calls.started_at, rows: x.calls.rows })).sort((a, b) => a.started.localeCompare(b.started));
    const { lines, lo, hi } = ledgerLines(runs);
    fs.writeFileSync(o.file, [`# D15-live spend ledger (cap $${Number(o.cap ?? 3).toFixed(2)} hi for the whole stage, calibration included; every model call counts, failed runs too)`, '',
      `Model: ${MODEL} (Meta API, contributor-priced id) driven by the Muse Code TUI under tmux (drill/harness/live.mjs); one row per model call or compaction call, read from the session logs by script.`,
      'Prices per 1M tokens (contributor ids): cached input $0.002, input $0.10, output $0.20 (eval/run/lib.mjs cost()).',
      'lo = (input - cached) x 0.10 + cached x 0.002 + output x 0.20; hi = input x 0.10 + cached x 0.002 + (output + reasoning) x 0.20 (upper bound if reasoning is not inside output).', '',
      '| # | run | call | time UTC | model | input | cached | output | reasoning | $ lo | $ hi | cumulative $ hi |', '|---|-----|------|----------|-------|-------|--------|--------|-----------|------|------|-----------------|',
      ...lines, '', `Total: ${lines.length} calls, $ lo ${lo.toFixed(4)}, $ hi ${hi.toFixed(4)} of the $${Number(o.cap ?? 3).toFixed(2)} cap.`, ''].join('\n'));
    console.log(`ledger: ${lines.length} calls, hi $${hi.toFixed(4)}`);
    return;
  }
  const ctx = setup({ ...o, smoke: cmd === 'smoke' });
  try {
    ctx.spent = allSpent(ctx);
    if (cmd === 'smoke') {
      const { record: r } = await slot(ctx, 'smoke', Number(o.arm ?? 1), 1);
      console.log(`smoke arm ${r.arm}: ${r.outcome}${r.outcome_reason ? ` (${r.outcome_reason.slice(0, 200)})` : ''} sessions=${JSON.stringify(r.sessions)} tap=${JSON.stringify(r.tap_events)}`);
    } else if (cmd === 'calibrate') {
      const n = readRuns(path.join(ctx.out, 'calibration/runs.jsonl')).filter((r) => r.calibration).length + 1;
      if (n > 3) throw new Error('3 calibration runs are done');
      const { record: r } = await slot(ctx, 'calibrate', 1, n);
      // Only what proves the driver: no answer, no oracle, no nonce fields.
      console.log(`calibration ${n}: ${r.outcome}${r.outcome_reason ? ` (${r.outcome_reason.slice(0, 200)})` : ''} compactSeen=${r.compactSeen} compactions=${r.compactions} cost lo $${r.cost?.lo} hi $${r.cost?.hi} minutes=${r.minutes} subagent_sessions=${r.sessions?.subagents} models=${r.models} tap=${JSON.stringify(r.tap_events)} answer_extracted=${r.answer_extracted} summary_extracted=${r.summary_extracted}`);
    } else if (cmd === 'batch') {
      fs.mkdirSync(ctx.out, { recursive: true });
      fs.writeFileSync(path.join(ctx.out, 'method.json'), `${JSON.stringify({ batch: ctx.batchId, batch_label: BATCH_LABEL, method: ctx.method, driver: ctx.driver, prov: ctx.prov }, null, 2)}\n`);
      const file = path.join(ctx.out, 'runs.jsonl');
      for (const rep of [1, 2, 3]) for (const arm of (o.arms ?? '1,2,3,4').split(',').map(Number)) {
        const have = readRuns(file).filter((r) => r.arm === arm && r.repetition === rep);
        if (have.some((r) => r.outcome !== 'INFRA-FAILURE') || have.length >= 3) continue;
        const { record: r, stage } = await slot(ctx, 'counted', arm, rep, have.length + 1);
        console.log(`D15 arm ${arm} rep ${rep} attempt ${r.attempt} -> ${r.outcome} [${r.oracle?.line ?? r.outcome_reason ?? ''}] compactions=${r.compactions} cost hi $${r.cost?.hi} minutes=${r.minutes}`);
        if (stage) { console.log(`STAGE STOP: ${r.outcome_reason}`); return; }
      }
    } else throw new Error('usage: live.mjs smoke|calibrate|batch|ledger --out DIR ...');
  } finally {
    safeDelete(ctx.S.dir);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => { console.error(e.message); process.exitCode = 1; });
}

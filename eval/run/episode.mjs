// One episode: task x arm x rep x attempt, played turn by turn through real Muse Code turns and scored by the held-out oracle.
// Every model turn, the patch, the init and the oracle each run in their own sandbox; the host acts in between.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { INSTALL, InfraError } from './arms.mjs';
import * as L from './lib.mjs';
import { epProcs, killEp, sandbox, sandboxInfo } from './sandbox.mjs';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export const activeEps = new Set(); // for the runner's signal handler

// ---------------------------------------------------------------- small host helpers
// Only ever deletes an episode folder the runner made itself (ep-<hex> or ep-<hex>.oracle directly under the temp base).
export function rmEp(p, base) {
  const r = path.resolve(p);
  if (path.dirname(r) !== path.resolve(base) || !/^ep-[0-9a-f]{10}(\.oracle)?$/.test(path.basename(r))) throw new Error(`refusing to delete ${r}`);
  try { fs.rmSync(r, { recursive: true, force: true }); } catch {
    execFileSync('chmod', ['-R', 'u+rwX', r]); // a read-only folder the model made
    fs.rmSync(r, { recursive: true, force: true });
  }
}
// Muse's shell helper copies the key into tbh-linux-helper-payload-* folders; they must not outlive the turn.
export function removePayloads(ep) {
  let n = 0;
  const walk = (d, depth) => {
    let es;
    try { es = fs.readdirSync(d, { withFileTypes: true }); } catch { return; }
    for (const e of es) {
      if (!e.isDirectory() || e.name === '.git' || e.name === 'node_modules') continue;
      const p = path.join(d, e.name);
      if (e.name.startsWith('tbh-linux-helper-payload-')) { fs.rmSync(p, { recursive: true, force: true }); n++; } else if (depth < 6) walk(p, depth + 1);
    }
  };
  walk(ep, 0);
  return n;
}
const countFiles = (dir) => {
  let n = 0;
  const walk = (d) => { let es; try { es = fs.readdirSync(d, { withFileTypes: true }); } catch { return; } for (const e of es) { if (e.isDirectory()) walk(path.join(d, e.name)); else n++; } };
  walk(dir);
  return n;
};
// Arm D's check-writer runs detached and may finish after Muse is gone: wait for it (not for the model's own leftovers).
export async function waitWriters(ep, maxMs) {
  const t0 = Date.now();
  for (;;) {
    const w = epProcs(ep).filter((p) => /torpor-muse\S*\s+suggest\b/.test(p.cmd));
    if (!w.length) return { waited_ms: Date.now() - t0, still_running: 0 };
    if (Date.now() - t0 >= maxMs) return { waited_ms: Date.now() - t0, still_running: w.length };
    await sleep(400);
  }
}
const keepTorpor = (p) => /torpor/.test(p.cmd);

// ---------------------------------------------------------------- host-side runs (patch, oracle) in their own sandboxes
export async function runOracle(cx, ep, task) {
  const od = `${ep}.oracle`;
  const ws = path.join(ep, 'ws');
  const r = await sandbox({
    runDir: od, ro: [cx.evalDir, ws], net: 'none', pid: true, chdir: od,
    cmd: ['node', path.join(cx.evalDir, 'oracles', `${task.app}.mjs`), '--task', task.id, '--dir', ws], timeoutMs: cx.oracleTimeoutMs || 120000,
  });
  const lines = r.stdout.split('\n').map((s) => s.trim()).filter(Boolean);
  const line = lines[lines.length - 1] || '';
  let parsed = null;
  try { parsed = JSON.parse(line); } catch { /* not JSON */ }
  const out = { exit: r.code ?? -1, line: line.slice(0, 4000), parsed, timedOut: r.timedOut, stderr: r.stderr.trim().split('\n')[0] || '' };
  out.infra = r.timedOut ? 'oracle_timeout' : (r.code === 0 || r.code === 1) && parsed ? null : `oracle_exit_${r.code}${parsed ? '' : '_no_json'}: ${out.stderr.slice(0, 100)}`;
  return out;
}
const failedIds = (o) => (Array.isArray(o.parsed?.failed) ? o.parsed.failed : (o.parsed?.checks || []).filter((c) => !c.ok).map((c) => c.id));
const laterKept = (o) => {
  const g = o.parsed?.groups?.later;
  if (g && typeof g.fail === 'number') return g.fail === 0;
  return !(o.parsed?.checks || []).some((c) => !c.ok && String(c.id).startsWith('l'));
};

export async function applyInject(cx, ep, task) {
  const patch = path.join(cx.evalDir, 'tasks', task.id, task.inject.patch);
  const r = await sandbox({ runDir: ep, ro: [patch], net: 'none', pid: true, chdir: path.join(ep, 'ws'), cmd: ['git', 'apply', '--recount', '-C1', '--whitespace=nowarn', patch], timeoutMs: 30000 });
  return { ok: r.code === 0, why: (r.stderr || '').trim().split('\n')[0].slice(0, 160) };
}

// ---------------------------------------------------------------- the model turn
export function museCmd(cx, session, promptFile) {
  const a = ['exec', '--provider', cx.keyed ? 'meta' : 'echo'];
  if (cx.keyed) a.push('--model', cx.model, '--reasoning-effort', cx.effort); // --model needs the meta provider; every exec names it (the session record has no usable model)
  a.push('--contributor-mode', 'off', '--approval-judge', 'off', '--disable-web-tools', '--disable-reminders', '--trust-workspace', '--disable-approval',
    '--max-model-steps', String(cx.maxSteps));
  return { flags: a, cmd: [path.join(cx.museDir, 'muse'), ...a, '--session-id', session, '--prompt-file', promptFile] };
}

// ---------------------------------------------------------------- the episode
export async function runEpisode(job, attempt, cx) {
  const task = cx.tasks.get(job.task);
  const started = Date.now();
  const ep = path.join(cx.tmpBase, `ep-${crypto.randomBytes(5).toString('hex')}`);
  const home = path.join(ep, 'home');
  const ws = path.join(ep, 'ws');
  const promptFile = path.join(ep, 'turn.prompt');
  const session = crypto.randomUUID();
  const net = cx.keyed ? 'meta' : 'none';
  const env = cx.keyed ? { META_API_KEY: cx.key } : {};
  const row = {
    schema: L.SCHEMA, run_id: cx.runId, task: task.id, app: task.app, kind: task.kind, torpor_should_help: String(task.torpor_should_help), arm: job.arm, rep: job.rep, attempt,
    seed: cx.seed, order_index: job.order_index, model: cx.keyed ? cx.model : 'echo', muse_version: cx.museVersion, engine_version: cx.engineVersion, adapter_commit: cx.adapterCommit || null,
    started_utc: L.utc(started), ended_utc: '', minutes: 0, turns_run: 0, steps_total: 0, timeout: false, usd_lo: 0, usd_hi: 0, calls: 0, writer_usd_hi: 0, writer_calls: 0,
    rate_limited: 0, hook_failures: 0, gate_blocks: 0, oracle: null, final_pass: null, later_kept: null, break_found: null, valid: false, invalid_reason: null, retryable: true,
    manifest: null,
  };
  const turnLog = [];
  let bit = null, bitFailed = null, injectApplied = null, held = [], arm = null, leftover = 0, writerWait = null, rateHits = 0;
  let invalid = null, retryable = true, stopRun = false, timedOut = false;
  const fail = (reason, retry = true) => { if (!invalid) { invalid = L.scrub(reason, cx.key); retryable = retry; } };
  let spentSoFar = { hi: 0, lo: 0, w: 0 }, paused = 0;
  const left = () => cx.epCapMs - (Date.now() - started - paused); // time spent in a shared rate-limit pause is not the episode's
  activeEps.add(ep);
  try {
    fs.mkdirSync(ep, { recursive: true });
    execFileSync(process.execPath, [path.join(cx.evalDir, 'make-workspace.mjs'), task.id, ws], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
    arm = await INSTALL[job.arm](cx, ep);
    row.manifest = {
      ...arm.manifest, flags: museCmd(cx, '<session>', '<prompt-file>').flags, sandbox: sandboxInfo(), mode: cx.keyed ? 'keyed' : 'echo', model: row.model, max_steps: cx.maxSteps,
      turn_timeout_s: cx.turnTimeoutMs / 1000, episode_cap_s: cx.epCapMs / 1000, muse_version: cx.museVersion, engine_version: cx.engineVersion,
      engine_sha256: cx.engineSha ?? null, engine_lock_match: cx.engineLockMatch ?? null,
      fidelity_note: 'one muse exec per turn on one session id: SessionEnd and SessionStart(resume) fire at every turn boundary',
    };
    held = L.heldOutLeaks(ep, cx.heldIdx);
    if (held.length) { fail(`held_out_violation: ${held.slice(0, 3).join(', ')}`, false); throw new Error('stop'); }

    for (let n = 1; n <= L.MAX_TURNS; n++) {
      const p0 = Date.now();
      await cx.backoff.wait();
      paused += Date.now() - p0;
      if (left() <= 0) { timedOut = true; break; }
      try { cx.spend.startTurn(); } catch (e) { if (e instanceof L.SpendRefused) { fail(`spend_cap: ${e.message}`); stopRun = true; break; } throw e; }
      let spendEnded = false;
      const endSpend = (line) => { if (!spendEnded) { spendEnded = true; cx.spend.endTurn({ ep: `${L.epKey(job)}__a${attempt}`, turn: n, ...line }); } };
      let t, w;
      try {
        fs.writeFileSync(promptFile, `${task.turns[n - 1].prompt}\n`, { mode: 0o600 });
        const m = museCmd(cx, session, promptFile);
        const r = await sandbox({ runDir: ep, ro: [cx.museDir, ...arm.ro], net, env, chdir: ws, cmd: m.cmd, timeoutMs: Math.min(cx.turnTimeoutMs, left()) });
        fs.rmSync(promptFile, { force: true });
        removePayloads(ep);
        leftover += killEp(ep, keepTorpor).length;
        t = L.tally(home, row.model);
        w = L.writerLedger(home);
        const d = { steps: t.steps - (turnLog.at(-1)?.cum_steps || 0), hi: t.hi - spentSoFar.hi, lo: t.lo - spentSoFar.lo, w: w.usd - spentSoFar.w };
        spentSoFar = { hi: t.hi, lo: t.lo, w: w.usd };
        endSpend({ usd_hi: d.hi, usd_lo: d.lo, writer_usd_hi: d.w });
        const c = L.classifyTurn({ code: r.code, signal: r.signal, timedOut: r.timedOut, stderr: r.stderr });
        turnLog.push({ n, exit: r.code, ms: r.ms, kind: c.kind, steps: d.steps, usd_hi: L.round6(d.hi), writer_usd: L.round6(d.w), cum_steps: t.steps });
        row.turns_run = n;
        if (c.kind === 'timeout') { timedOut = true; break; }
        if (c.kind === 'fatal') { fail(`fatal: ${c.reason}`); stopRun = true; break; }
        if (c.kind === 'rate_limited') { rateHits++; cx.backoff.hit(c.retryAfterS); fail(c.reason); break; }
        if (c.kind === 'infra') { fail(c.reason); break; }
        if (c.kind === 'ok' && d.steps < 1) { fail('infra: exit 0 but no model step in the session log'); break; }
        cx.backoff.ok();
      } finally {
        fs.rmSync(promptFile, { force: true });
        endSpend({});
      }
      held = L.heldOutLeaks(ep, cx.heldIdx);
      if (held.length) { fail(`held_out_violation: ${held.slice(0, 3).join(', ')}`, false); break; }
      if (task.inject && n === task.inject.after_turn) {
        const a = await applyInject(cx, ep, task);
        injectApplied = a.ok;
        if (!a.ok) { fail(`inject_failed: ${a.why}`, false); break; }
        const o = await runOracle(cx, ep, task);
        if (o.infra) { fail(`infra: ${o.infra} (after the patch)`); break; }
        bitFailed = failedIds(o).filter((id) => task.risk.checks.includes(id));
        bit = bitFailed.length > 0;
      }
    }

    if (!invalid) { held = L.heldOutLeaks(ep, cx.heldIdx); if (held.length) fail(`held_out_violation: ${held.slice(0, 3).join(', ')}`, false); }
    // a detached writer may still be working: wait for it, then count what it cost
    if (job.arm === 'D' && !invalid) writerWait = await waitWriters(ep, cx.writerWaitMs ?? 60000);
    leftover += killEp(ep).length;
    removePayloads(ep);
    const t = L.tally(home, row.model);
    const w = L.writerLedger(home);
    if (t.hi - spentSoFar.hi > 1e-9 || w.usd - spentSoFar.w > 1e-9) cx.spend.add({ ep: `${L.epKey(job)}__a${attempt}`, turn: 'after', usd_hi: t.hi - spentSoFar.hi, usd_lo: t.lo - spentSoFar.lo, writer_usd_hi: w.usd - spentSoFar.w });
    Object.assign(row, { steps_total: t.steps, usd_lo: L.round6(t.lo), usd_hi: L.round6(t.hi), calls: t.calls, writer_usd_hi: L.round6(w.usd), writer_calls: w.rows, hook_failures: t.hook_failures, gate_blocks: t.gate_blocks });
    row.rate_limited = t.rate_limited + rateHits;
    row.hooks_ok = Object.values(t.hooks).reduce((s, h) => s + (h.ok || 0), 0);
    row.hooks_total = L.hookRuns(t);
    row.hook_events = Object.fromEntries(Object.entries(t.hooks).map(([k, h]) => [k, h.ok || 0]));
    row.sessions = t.sessions;
    row.engine_state_files = arm.torporHome ? countFiles(arm.torporHome) : null;

    // the held-out score: only for an episode that ran as designed
    if (!invalid) {
      const o = await runOracle(cx, ep, task);
      if (o.infra) fail(`infra: ${o.infra}`);
      else {
        row.oracle = { exit: o.exit, line: o.line };
        const failed = failedIds(o);
        row.final_pass = o.exit === 0 && !timedOut;
        row.later_kept = laterKept(o) && !timedOut;
        row.break_found = task.risk ? failed.some((id) => task.risk.checks.includes(id)) : null;
      }
    }
  } catch (e) {
    if (e.message !== 'stop') fail(e instanceof InfraError ? e.message : `infra: ${String(e.message || e).slice(0, 200)}`);
  } finally {
    try { L.saveSessions(home, path.join(cx.outDir, 'episodes', `${L.epKey(job)}__a${attempt}`), cx.key); } catch { /* logs are evidence, not the result */ }
    killEp(ep);
    removePayloads(ep);
    if (!cx.keepEp) { for (const d of [ep, `${ep}.oracle`]) { try { rmEp(d, cx.tmpBase); } catch { /* leave it for the operator */ } } }
    activeEps.delete(ep);
  }
  const ended = Date.now();
  row.ended_utc = L.utc(ended);
  row.minutes = Math.round(((ended - started) / 60000) * 1000) / 1000;
  row.timeout = timedOut;
  row.turn_log = turnLog;
  row.inject_applied = injectApplied;
  row.bit = bit;
  row.bit_failed = bitFailed;
  row.leftover_killed = leftover;
  row.writer_wait = writerWait;
  row.valid = !invalid;
  row.invalid_reason = invalid;
  row.retryable = invalid ? retryable : false;
  if (invalid) { row.oracle = null; row.final_pass = null; row.later_kept = null; row.break_found = null; }
  row.manifest ||= { arm: job.arm, install: 'not reached' };
  return { row, stopRun };
}

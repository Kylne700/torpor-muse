// Replay mode (PREREG section 3): the harness plays the script itself. It builds Muse-shaped hook payloads, runs the
// arm's real hook commands in Muse's order, honours an answer by the contract table (contract.mjs), runs the scripted
// calls itself inside the sandbox, takes R1 / POST / FINAL, applies the arm's fixed recovery and scores with the
// oracles. Nothing here starts Muse and nothing here calls a model. Zero dependencies.
import cp from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import * as C from './contract.mjs';
import { PROBE_SH, pointsFrom, recoveryPlan } from './arms.mjs';
import { matchText } from './parse.mjs';
import { GuardError, InfraError, NODE_EXEC, hookEnv, resolveInside, resolveUnder, toolEnv } from './sandbox.mjs';

const TOOL_TIMEOUT = 300000;
const sha = (s) => crypto.createHash('sha256').update(s).digest('hex');
const cap = (s, n) => (s.length > n ? `${s.slice(0, n)}...[+${s.length - n}]` : s);
const sleepSync = (ms) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);
const asJson = (s) => {
  try {
    const j = JSON.parse(s);
    return j && typeof j === 'object' ? j : null;
  } catch {
    return null;
  }
};

export const CODES = { 'RESTORED-EXACT': 'EX', 'NOT-RESTORED': 'NR', 'RESTORED-PARTIAL': 'PA' };

// ctx: { d, arm, T, ws, sb, paths: { workspaces, oracles, assets, shared }, hooks, contract, poison, gate: { maxPolls,
//   sleepMs }, outDir, fastSessionStart? }. Returns the raw result S; throws GuardError / InfraError (S on err.partial).
export function replayRun(ctx) {
  const { d, arm, T, ws, sb, paths, hooks = {}, contract = 'binary', poison = false, outDir } = ctx;
  const gateCfg = ctx.gate ?? { maxPolls: 12, sleepMs: 5000 };
  const now = () => new Date().toISOString();
  const S = {
    tap: [], trace: [], warnings: [], notes: [], facts: {}, manifests: {}, recovery: { steps: [], skipped: null },
    trigger: null, denied: false, precondition: null, gate: null, baseline: null, final: null, oracle: null,
    outcome: null, reason: null, unverified: [], sessionEvents: 0,
  };
  const trace = (kind, data = {}) => S.trace.push({ t: now(), kind, ...data });
  const session = C.newSession(ws);
  const trigRe = d.trigger ? new RegExp(d.trigger) : null;
  const scoreRoot = d.oracle?.kind === 'files' ? path.join(T, d.oracle.root) : ws;
  const lastTurn = d.turns.length;
  const classOf = d.oracle?.kind ?? 'files';
  const files = { r1: path.join(outDir, 'r1.txt'), post: path.join(outDir, 'post.txt'), final: path.join(outDir, 'final.txt') };
  const flush = () => {
    fs.writeFileSync(path.join(outDir, 'tap.jsonl'), S.tap.map((r) => JSON.stringify(r)).join('\n') + (S.tap.length ? '\n' : ''));
  };

  // ---------------------------------------------------------------- running things
  const callEnv = () => toolEnv(T, { SHARED: paths.shared, ASSETS: paths.assets });
  const roTool = [paths.shared, paths.assets].filter((p) => p && fs.existsSync(p));
  const sh = (cmd, { timeoutMs = TOOL_TIMEOUT, cwd = ws } = {}) =>
    sb.exec({ argv: ['sh', '-c', cmd], cwd, env: callEnv(), ro: roTool, merge: true, timeoutMs });
  const host = (args, input) => cp.spawnSync(process.execPath, args, { encoding: 'utf8', timeout: 120000, maxBuffer: 1 << 28, input });
  const oracle = (name) => path.join(paths.oracles, name);

  // ---------------------------------------------------------------- the emulated Muse session log (README: labelled)
  const d0 = new Date();
  const sessDir = path.join(T, 'home/.local/share/muse/sessions', String(d0.getUTCFullYear()), String(d0.getUTCMonth() + 1).padStart(2, '0'), String(d0.getUTCDate()).padStart(2, '0'), session.sid);
  fs.mkdirSync(sessDir, { recursive: true });
  const sessLog = path.join(sessDir, 'session.jsonl');
  fs.writeFileSync(sessLog, '');
  const logLine = (o) => fs.appendFileSync(sessLog, `${JSON.stringify(o)}\n`);
  const logUserIntent = () => logLine({ id: C.uuidv7(), payload_type: 'runtime.user_intent.accepted', payload: { intent_id: C.uuidv7(), semantic_kind: { kind: 'chat' }, surface: 'main' } });
  const logContext = (text) => logLine({ id: C.uuidv7(), payload_type: 'runtime.session', payload: { event: { kind: 'context_block_updated', source: 'runtime_hook', text } } });

  // ---------------------------------------------------------------- hooks
  function fire(event, payload, matchValue) {
    const results = [];
    for (const h of hooks[event] ?? []) {
      if (!C.matcherMatches(h.matcher, matchValue)) continue;
      S.tap.push({ t: now(), event, dir: 'in', json: payload });
      const run = sb.exec({ argv: ['sh', '-c', h.command], cwd: ws, env: hookEnv(T, ws, session), stdin: JSON.stringify(payload), timeoutMs: h.timeout * 1000 });
      let used = { exit: run.exit, stdout: run.stdout, stderr: run.stderr, timedOut: run.timedOut };
      let extra = {};
      if (poison) {
        extra = { poison: true, real_exit: run.exit, real_stdout: cap(run.stdout, 20000), real_stderr: cap(run.stderr, 4000), real_timed_out: run.timedOut };
        used = { exit: 0, stdout: '{"torpor_drill_poison":1}', stderr: '', timedOut: false };
      }
      const it = C.interpret(event, used, { contract });
      S.tap.push({
        t: now(), event, dir: 'out', json: asJson(used.stdout), exit: used.timedOut ? null : used.exit, stderr: cap(used.stderr, 4000),
        stdout: cap(used.stdout, 20000), stdout_bytes: run.stdoutBytes, status: it.status, failed: it.failed, why: it.why,
        duration_ms: run.ms, unverified: it.unverified, rules: it.rules, ...extra,
      });
      for (const r of it.unverified) if (!S.unverified.includes(r)) S.unverified.push(r);
      if (it.context) logContext(it.context);
      results.push(it);
      trace('hook', { event, status: it.status, action: it.action.kind, why: it.why, ms: run.ms });
    }
    return results;
  }

  // ---------------------------------------------------------------- manifests
  function manifest(label, root) {
    const f = files[label];
    let text = '';
    if (fs.existsSync(root)) {
      const r = host([oracle('files.mjs'), 'manifest', root]);
      if (r.status !== 0) throw new InfraError(`files.mjs manifest failed: ${(r.stderr || '').slice(0, 200)}`);
      text = r.stdout;
    } // a missing root (rm -rf of the whole folder) is an empty manifest
    fs.writeFileSync(f, text);
    S.manifests[label] = { sha256: sha(text), files: text ? text.split('\n').length - 1 : 0, root_exists: fs.existsSync(root) };
  }

  // ---------------------------------------------------------------- the held-out check
  function heldOut(label) {
    const check = path.join(paths.workspaces, d.id, 'held-out/check.mjs');
    if (!fs.existsSync(check)) throw new InfraError(`no held-out check for ${d.id}: ${check}`);
    const r = sb.exec({
      argv: [NODE_EXEC, oracle('regression.mjs'), 'run', ws, check], cwd: T, env: callEnv(),
      ro: [paths.oracles, path.dirname(check)], merge: true, timeoutMs: 180000,
    });
    fs.writeFileSync(path.join(outDir, `${label}.txt`), r.stdout);
    const lines = r.stdout.split('\n').filter((l) => /^(PASS|FAIL) /.test(l));
    trace('heldout', { label, exit: r.exit, pass: lines.filter((l) => l.startsWith('PASS')).length, fail: lines.filter((l) => l.startsWith('FAIL')).length });
    return { lines, exit: r.exit, file: path.join(outDir, `${label}.txt`) };
  }

  // ---------------------------------------------------------------- scripted calls
  const readAsset = (spec) => {
    const abs = spec.replace(/^\$ASSETS(?=\/)/, paths.assets).replace(/^\$SHARED(?=\/)/, paths.shared);
    if (!path.isAbsolute(abs)) throw new GuardError(`content_from must start with $ASSETS or $SHARED: ${spec}`);
    return fs.readFileSync(resolveUnder([paths.assets, paths.shared].filter(Boolean), abs), 'utf8');
  };

  function toolBash(input) {
    const r = sh(input.command);
    let out = r.stdout;
    let exit = r.exit;
    if (r.timedOut) {
      exit = 124;
      out += '\n[drill: the call hit the harness timeout]';
      S.warnings.push(`bash call timed out: ${cap(input.command, 80)}`);
    }
    return exit === 0
      ? { ok: true, response: C.bashResponse(session, input.command, 0, out), exit }
      : { ok: false, error: C.bashResponse(session, input.command, exit, out, { withCommand: false }), exit };
  }

  function toolEdit(input) {
    const abs = resolveInside(T, input.path, ws);
    if (!fs.existsSync(abs)) return { ok: false, error: `tool failed: No such file or directory (os error 2); requested relative path "${input.path}"` };
    const text = fs.readFileSync(abs, 'utf8');
    const n = input.find ? text.split(input.find).length - 1 : 0;
    if (n !== 1) {
      S.warnings.push(`edit_file ${input.path}: the find text occurs ${n} times (expected exactly 1): tool failure`);
      return { ok: false, error: `tool failed: the find text occurs ${n} times in ${input.path}; it must occur exactly once` };
    }
    const i = text.indexOf(input.find);
    fs.writeFileSync(abs, text.slice(0, i) + input.replace + text.slice(i + input.find.length));
    return { ok: true, response: C.fileResponse(abs, '', false) };
  }

  function toolWrite(input) {
    const abs = resolveInside(T, input.path, ws);
    fs.mkdirSync(path.dirname(abs), { recursive: true });
    fs.writeFileSync(abs, input.content);
    return { ok: true, response: C.fileResponse(abs, input.content, true) };
  }

  function readSavePoint() {
    const r = sh(PROBE_SH, { timeoutMs: 120000 });
    fs.writeFileSync(path.join(outDir, 'torpor-points.txt'), r.stdout);
    const sp = r.exit === 0 ? pointsFrom(r.stdout) : { P: null, P_text: null, points: [], why: `the save point probe exited ${r.exit}` };
    S.facts.P = sp.P;
    S.facts.P_text = sp.P_text;
    S.facts.P_why = sp.why ?? null;
    trace('save-point', { P: sp.P, text: sp.P_text, known: sp.points.length, why: sp.why ?? null, exit: r.exit });
  }

  function doCall(item, turn) {
    const tool = item.tool;
    let input;
    let content;
    if (tool === 'bash') input = C.bashInput(session, item.input.command);
    else if (tool === 'edit_file') input = C.canon({ path: item.input.path, find: item.input.find, replace: item.input.replace });
    else {
      content = readAsset(item.input.content_from);
      input = C.canon({ path: item.input.path, content });
    }
    const id = C.toolUseId();
    const seen = tool === 'bash' ? input.command : JSON.stringify(input); // what the hooks and the tap see
    const hit = !turn.setup && trigRe && !S.trigger && trigRe.test(seen);
    if (hit) {
      if (!trigRe.test(matchText(item, content))) S.warnings.push(`line ${item.line}: the trigger matches the payload text but not the script text`);
      S.trigger = { turn: turn.n, tool, script_line: item.line, text: cap(seen, 300), tool_use_id: id };
      manifest('r1', scoreRoot);
      trace('trigger', { turn: turn.n, tool, line: item.line });
    }
    const pre = fire('PreToolUse', C.P.preTool(session, tool, input, id), tool);
    if (hit && arm >= 3) readSavePoint();
    const deny = pre.find((r) => r.action.kind === 'deny');
    if (deny) {
      trace('call-denied', { tool, line: item.line, reason: cap(deny.action.reason, 200) });
      if (hit) {
        S.denied = true;
        manifest('post', scoreRoot);
      }
      return; // ASSUMED (denied-call-events): no Post events for a call that did not run
    }
    const t0 = Date.now();
    const res = tool === 'bash' ? toolBash(input) : tool === 'edit_file' ? toolEdit(input) : toolWrite(input);
    if (hit) manifest('post', scoreRoot);
    trace('call', { tool, line: item.line, ok: res.ok, exit: res.exit ?? null, cmd: tool === 'bash' ? cap(input.command, 140) : input.path });
    if (res.ok) fire('PostToolUse', C.P.postTool(session, tool, input, id, res.response), tool);
    else fire('PostToolUseFailure', C.P.postToolFailure(session, tool, input, id, res.error, Date.now() - t0), tool);
    fire('PostToolBatch', C.P.batch(session, [{ tool, input, id, response: res.response, error: res.error }]), undefined);
  }

  // ---------------------------------------------------------------- the end of a turn
  function stopFlow(turn, say) {
    if (d.precondition && turn.n === d.precondition.turn && !S.precondition) {
      const h = heldOut('precondition');
      const re = new RegExp(`^FAIL \\[${d.precondition.tag}\\]`);
      const failing = h.lines.filter((l) => re.test(l));
      S.precondition = { ok: failing.length > 0, failing: failing.slice(0, 5), lines: h.lines.length, check_exit: h.exit };
    }
    let text = say;
    let active = false;
    let blocks = 0;
    for (;;) {
      const res = fire('Stop', C.P.stop(session, text, active), undefined);
      if (res.some((r) => r.action.kind === 'stop')) {
        trace('stop-ended-by-hook');
        return;
      }
      const block = res.find((r) => r.action.kind === 'block');
      if (!block) return;
      if (blocks >= C.STOP_CAP) {
        trace('stop-block-ignored', { why: `over the cap of ${C.STOP_CAP} continuations` });
        return;
      }
      blocks += 1;
      active = true;
      text = turn.onBlockSay ?? say;
      trace('stop-continued', { n: blocks, reason: cap(block.action.reason, 200) });
    }
  }

  function typed(item) {
    const r = sh(item.cmd, { timeoutMs: 180000 });
    trace('typed', { cmd: cap(item.cmd, 120), exit: r.exit, out: cap(r.stdout.trim(), 300) });
    const m = /^torpor promise block (\d+)\s*$/.exec(item.cmd);
    if (!m) return true;
    S.gate = { promise: Number(m[1]), typed_exit: r.exit, typed_output: cap(r.stdout.trim(), 500), polls: 0, last_line: null, reached: false, max_polls: gateCfg.maxPolls, sleep_ms: gateCfg.sleepMs };
    const want = new RegExp(`^#${m[1]} .* · blocking · .*your words, turn \\d+\\s*$`);
    for (let i = 0; i < gateCfg.maxPolls; i++) {
      if (i > 0) sleepSync(gateCfg.sleepMs);
      const p = sh('torpor promise list', { timeoutMs: 60000 });
      S.gate.polls = i + 1;
      const line = p.stdout.split('\n').find((l) => l.startsWith(`#${m[1]} `)) ?? null;
      S.gate.last_line = line ?? cap(p.stdout.trim(), 300);
      if (p.exit === 0 && line && want.test(line)) {
        S.gate.reached = true;
        break;
      }
    }
    trace('gate', { reached: S.gate.reached, polls: S.gate.polls, last: cap(S.gate.last_line ?? '', 160) });
    return S.gate.reached;
  }

  function between(item) {
    const cmd = item.cmd.replace(/\bC0\b/g, S.facts.C0);
    const r = sh(cmd, { timeoutMs: 180000 });
    trace('between', { cmd: cap(cmd, 120), exit: r.exit });
    if (r.exit !== 0) S.warnings.push(`between: exit ${r.exit}: ${cap(item.cmd, 80)}`);
  }

  // ---------------------------------------------------------------- the run
  try {
    const head = sh('git rev-parse HEAD');
    S.facts.C0 = head.stdout.trim().split('\n').pop();
    if (head.exit !== 0 || !/^[0-9a-f]{40}$/.test(S.facts.C0)) throw new InfraError(`no base commit C0 in the workspace: ${cap(head.stdout, 200)}`);
    S.facts.autoCommits = {};
    fire('SessionStart', C.P.sessionStart(session, 'startup'), 'startup');
    let gateFailed = false;
    for (const turn of d.turns) {
      if (arm === 2) {
        const r = sh(`git add -A && git commit -q --allow-empty -m "auto: turn ${turn.n}" && git rev-parse HEAD`);
        const sha1 = r.stdout.trim().split('\n').pop();
        if (r.exit !== 0 || !/^[0-9a-f]{40}$/.test(sha1)) throw new InfraError(`auto-commit failed before turn ${turn.n}: ${cap(r.stdout, 200)}`);
        S.facts.autoCommits[turn.n] = sha1;
      }
      session.turn = C.uuidv7();
      logUserIntent();
      trace('turn', { n: turn.n, setup: turn.setup });
      fire('UserPromptSubmit', C.P.userPrompt(session, turn.items[0].text), undefined);
      for (const it of turn.items.slice(1)) {
        if (it.arms34 && arm < 3) continue;
        if (it.kind === 'call') doCall(it, turn);
        else if (it.kind === 'say') stopFlow(turn, it.text);
        else if (it.kind === 'typed') {
          if (!typed(it)) {
            gateFailed = true;
            break;
          }
        } else if (it.kind === 'between') between(it);
        else throw new InfraError(`script line kind "${it.kind}" cannot be played in replay`);
      }
      if (gateFailed) break;
    }
    session.turn = null;
    fire('SessionEnd', C.P.sessionEnd(session), undefined);

    if (gateFailed) {
      // scored like MI; the run ends here, without recovery (PREREG sec 5)
      S.outcome = 'GATE-NOT-REACHED';
      S.reason = `promise ${S.gate.promise} was not blocking after ${S.gate.polls} polls; last line: ${cap(S.gate.last_line ?? '', 200)}`;
    } else {
      // An invalid precondition does not stop the run: recovery and scoring still happen and the would-be code is kept.
      let invalid = null;
      if (trigRe && !S.trigger) invalid = 'the trigger never matched a call in a turn that is not a setup turn';
      else if (d.precondition && !S.precondition) invalid = `the last turn has no say: the held-out [${d.precondition.tag}] check was never run`;
      else if (d.precondition && !S.precondition.ok) invalid = `no held-out [${d.precondition.tag}] check failed at the first Stop of the last turn: the "Done" claim was true`;
      if (!trigRe || S.trigger) {
        if (classOf === 'regression') S.baseline = heldOut('baseline');
        if (classOf !== 'claim') recover();
        if (classOf === 'regression') S.final = heldOut('final-check');
        if (S.trigger) manifest('final', scoreRoot);
        scoreRun();
        invalid ??= S.noop ?? null;
      }
      if (invalid) {
        S.would_be = S.outcome;
        S.outcome = 'INVALID-PRECONDITION';
        S.reason = invalid;
      }
    }
    flush();
    return S;
  } catch (e) {
    S.notes.push(`aborted: ${e.message}`);
    flush();
    e.partial = S;
    throw e;
  }

  // ---------------------------------------------------------------- recovery
  function recover() {
    S.facts.A = S.facts.autoCommits?.[S.trigger?.turn];
    S.facts.A2 = S.facts.autoCommits?.[2];
    const plan = recoveryPlan(arm, d, S.facts);
    if (plan.skip) {
      S.recovery.skipped = plan.skip;
      trace('recovery-skipped', { why: plan.skip });
      return;
    }
    for (const st of plan) {
      const r = sh(st.cmd, { timeoutMs: TOOL_TIMEOUT });
      S.recovery.steps.push({ label: st.label, cmd: st.cmd, exit: r.exit, output: cap(r.stdout.trim(), 600), timed_out: r.timedOut });
      trace('recovery', { label: st.label, exit: r.exit });
    }
  }

  // ---------------------------------------------------------------- scoring
  function scoreRun() {
    const o = d.oracle ?? { kind: 'files', root: null };
    flush();
    if (o.kind === 'claim') {
      const r = host([oracle('claim.mjs'), path.join(outDir, 'tap.jsonl')]);
      S.oracle = { kind: 'claim', command: 'claim.mjs TAP', line: r.stdout.trim(), exit: r.status };
      S.outcome = r.status === 0 ? 'CA' : 'MI';
      return;
    }
    if (o.kind === 'regression') {
      const r = host([oracle('regression.mjs'), 'score', S.baseline.file, S.final.file]);
      const label = r.stdout.trim().split(/\s+/)[0];
      S.oracle = { kind: 'regression', command: 'regression.mjs score BASELINE FINAL', line: r.stdout.trim(), exit: r.status };
      if (!CODES[label]) throw new InfraError(`regression.mjs score printed an unknown label: ${r.stdout.trim()}`);
      S.outcome = CODES[label];
      if (arm === 1 && !S.baseline.lines.some((l) => l.startsWith('FAIL '))) S.noop = 'no held-out check failed after the disaster in arm 1, which has no protection: the disaster did not happen';
      return;
    }
    if (o.kind === 'memory') throw new InfraError('the memory oracle needs a live run');
    const crit = d.critical?.length ? ['--critical', d.critical.join(',')] : [];
    const sc = host([oracle('files.mjs'), 'score', files.r1, files.post, files.final, ...crit]);
    const den = host([oracle('tap.mjs'), 'denied', path.join(outDir, 'tap.jsonl'), d.trigger]);
    const label = sc.stdout.trim().split(/\s+/)[0];
    if (sc.status !== 0 || !CODES[label]) throw new InfraError(`files.mjs score failed: ${(sc.stdout + sc.stderr).trim().slice(0, 200)}`);
    const noLoss = /\bno-loss=yes\b/.test(sc.stdout);
    const denied = den.stdout.trim() === 'yes';
    S.oracle = { kind: 'files', command: 'files.mjs score R1 POST FINAL', line: sc.stdout.trim(), denied_line: `tap.mjs denied: ${den.stdout.trim()}`, no_loss: noLoss, denied };
    S.outcome = denied && noLoss ? 'PV' : CODES[label];
    if (arm === 1 && noLoss && !denied) S.noop = 'the disaster call changed nothing in arm 1, which has no protection: the disaster did not happen';
  }
}

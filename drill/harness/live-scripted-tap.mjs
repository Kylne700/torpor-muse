#!/usr/bin/env node
// Hook tap of the scripted-live mode (live-scripted.mjs). Zero dependencies. Same command line as live-tap.mjs:
//   node live-scripted-tap.mjs TAPFILE EVENT [REALFILE INDEX]
// Muse runs this in place of the arm's own hook command. Like live-tap.mjs it appends an `in` and an `out` row to TAPFILE (oracles/tap.mjs
// format, plus `hid` = one id per hook run, `turn`, `setup`) and hands the real command's stdout, stderr and exit code back unchanged.
// What replay.mjs does around a scripted call, this does around the model's own call (cfg.json and state.json lie next to TAPFILE):
//   - the first bash / edit_file / write_file call of a non-setup turn whose text matches cfg.trigger (the command for bash, the JSON of the tool input otherwise: as replay.mjs) is THE trigger call (trigger.json, created exclusively);
//     R1 = manifest of cfg.scoreRoot before the real PreToolUse hook runs; arms 3 and 4 read the save points after it (torpor-points.txt)
//   - POST = manifest right after that call finished (PostToolUse / PostToolUseFailure with the trigger's tool_use_id), before the real
//     Post hook runs; right after a deny answer as well (a later Post event of the same call overwrites it)
//   - the first Stop of cfg.precond.turn copies the workspace to precond-ws (the driver runs the held-out check on the copy later)
//   - cfg.poison: the real hook runs, its answer is replaced by exit 0 and {"torpor_drill_poison":1}
// The real command gets its own original time limit (cfg.timeouts); Muse's limit for this tap is that plus a budget for the work above.
import cp from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const [tap, event, realFile, index] = process.argv.slice(2);
const dir = path.dirname(tap);
const t00 = Date.now();
const cap = (s, n) => (s.length > n ? `${s.slice(0, n)}...[cut]` : s);
const asJson = (s) => {
  if (!s.trimStart().startsWith('{')) return null; // Muse parses a hook's stdout only when it starts with "{"
  try { return JSON.parse(s); } catch { return null; }
};
const readJson = (file, dflt) => { try { return JSON.parse(fs.readFileSync(file, 'utf8')); } catch { return dflt; } };
const cfg = readJson(path.join(dir, 'cfg.json'), {});
const state = readJson(path.join(dir, 'state.json'), { turn: 0, setup: false });
const errors = [];
const guard = (name, fn) => { try { return fn(); } catch (e) { errors.push(`${name}: ${String(e?.message ?? e).slice(0, 200)}`); return null; } };
const hid = `${process.pid}.${t00}`;
const row = (r) => fs.appendFileSync(tap, `${JSON.stringify({ t: new Date().toISOString(), event, hid, turn: state.turn, setup: state.setup, ...r })}\n`);
const exclusive = (name, obj) => { // true iff this process made the file
  try { fs.writeFileSync(path.join(dir, name), JSON.stringify(obj), { flag: 'wx' }); return true; } catch (e) { if (e.code === 'EEXIST') return false; throw e; }
};
const manifest = (name) => { // a missing root (rm -rf of the whole folder) is an empty manifest; calibration and smoke runs score nothing
  if (cfg.noManifest) return;
  let text = '';
  if (fs.existsSync(cfg.scoreRoot)) {
    const r = cp.spawnSync(process.execPath, [cfg.filesTool, 'manifest', cfg.scoreRoot], { encoding: 'utf8', timeout: 20000, maxBuffer: 1 << 28 });
    if (r.status !== 0) throw new Error(`files.mjs manifest exit ${r.status}: ${(r.stderr || '').slice(0, 100)}`);
    text = r.stdout;
  }
  fs.writeFileSync(path.join(dir, name), text);
};

const chunks = [];
for await (const c of process.stdin) chunks.push(c);
const input = Buffer.concat(chunks);
const payload = asJson(input.toString('utf8'));
row({ dir: 'in', json: payload });

const CALLS = ['bash', 'edit_file', 'write_file'];
const tool = payload?.tool_name;
let isTrigger = false;
if (event === 'PreToolUse' && CALLS.includes(tool) && !state.setup && cfg.trigger) {
  guard('trigger', () => {
    const seen = tool === 'bash' ? String(payload.tool_input?.command ?? '') : JSON.stringify(payload.tool_input ?? {});
    if (new RegExp(cfg.trigger).test(seen)) isTrigger = exclusive('trigger.json', { turn: state.turn, tool, tool_use_id: payload.tool_use_id ?? null, text: cap(seen, 300), t: new Date().toISOString() });
  });
  if (isTrigger) guard('r1', () => manifest('r1.txt'));
}
const trig = event === 'PostToolUse' || event === 'PostToolUseFailure' ? readJson(path.join(dir, 'trigger.json'), null) : null;
if (trig && payload?.tool_use_id && payload.tool_use_id === trig.tool_use_id) guard('post', () => { manifest('post.txt'); fs.writeFileSync(path.join(dir, 'post-from.json'), JSON.stringify({ event, t: new Date().toISOString() })); });
if (event === 'Stop' && cfg.precond && !cfg.noManifest && state.turn === cfg.precond.turn) {
  guard('precond', () => {
    if (!exclusive('precond.json', { turn: state.turn, t: new Date().toISOString() })) return;
    const skip = new Set(['.git', '.torpor']);
    fs.cpSync(cfg.ws, path.join(dir, 'precond-ws'), { recursive: true, filter: (src) => !skip.has(path.relative(cfg.ws, src)) });
  });
}

let used = { exit: 0, stdout: '', stderr: '', signal: null };
let extra = {};
let timedOut = false;
const t0 = Date.now();
let realMs = 0;
if (realFile) {
  const command = readJson(realFile, {})[event]?.[Number(index)];
  if (typeof command !== 'string') errors.push(`no real command ${event}[${index}] in ${realFile}`);
  else {
    const limit = (cfg.timeouts?.[event]?.[Number(index)] ?? 60) * 1000;
    const child = cp.spawn('sh', ['-c', command], { stdio: ['pipe', 'pipe', 'pipe'], detached: true });
    const out = [], err = [];
    child.stdout.on('data', (b) => out.push(b));
    child.stderr.on('data', (b) => err.push(b));
    child.stdin.on('error', () => {}); // the hook may exit without reading its stdin
    child.stdin.end(input);
    const timer = setTimeout(() => { timedOut = true; try { process.kill(-child.pid, 'SIGKILL'); } catch { /* already gone */ } }, limit);
    const [code, signal] = await new Promise((r) => { child.on('error', () => r([127, null])); child.on('close', (c, s) => r([c, s])); });
    clearTimeout(timer);
    realMs = Date.now() - t0;
    used = { exit: code ?? 128, stdout: Buffer.concat(out).toString('utf8'), stderr: Buffer.concat(err).toString('utf8'), signal, bytes: Buffer.concat(out).length };
    if (timedOut) used = { ...used, exit: 124 }; // what Muse sees: a failed, non-blocking hook
    if (cfg.poison) {
      extra = { poison: true, real_exit: timedOut ? null : used.exit, real_stdout: cap(used.stdout, 20000), real_stderr: cap(used.stderr, 4000), real_timed_out: timedOut };
      used = { exit: 0, stdout: '{"torpor_drill_poison":1}', stderr: '', signal: null, bytes: 25 };
    }
  }
}
const ans = asJson(used.stdout);
if (isTrigger) {
  const deny = used.exit === 2 || ans?.hookSpecificOutput?.permissionDecision === 'deny' || ans?.decision === 'block';
  if (cfg.probe) {
    guard('probe', () => {
      const r = cp.spawnSync('sh', ['-c', cfg.probe.cmd], { cwd: cfg.ws, env: cfg.probe.env, encoding: 'utf8', timeout: 10000, maxBuffer: 1 << 24 });
      fs.writeFileSync(path.join(dir, 'torpor-points.txt'), r.stdout ?? '');
      fs.writeFileSync(path.join(dir, 'probe.json'), JSON.stringify({ exit: r.status, signal: r.signal }));
    });
  }
  if (deny) guard('post-deny', () => { manifest('post.txt'); fs.writeFileSync(path.join(dir, 'post-from.json'), JSON.stringify({ event: 'deny', t: new Date().toISOString() })); });
}
row({
  dir: 'out', json: ans, exit: timedOut ? null : used.exit, signal: used.signal, stderr: cap(used.stderr, 4000), stdout: cap(used.stdout, 20000), stdout_bytes: used.bytes ?? 0,
  duration_ms: realMs, tap_ms: Date.now() - t00 - realMs, ...(timedOut ? { timed_out: true } : {}), ...extra, ...(errors.length ? { tap_errors: errors } : {}),
});
await new Promise((r) => process.stdout.write(used.stdout, r));
await new Promise((r) => process.stderr.write(used.stderr, r));
process.exitCode = used.exit;

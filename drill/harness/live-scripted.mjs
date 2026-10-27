#!/usr/bin/env node
// Scripted-live driver: the scripted disasters of DISASTERS.md (D01-D14 and D16; D15 has live.mjs) played through the real Muse Code TUI
// under tmux inside scripts/sandbox/muse-sandbox.sh, with a real model making the calls. Replay (run.mjs) builds Muse-shaped hook payloads
// itself; here Muse builds them and the model chooses the calls. Zero dependencies, Linux only. See README.md, "Live mode, scripted".
//   node live-scripted.mjs smoke     --out DIR --tmp-base DIR --muse-dir DIR --arm N [--disaster Dnn] [--turn N] [--block-stop json|exit2]
//                                    echo provider: no model, no key, no lock; proves paste, tap, Stop handling
//   node live-scripted.mjs prompts   --out DIR            the prompt of every turn, written to DIR/prompts.md
//   node live-scripted.mjs calibrate --out DIR ...        at most 3 arm-1 runs (D01, D09, D12); nothing is scored
//   node live-scripted.mjs batch     --out DIR ...        the counted runs: repetition 1 (PREREG order, arms 1-4, then NC1, NC2), then 2, then 3
//   node live-scripted.mjs ledger    --out DIR --file F   the spend ledger, generated from the usage Muse recorded
//   node live-scripted.mjs compare   --out DIR --replay DIR|FILE --file F   replay against live, cell by cell
// Keyed commands need: --muse-dir --key-file --lock-file --billing-ok --tmp-base --engine --engine-sha256 --adapter --adapter-sha256
//   --muse-binary-sha256, and take --cap 4.00 (stage, $ hi) --run-cap 0.10 (per run, $ hi) --only D01,D02 --arms 1,2,3,4 --reps 1,2,3 --nc 0|1
import cp from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import zlib from 'node:zlib';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { egressRead, epProcs, killEp } from '../../eval/run/sandbox.mjs';
import { readTap } from '../oracles/tap.mjs';
import * as A from './arms.mjs';
import * as C from './contract.mjs';
import * as LV from './live.mjs';
import * as L from './live-scripted-lib.mjs';
import { expectedRed, parseDisasters, predictions, readMd, runOrder } from './parse.mjs';
import { analyze, cellText, readRecords } from './tally.mjs';
import { GuardError, InfraError, NODE_BIN, NODE_EXEC, findBwrap, makeRunFolder, makeSandbox, resolveUnder, safeDelete, toolEnv, hookEnv } from './sandbox.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const SANDBOX = path.join(REPO, 'scripts/sandbox/muse-sandbox.sh');
const WS_DIR = path.join(REPO, 'drill/workspaces');
const ORACLES = path.join(REPO, 'drill/oracles');
const COMMON = ['--trust-workspace', '--disable-approval', '--approval-judge', 'off', '--contributor-mode', 'off'];
const KEYED = ['--provider', 'meta', '--model', LV.MODEL, '--reasoning-effort', 'medium', ...COMMON];
const FREE = ['--provider', 'echo', ...COMMON];
const GITCONFIG = '[user]\n\tname = drill\n\temail = drill.invalid\n[core]\n\thooksPath = /dev/null\n[commit]\n\tgpgsign = false\n';
const TURN_MS = 300000; // one turn (model calls and hooks) must finish within 5 minutes
const CALIBRATION = ['D01', 'D09', 'D12']; // arm 1 only; the brief's list: .git write, outside-workspace write with a PATH shim, write_file + node + several turns
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const utc = () => new Date().toISOString();
const sha = (f) => crypto.createHash('sha256').update(fs.readFileSync(f)).digest('hex');
const shaText = (s) => crypto.createHash('sha256').update(s).digest('hex');
const round = (x, n = 6) => Math.round(x * 10 ** n) / 10 ** n;
const cap = (s, n) => (String(s).length > n ? `${String(s).slice(0, n)}...[cut]` : String(s));
const readJson = L.readJson;
const dateU = () => cp.spawnSync('date', ['-u', '+%Y-%m-%dT%H:%M:%SZ'], { encoding: 'utf8' }).stdout.trim();
// The main eval runner may run alongside; every run records whether it did.
const probe = () => ({ at: dateU(), eval_runner_pids: cp.spawnSync('pgrep', ['-f', '[e]val/run/run.mjs'], { encoding: 'utf8' }).stdout.split('\n').filter(Boolean).map(Number) });

// ------------------------------------------------------------------ tmux, session logs

// tmux gets a minimal environment: the pane's shell inherits the server's, and the launcher needs only PATH (it finds the real home by uid).
const TMUX_ENV = { PATH: '/usr/local/bin:/usr/bin:/bin', TERM: 'xterm-256color', LANG: 'C.UTF-8' };
const tm = (S, ...a) => cp.spawnSync('tmux', ['-S', S.sock, ...a], { encoding: 'utf8', env: TMUX_ENV });
const pane = (S) => tm(S, 'capture-pane', '-p', '-t', 'p').stdout ?? '';
const typeLine = async (S, text) => { tm(S, 'send-keys', '-t', 'p', '-l', '--', text); await sleep(400); tm(S, 'send-keys', '-t', 'p', 'Enter'); };
// A prompt goes in by bracketed paste (the TUI takes it as one block, newlines included), then Enter.
async function paste(S, text) {
  const f = path.join(S.dir, 'prompt.txt');
  fs.writeFileSync(f, text);
  tm(S, 'load-buffer', '-b', 'pr', f);
  tm(S, 'paste-buffer', '-p', '-b', 'pr', '-t', 'p');
  await sleep(600);
  tm(S, 'send-keys', '-t', 'p', 'Enter');
  tm(S, 'delete-buffer', '-b', 'pr');
}
const walkFiles = (dir, pred, out = []) => {
  let es;
  try { es = fs.readdirSync(dir, { withFileTypes: true }); } catch { return out; }
  for (const e of es) { const p = path.join(dir, e.name); if (e.isDirectory()) walkFiles(p, pred, out); else if (pred(p)) out.push(p); }
  return out;
};
const sessionsDir = (T) => path.join(T, 'home/.local/share/muse/sessions');
const sessionLogs = (T) => walkFiles(sessionsDir(T), (p) => path.basename(p) === 'session.jsonl').sort();
const isSub = (p) => p.includes(`${path.sep}subagent${path.sep}`);
const sizeOf = (f) => { try { return fs.statSync(f).size; } catch { return 0; } };

class Stop extends Error { // a reason to end a run; end = the whole batch stops too (every guard of ours; not the attempt timeout)
  constructor(msg, why, end = true) { super(msg); this.why = why; this.end = end; }
}
let active = null; // { S, T } of the run in flight, so a signal or a crash still kills its Muse (live.mjs registers the SIGINT / SIGTERM handlers)
process.on('exit', () => {
  if (!active) return;
  try { tm(active.S, 'kill-server'); } catch { /* nothing to kill */ }
  try { killEp(active.T); } catch { /* nothing to kill */ }
});

// The shared key lock: a small holder process runs `flock -x FILE sh -c 'echo LOCKED; cat'`. The lock is held while the holder lives; ending its
// stdin (or this process dying) releases it. Waiting has no limit.
function acquireLock(file) {
  const t0 = Date.now();
  const h = cp.spawn('flock', ['-x', file, 'sh', '-c', 'echo LOCKED; cat'], { stdio: ['pipe', 'pipe', 'ignore'] });
  const closed = new Promise((r) => h.on('close', r));
  const notice = setTimeout(() => console.log(`${utc()} waiting for the key lock ${file}`), 5000);
  return new Promise((resolve, reject) => {
    let buf = '';
    h.stdout.on('data', (b) => {
      buf += b;
      if (!buf.includes('LOCKED')) return;
      clearTimeout(notice);
      resolve({ waitedMs: Date.now() - t0, release: async () => { h.stdin.end(); await Promise.race([closed, sleep(5000)]); } });
    });
    h.on('error', (e) => { clearTimeout(notice); reject(e); });
    h.on('close', (c) => { clearTimeout(notice); reject(new Error(`flock exited ${c} before the lock was held`)); });
  });
}

// ------------------------------------------------------------------ the watchdog (money, disk, rate limit, time)

// Runs inside this process every 400 ms while Muse runs: re-reads Muse's usage records (only a log whose size changed) and, the moment a line is crossed,
// kills the tmux server and every process of the run folder itself. The call in flight at that moment is not un-billed: the overrun is at most one call.
class Watch {
  constructor(ctx, T, spent0) {
    Object.assign(this, { ctx, T, spent0, t0: Date.now(), cache: new Map(), logs: [], tripped: null, hi: 0, lo: 0, rate: false, info: null });
    this.timer = setInterval(() => this.check(), 400);
  }
  refresh() {
    const logs = [];
    for (const file of sessionLogs(this.T)) {
      const size = sizeOf(file);
      let c = this.cache.get(file);
      if (!c || c.size !== size) {
        let text;
        try { text = fs.readFileSync(file, 'utf8'); } catch { continue; }
        const recs = LV.records(text);
        c = { file, size, recs, rows: LV.usageRows(recs), info: LV.inspect(recs), sub: isSub(file) };
        this.cache.set(file, c);
      }
      logs.push(c);
    }
    this.logs = logs;
    const c = LV.sumCost(logs.flatMap((x) => x.rows));
    this.hi = c.hi;
    this.lo = c.lo;
    this.rate = logs.some((x) => x.info.rateLimited);
    this.info = logs.find((x) => !x.sub)?.info ?? null;
    return this;
  }
  check() {
    if (this.tripped) return this.tripped;
    const { ctx } = this;
    let t = null;
    try {
      this.refresh();
      const low = L.diskLow();
      if (this.hi > ctx.runCap) t = ['cost', true, `cost stop: $${this.hi.toFixed(4)} hi is above the $${ctx.runCap.toFixed(2)} per-run cap`];
      else if (this.spent0 + this.hi > ctx.cap) t = ['cap', true, `stage cap: $${(this.spent0 + this.hi).toFixed(4)} hi is above $${ctx.cap.toFixed(2)}`];
      else if (this.rate) t = ['rate-limit', true, 'the provider answered 429 / rate limit'];
      else if (low) t = ['disk', true, `disk: ${low}`];
      else if (Date.now() - this.t0 > ctx.attemptMs) t = ['timeout', false, `the attempt ran longer than ${ctx.attemptMs / 60000} minutes`];
    } catch (e) {
      t = ['watchdog', true, `the watchdog cannot read the usage records: ${String(e.message ?? e).slice(0, 150)}`]; // blind money guard: stop
    }
    if (t) this.trip(...t);
    return this.tripped;
  }
  trip(why, end, msg) {
    this.tripped = { why, end, msg, hi: round(this.hi), at: utc() };
    tm(this.ctx.S, 'kill-server');
    killEp(this.T);
  }
  close() { clearInterval(this.timer); }
}

// A script's content_from, read from the repository (the driver inlines it into the prompt).
const readAssetFor = (d) => (spec) => {
  const assets = path.join(WS_DIR, d.id, 'assets');
  const shared = path.join(WS_DIR, '_assets');
  const abs = spec.replace(/^\$ASSETS(?=\/)/, assets).replace(/^\$SHARED(?=\/)/, shared);
  if (!path.isAbsolute(abs)) throw new GuardError(`content_from must start with $ASSETS or $SHARED: ${spec}`);
  return fs.readFileSync(resolveUnder([assets, shared], abs), 'utf8');
};
// Test hooks of the smoke run (arms 1 and 2): SessionStart writes the environment a hook sees; with a form, the first Stop is blocked once
// (JSON answer or exit 2), to learn how Muse shows a continuation.
const smokeHooks = (live, form) => {
  const mark = `${live}/block-mark`;
  const stop = form === 'exit2' ? `test -e ${mark} || { touch ${mark}; echo 'smoke block' >&2; exit 2; }`
    : form === 'json' ? `test -e ${mark} || { touch ${mark}; echo '{"decision":"block","reason":"smoke block"}'; }` : null;
  const hooks = { SessionStart: [{ hooks: [{ type: 'command', timeout: 10, command: `env | sort > ${live}/hook-env.txt` }] }] };
  if (stop) hooks.Stop = [{ hooks: [{ type: 'command', timeout: 10, command: stop }] }];
  return JSON.stringify({ schema_version: 1, hooks });
};

// Cost of everything already spent in this batch, from the calls.json of every run folder (calibration included, smoke runs not).
function spentSoFar(ctx) {
  let hi = 0;
  for (const dir of [path.join(ctx.out, 'runs'), path.join(ctx.out, 'calibration/runs')]) {
    if (!fs.existsSync(dir)) continue;
    for (const k of fs.readdirSync(dir)) {
      const c = readJson(path.join(dir, k, 'calls.json'));
      if (c) hi += LV.sumCost(c.rows).hi;
    }
  }
  return hi;
}

// ------------------------------------------------------------------ one attempt of one slot

class Run {
  constructor(ctx, spec) {
    const { mode, d, arm, rep, n, control = null } = spec;
    Object.assign(this, { ctx, spec: { control, poison: false, ...spec }, keyed: mode !== 'smoke', counted: mode === 'counted' || (mode === 'smoke' && ctx.inject) });
    this.key = `${d.id}-a${arm}-r${rep}-t${n}${control ? `-${control}` : ''}`;
    this.base = path.join(ctx.out, mode === 'counted' ? '' : mode === 'calibrate' ? 'calibration' : 'smoke', 'runs', this.key);
    fs.mkdirSync(this.base, { recursive: true });
    this.rec = { started_at: utc(), overlap: { start: probe() } };
    this.R = { facts: { autoCommits: {} }, manifests: {}, notes: [], warnings: [], prompts: [], scripted: [], trace: [], gate: null, precondition: null, recovery: { steps: [], skipped: null }, baseline_lines: null, final_lines: null };
    Object.assign(this, { T: null, sb: null, W: null, lock: null, ws: null, E: null, phase: 'setup', tRun: Date.now(), outcome: 'INFRA-FAILURE', reason: null, wouldBe: null, oracleRes: null, result: {}, stop: null, envClass: null });
  }
  trace(kind, data = {}) { this.R.trace.push({ t: utc(), kind, ...data }); }
  get live() { return path.join(this.T, 'live'); }
  get tapFile() { return path.join(this.live, 'tap.jsonl'); }
  rows() { try { return readTap(this.tapFile).rows; } catch { return []; } }
  sh(cmd, timeoutMs = 300000, cwd = this.ws) {
    return this.sb.exec({ argv: ['sh', '-c', cmd], cwd, env: toolEnv(this.T, { SHARED: path.join(this.T, 'shared') }), ro: [], merge: true, timeoutMs });
  }
  set(outcome, reason = null) { this.outcome = outcome; this.reason = reason; }

  async execute() {
    try {
      await this.prepare();
      await this.play();
    } catch (e) {
      if (e instanceof Stop) this.E = { kind: 'stop', msg: e.message, why: e.why, end: e.end };
      else if (e instanceof GuardError) this.E = { kind: 'guard', msg: e.message };
      else if (e instanceof InfraError) this.E = { kind: 'infra', msg: e.message };
      else this.E = { kind: 'defect', msg: `harness defect: ${String(e.stack ?? e).split('\n').slice(0, 4).join(' | ')}` };
      if (this.T && this.W) { try { fs.writeFileSync(path.join(this.base, `pane-failed-in-${this.phase}.txt`), pane(this.ctx.S)); } catch { /* best effort */ } }
    }
    try {
      await this.collect();
      await this.decide();
    } catch (e) {
      this.set('INFRA-FAILURE', e instanceof InfraError ? e.message : `harness defect: ${String(e.stack ?? e).split('\n').slice(0, 4).join(' | ')}`);
      if (!(e instanceof InfraError)) this.stop = { why: 'defect', msg: this.reason };
    } finally {
      this.W?.close();
      if (this.lock) await this.lock.release().catch(() => {});
      if (this.sb) this.sb.reapRun();
      if (this.T) { killEp(this.T); safeDelete(this.T); }
      active = null;
    }
    const E = this.E;
    if (!this.stop && E && ((E.kind === 'stop' && E.end) || E.kind === 'defect')) this.stop = { why: E.why ?? 'defect', msg: E.msg };
    return { record: this.toRecord(), stop: this.stop };
  }

  // ---- folder, builder, arm, settings, Muse in tmux
  async prepare() {
    const { ctx, spec: { d, arm, rep, n, control, poison } } = this;
    const R = this.R;
    const low = L.diskLow();
    if (low) throw new Stop(`disk: ${low}`, 'disk');
    if (this.keyed) {
      if (!fs.existsSync(ctx.billingOk)) throw new Stop(`${ctx.billingOk} does not exist: keyed calls are not allowed yet`, 'billing');
      ctx.spent = spentSoFar(ctx);
      if (ctx.spent + ctx.runCap > ctx.cap + 1e-9) throw new Stop(`pre-start check: $${ctx.spent.toFixed(4)} hi spent plus the $${ctx.runCap.toFixed(2)} per-run cap would pass the $${ctx.cap.toFixed(2)} stage cap`, 'cap');
      this.lock = await acquireLock(ctx.lockFile);
      R.lock_wait_s = round(this.lock.waitedMs / 1000, 1);
      const low2 = L.diskLow();
      if (low2) throw new Stop(`disk: ${low2}`, 'disk');
    }
    this.tRun = Date.now();
    const T = (this.T = makeRunFolder(ctx.tmpBase));
    active = { S: ctx.S, T };
    this.sb = makeSandbox({ T, bwrap: ctx.bwrap });
    const build = this.sb.exec({ argv: ['sh', path.join(WS_DIR, d.id, 'build.sh'), T], cwd: T, env: toolEnv(T), ro: [WS_DIR], merge: true, timeoutMs: 180000 });
    R.build = { exit: build.exit, ms: build.ms, output: build.stdout.slice(0, 600) };
    if (build.exit === 2 && /^refused: /m.test(build.stdout)) throw new GuardError(`the builder refused the folder: ${build.stdout.trim().slice(0, 200)}`);
    if (build.exit !== 0) throw new InfraError(`the builder failed (exit ${build.exit}): ${build.stdout.trim().slice(0, 300)}`);
    const ws = (this.ws = path.join(T, 'ws'));
    if (!fs.existsSync(path.join(ws, '.synthetic')) || !fs.existsSync(path.join(T, '.synthetic'))) throw new InfraError('the builder left no .synthetic marker');
    fs.writeFileSync(path.join(T, '.drill-run'), `${JSON.stringify({ id: d.id, arm, rep, mode: 'live-scripted', attempt: n, control })}\n`);
    for (const dir of ['home/.config', 'tmp', 'torpor-home', 'muse-share', 'bin', 'live']) fs.mkdirSync(path.join(T, dir), { recursive: true });
    fs.writeFileSync(path.join(T, 'home/.gitconfig'), GITCONFIG);
    fs.cpSync(path.join(WS_DIR, '_assets'), path.join(T, 'shared'), { recursive: true }); // $SHARED of the script: the model reads it, so it lives in the run folder
    let armText = null;
    if (arm >= 3) {
      const progs = A.installPrograms(T, arm, { engine: ctx.engine, adapter: ctx.adapter });
      R.programs = { engine: progs.engine?.sha256 ?? null, adapter: arm === 4 ? progs.adapter?.sha256 ?? null : null };
      if (ctx.pin.engine && progs.engine?.sha256 !== ctx.pin.engine) throw new Stop('the engine copied into the run folder differs from the pinned sha256', 'binary');
      if (arm === 4 && ctx.pin.adapter && progs.adapter?.sha256 !== ctx.pin.adapter) throw new Stop('the adapter copied into the run folder differs from the pinned sha256', 'binary');
      armText = (arm === 3 ? A.writeArm3Settings(T) : A.runArm4Init(T, this.sb)).text;
    } else if (this.spec.mode === 'smoke' && !ctx.inject) armText = smokeHooks(this.live, ctx.blockStop);
    // Muse hands a hook only HOME, PATH, LANG, TERM, TMPDIR, PWD (smoke: hook-env.txt) and may do the same for the model's shell: a `torpor` call of the
    // model without TORPOR_HOME then uses ~/.torpor. That path is the run's engine home (the hooks and the user's own commands set TORPOR_HOME).
    if (arm >= 3 && !fs.existsSync(path.join(T, 'home/.torpor'))) fs.symlinkSync('../torpor-home', path.join(T, 'home/.torpor'));
    const tool = path.join(this.live, 'live-scripted-tap.mjs');
    fs.copyFileSync(path.join(HERE, 'live-scripted-tap.mjs'), tool);
    fs.copyFileSync(path.join(ORACLES, 'files.mjs'), path.join(this.live, 'files.mjs'));
    const head = this.sh('git rev-parse HEAD');
    R.facts.C0 = head.stdout.trim().split('\n').pop();
    if (head.exit !== 0 || !/^[0-9a-f]{40}$/.test(R.facts.C0)) throw new InfraError(`no base commit C0 in the workspace: ${cap(head.stdout, 200)}`);
    const realFile = path.join(this.live, 'real.json');
    const w = LV.wrapSettings(armText, { node: NODE_EXEC, tool, tap: this.tapFile, realFile });
    fs.writeFileSync(realFile, JSON.stringify(w.real));
    const bud = L.budgetTimeouts(w.text);
    fs.mkdirSync(path.dirname(A.settingsPath(T)), { recursive: true });
    fs.writeFileSync(A.settingsPath(T), bud.text);
    const st = A.parseSettings(bud.text); // a file Muse would refuse is an infra failure
    R.arm_setup = { events: Object.keys(st.hooks ?? {}), warnings: st.warnings ?? [] };
    fs.copyFileSync(A.settingsPath(T), path.join(this.base, 'settings.json'));
    fs.copyFileSync(realFile, path.join(this.base, 'real.json'));
    const scoreRoot = d.oracle?.kind === 'files' ? path.join(T, d.oracle.root) : ws;
    fs.writeFileSync(path.join(this.live, 'cfg.json'), JSON.stringify({
      arm, trigger: d.trigger ?? null, scoreRoot, filesTool: path.join(this.live, 'files.mjs'), ws, precond: d.precondition ? { turn: d.precondition.turn } : null,
      poison: Boolean(poison), timeouts: bud.timeouts, probe: arm >= 3 && this.counted ? { cmd: A.PROBE_SH, env: toolEnv(T) } : null, noManifest: !this.counted,
    }));
    fs.writeFileSync(path.join(this.live, 'state.json'), JSON.stringify({ turn: 0, setup: false }));
    // Muse's own environment: what the model's shell needs (the shims and the engine on PATH, the engine's home and key file), and the eval's switches.
    const env = ['TERM=xterm-256color', `PATH=${T}/bin:${NODE_BIN}:/usr/local/bin:/usr/bin:/bin`, `TORPOR_HOME=${T}/torpor-home`, `TORPOR_KEY_FILE=${path.join(T, 'home', '.config', 'torpor', 'key')}`, 'TZ=UTC', 'GIT_TERMINAL_PROMPT=0', ...LV.MUSE_ENV];
    const launcher = [SANDBOX, '--run-dir', T, '--ro', ctx.museDir, '--ro', ctx.nodeRoot, '--net', this.keyed ? 'meta' : 'none', ...(this.keyed ? ['--drain', '3'] : []), '--chdir', ws, '--', 'env', ...env, path.join(ctx.museDir, 'muse'), ...(this.keyed ? KEYED : FREE)];
    // The pane's shell reads the key from the file for this one command; this program never sees it.
    const paneCmd = `${this.keyed ? `META_API_KEY="$(cat '${ctx.keyFile}')" ` : ''}${launcher.join(' ')}; echo EXITED $?; sleep 900`;
    this.W = new Watch(ctx, T, ctx.spent);
    cp.spawnSync('tmux', ['-S', ctx.S.sock, '-f', '/dev/null', 'new-session', '-d', '-s', 'p', '-x', '170', '-y', '50', '-c', ctx.S.dir, paneCmd], { env: TMUX_ENV });
  }

  // ---- waiting
  async until(what, ms, fn) {
    const S = this.ctx.S;
    for (const end = Date.now() + ms; ; await sleep(1000)) {
      const t = this.W.check();
      if (t) throw new Stop(t.msg, t.why, t.end);
      const v = fn();
      if (v) return v;
      if (/EXITED \d+/.test(pane(S))) throw new InfraError(`Muse exited while waiting for ${what}`);
      if (Date.now() > end) throw new InfraError(`timeout (${ms / 1000} s) waiting for ${what}`);
    }
  }
  soft(what, ms, fn) { return this.until(what, ms, fn).catch((e) => { if (!(e instanceof InfraError)) throw e; return false; }); }
  info() { return this.W.refresh().info ?? { prompts: [], terminals: [], completed: 0 }; }
  // Nothing new in the tap or in the session logs for 2 s (late hooks of the last call are in).
  async quiet() {
    const sig = () => `${sizeOf(this.tapFile)}:${this.W.logs.reduce((s, c) => s + c.size, 0)}`;
    let last = sig(), since = Date.now();
    for (const end = Date.now() + 60000; Date.now() < end;) {
      await sleep(500);
      const t = this.W.check();
      if (t) throw new Stop(t.msg, t.why, t.end);
      const s = sig();
      if (s !== last) { last = s; since = Date.now(); } else if (Date.now() - since >= 2000) return;
    }
    this.R.notes.push('the tap and the session logs never went quiet for 2 s within 60 s');
  }
  // Muse records its `terminal` only after the Stop hook sequence of the turn, blocked answers and continuations included (smoke, echo provider):
  // what is left to wait for is a hook row that lags behind the terminal, then a quiet tap.
  async settle(n) {
    await this.soft(`the Stop hook of turn ${n}`, 30000, () => L.stopStatus(this.rows(), n).outs >= 1);
    await this.soft('the Stop hook to answer', 70000, () => L.stopStatus(this.rows(), n).wait !== 'out');
    const st = L.stopStatus(this.rows(), n);
    if (st.wait === 'continuation') this.R.notes.push(`turn ${n}: the last Stop answer blocked, but Muse did not continue (${st.outs} Stop runs)`);
    await this.quiet();
  }

  // ---- the turns
  autocommit(n) {
    if (this.spec.arm !== 2) return;
    const r = this.sh(`git add -A && git commit -q --allow-empty -m "auto: turn ${n}" && git rev-parse HEAD`, 60000);
    const s = r.stdout.trim().split('\n').pop();
    if (r.exit !== 0 || !/^[0-9a-f]{40}$/.test(s)) throw new InfraError(`auto-commit failed before turn ${n}: ${cap(r.stdout, 200)}`);
    this.R.facts.autoCommits[n] = s;
  }
  async typed(it) {
    const R = this.R;
    const r = this.sh(it.cmd, 180000);
    this.trace('typed', { cmd: cap(it.cmd, 120), exit: r.exit });
    const m = /^torpor promise block (\d+)\s*$/.exec(it.cmd);
    if (!m) return true;
    R.gate = { promise: Number(m[1]), typed_exit: r.exit, typed_output: cap(r.stdout.trim(), 500), polls: 0, last_line: null, listed: false, reached: false, max_polls: 12, sleep_ms: 5000 };
    const want = new RegExp(`^#${m[1]} .* · blocking · .*your words, turn \\d+\\s*$`);
    for (let i = 0; i < 12; i++) {
      if (i > 0) await sleep(5000);
      const p = this.sh('torpor promise list', 60000);
      R.gate.polls = i + 1;
      const line = p.stdout.split('\n').find((l) => l.startsWith(`#${m[1]} `)) ?? null;
      R.gate.last_line = line ?? cap(p.stdout.trim(), 300);
      if (line) R.gate.listed = true;
      if (p.exit === 0 && line && want.test(line)) { R.gate.reached = true; break; }
    }
    this.trace('gate', { reached: R.gate.reached, polls: R.gate.polls, last: cap(R.gate.last_line ?? '', 160) });
    return R.gate.reached;
  }
  between(it) {
    const cmd = it.cmd.replace(/\bC0\b/g, this.R.facts.C0);
    const r = this.sh(cmd, 180000);
    this.trace('between', { cmd: cap(cmd, 120), exit: r.exit });
    if (r.exit !== 0) this.R.warnings.push(`between: exit ${r.exit}: ${cap(it.cmd, 80)}`);
  }
  // The user's turn: the prompt pasted into the TUI, then the model works until Muse records the turn's end and the hooks of it are in.
  async ask(turn, text) {
    const S = this.ctx.S;
    const i0 = this.info();
    const baseP = i0.prompts.length, baseT = i0.terminals.length;
    await paste(S, text);
    const acc = await this.until(`the prompt of turn ${turn.n} to be accepted`, 30000, () => { const i = this.info(); return i.prompts.length > baseP ? i : null; });
    if (!acc.prompts[baseP].includes(text.trim())) throw new InfraError(`Muse accepted a different text for turn ${turn.n}`);
    await this.until(`turn ${turn.n} to finish`, TURN_MS, () => {
      const i = this.info();
      if (i.terminals.length <= baseT) return null;
      if (i.terminals[baseT].terminal !== 'completed') throw new InfraError(`turn ${turn.n} ended with "${i.terminals[baseT].terminal}"`);
      return i;
    });
    await this.settle(turn.n);
  }
  // smoke --inject only (no model, no key): the user's prompt, the scripted calls and the Stop go through the arm's hooks the way replay.mjs sends them,
  // so the tap, the manifests, the verdict and the recovery run on a run that triggered (the echo model never calls a tool).
  inject(turn, text, opts) {
    const { arm } = this.spec, s = (this.sess ??= C.newSession(this.ws));
    const set = A.parseSettings(fs.readFileSync(A.settingsPath(this.T), 'utf8'));
    const fire = (payload, tool) => (set.hooks[payload.hook_event_name] ?? []).filter((h) => C.matcherMatches(h.matcher, tool)).map((h) => {
      const r = this.sb.exec({ argv: ['sh', '-c', h.command], cwd: this.ws, env: hookEnv(this.T, this.ws, s), stdin: JSON.stringify(payload), timeoutMs: h.timeout * 1000 });
      return C.interpret(payload.hook_event_name, { exit: r.exit, stdout: r.stdout, stderr: r.stderr, timedOut: r.timedOut }).action;
    });
    s.turn = C.uuidv7();
    fire(C.P.userPrompt(s, text));
    const lines = L.callLines(turn, arm, opts);
    let k = 0;
    for (const it of turn.items) {
      if (it.kind !== 'call' || (it.arms34 && arm < 3)) continue;
      const line = lines[k++], id = C.toolUseId(), tool = it.tool;
      const input = tool === 'bash' ? C.bashInput(s, line.text) : tool === 'edit_file' ? it.input : { path: it.input.path, content: opts.readAsset(it.input.content_from) };
      if (fire(C.P.preTool(s, tool, input, id), tool).some((a) => a.kind === 'deny')) continue; // Muse does not run a denied call
      let post;
      if (tool === 'bash') { const r = this.sh(line.text, 120000); post = C.P.postTool(s, tool, input, id, C.bashResponse(s, line.text, r.exit, r.stdout)); }
      else {
        const abs = path.resolve(this.ws, input.path);
        if (!abs.startsWith(this.ws + path.sep)) throw new GuardError(`inject: ${input.path} is outside the workspace`);
        if (tool === 'write_file') { fs.mkdirSync(path.dirname(abs), { recursive: true }); fs.writeFileSync(abs, input.content); post = C.P.postTool(s, tool, input, id, C.fileResponse(abs, input.content, true)); }
        else {
          const t = fs.existsSync(abs) ? fs.readFileSync(abs, 'utf8') : '';
          if (t.split(input.find).length - 1 !== 1) post = C.P.postToolFailure(s, tool, input, id, `the find text must occur exactly once in ${input.path}`, 1);
          else { fs.writeFileSync(abs, t.replace(input.find, () => input.replace)); post = C.P.postTool(s, tool, input, id, C.fileResponse(abs, '', false)); }
        }
      }
      fire(post, tool);
    }
    let last = turn.items.find((i) => i.kind === 'say')?.text ?? '';
    for (let a = 0; a < 3; a++) { // a blocked Stop gets the script's on-block answer, as in replay
      if (!fire(C.P.stop(s, last, a > 0)).some((x) => x.kind === 'block')) break;
      last = turn.onBlockSay ?? last;
    }
  }
  async play() {
    const { ctx, spec: { d, arm } } = this;
    const R = this.R, S = ctx.S;
    const readAsset = readAssetFor(d);
    const trigCall = L.triggerCall(d, arm, readAsset);
    this.phase = 'boot';
    await this.until('the TUI prompt', 90000, () => pane(S).includes('❯'));
    await sleep(1500);
    for (const turn of d.turns) {
      if (ctx.turnOnly && turn.n !== ctx.turnOnly) continue;
      this.phase = `turn${turn.n}`;
      this.autocommit(turn.n);
      fs.writeFileSync(path.join(this.live, 'state.json'), JSON.stringify({ turn: turn.n, setup: turn.setup }));
      const opts = { shared: path.join(this.T, 'shared'), readAsset };
      const text = L.promptFor(turn, arm, opts);
      R.prompts.push(text);
      R.scripted.push(...L.callLines(turn, arm, opts));
      if (ctx.inject) this.inject(turn, text, opts); else await this.ask(turn, text);
      this.trace('turn', { n: turn.n, setup: turn.setup, stop: L.stopStatus(this.rows(), turn.n) });
      const keepItem = turn.setup && arm >= 3 ? turn.items.find((i) => i.kind === 'call' && /\btorpor keep\b/.test(i.input.command ?? '')) : null;
      if (ctx.seedKeep && turn.setup) { // smoke only: the echo model makes no calls, so the edits of the setup turns and the keep are played here, without the variables the model's shell may lack
        for (const it of turn.items) {
          if (it.kind !== 'call' || it.tool !== 'edit_file') continue;
          const f = path.join(this.ws, it.input.path);
          fs.writeFileSync(f, fs.readFileSync(f, 'utf8').replace(it.input.find, () => it.input.replace));
        }
      }
      if (keepItem && ctx.seedKeep) {
        const r = this.sh(`env -u TORPOR_HOME -u TORPOR_KEY_FILE ${keepItem.input.command.replace(/\$SHARED(?=\/)/g, path.join(this.T, 'shared'))}`, 60000);
        this.trace('seed-keep', { exit: r.exit, output: cap(r.stdout.trim(), 300) });
      } else if (keepItem) {
        this.checkKeep(turn.n); // before the typed words: a promise the model never kept cannot be made blocking
        if (R.invalidPre || R.envFail) break;
      }
      let gateFailed = false;
      for (const it of turn.items) { // what the user types after the model's answer, in script order
        if (it.arms34 && arm < 3) continue;
        if (it.kind === 'typed') { if (!(await this.typed(it))) { gateFailed = true; break; } } else if (it.kind === 'between') this.between(it);
      }
      if (gateFailed) {
        R.gateReason = R.gate.listed
          ? `promise ${R.gate.promise} was not blocking after ${R.gate.polls} polls; last line: ${cap(R.gate.last_line ?? '', 200)}`
          : `promise ${R.gate.promise} was never listed after ${R.gate.polls} polls although the model's keep call exited 0: ${cap(R.gate.last_line ?? '', 200)}`;
        if (R.gate.listed) R.gateFailed = true; // scored like MI, the run ends here (PREREG section 5)
        else R.envFail = 'the promise the model kept is not in the engine home of the run (torpor keep did not reach T/torpor-home)';
        break;
      }
      if (trigCall && turn.n === trigCall.turn && !fs.existsSync(path.join(this.live, 'trigger.json'))) { R.early_stop = `no call matched the trigger in turn ${turn.n}; the later turns were not played`; break; }
    }
    this.phase = 'exit';
    await typeLine(S, '/exit');
    await this.soft('Muse to exit', 30000, () => /EXITED \d+/.test(pane(S)));
  }
  // The model's own `torpor keep` is part of the scripted setup: it must have run and worked.
  checkKeep(n) {
    const R = this.R;
    const k = L.keepCall(this.rows(), n);
    R.keep = k.found ? { exit: k.exit, env: k.env, output: cap(k.output ?? '', 300) } : { found: false };
    if (!k.found) R.invalidPre = `the model did not make the scripted setup call "torpor keep" in turn ${n}`;
    else if (k.exit !== 0 && ['command not found', 'permission', 'read-only file system'].includes(k.env)) R.envFail = `torpor keep: ${k.env}`;
    else if (k.exit !== 0) R.invalidPre = `the scripted setup call "torpor keep" failed (exit ${k.exit}): ${cap(k.output ?? '', 200)}`;
  }

  // ---- after the run: Muse dead, logs read, key scan, side files
  async collect() {
    const { ctx, T } = this;
    if (!T) return;
    this.W?.close();
    tm(ctx.S, 'kill-server');
    killEp(T);
    for (let i = 0; i < 20 && epProcs(T).length; i++) await sleep(250);
    if (epProcs(T).length) this.R.notes.push(`${epProcs(T).length} processes of the run folder were still alive after the kill`);
    if (this.lock) { await this.lock.release().catch(() => {}); this.lock = null; } // the keyed part is over
    const W = this.W?.refresh();
    const logs = W?.logs ?? [];
    const calls = logs.flatMap((c, i) => c.rows.map((u) => ({ ...u, log: c.sub ? `sub-${logs.slice(0, i).filter((x) => x.sub).length + 1}` : 'main' })));
    fs.writeFileSync(path.join(this.base, 'calls.json'), `${JSON.stringify({ started_at: this.rec.started_at, rows: calls })}\n`);
    const c = LV.sumCost(calls);
    this.result.cost = { lo: round(c.lo), hi: round(c.hi), calls: calls.filter((x) => x.call === 'model').length, compaction_calls: calls.filter((x) => x.call === 'compaction').length };
    this.result.minutes = round((Date.now() - this.tRun) / 60000, 2);
    this.result.sessions = { main: logs.filter((x) => !x.sub).length, subagents: logs.filter((x) => x.sub).length };
    this.result.models = [...new Set(calls.map((x) => x.model))];
    const main = logs.find((x) => !x.sub);
    this.actual = main ? L.sessionCalls(main.recs) : [];
    this.result.calls = L.compareCalls(this.R.scripted, this.actual);
    if (this.keyed) { // a leak stops the stage before anything leaves the run folder
      let scan = L.leakScan(ctx.keyFile, [T]);
      if (scan.status === 2) { cp.spawnSync('chmod', ['-R', 'u+rwX', T]); scan = L.leakScan(ctx.keyFile, [T]); }
      if (!scan.ok) { this.leak = scan.hits.length ? `KEY LEAK: ${scan.hits.length} file(s) in the run folder hold the key` : `the key scan failed (rg exit ${scan.status}): ${scan.err}`; this.stop = { why: 'leak', msg: this.leak }; return; }
    }
    const gz = (src, dst) => fs.writeFileSync(dst, zlib.gzipSync(fs.readFileSync(src), { level: 6 }));
    let subN = 0;
    for (const x of logs) gz(x.file, path.join(this.base, x.sub ? `session-sub-${++subN}.jsonl.gz` : 'session.jsonl.gz'));
    const rows = L.orderTap(this.rows());
    this.tap = rows;
    this.result.tap_events = L.tapCounts(rows);
    fs.writeFileSync(path.join(this.base, 'tap.jsonl'), rows.map((r) => `${JSON.stringify(this.counted ? r : { ...r, json: r.json ? Object.keys(r.json) : null, stdout: undefined, stderr: undefined, real_stdout: undefined, real_stderr: undefined })}\n`).join(''));
    for (const f of ['trigger.json', 'r1.txt', 'post.txt', 'post-from.json', 'precond.json', 'torpor-points.txt', 'probe.json', 'hook-env.txt']) {
      if (fs.existsSync(path.join(this.live, f))) fs.copyFileSync(path.join(this.live, f), path.join(this.base, f));
    }
    if (fs.existsSync(path.join(T, 'net/proxy.log'))) fs.copyFileSync(path.join(T, 'net/proxy.log'), path.join(this.base, 'proxy.log'));
    fs.writeFileSync(path.join(this.base, 'prompts.json'), `${JSON.stringify(this.R.prompts)}\n`);
    this.R.prompts_sha256 = this.R.prompts.map(shaText);
    this.result.egress = this.keyed ? egressRead(T) : null;
    const trig = readJson(path.join(this.base, 'trigger.json'));
    this.R.trigger = trig;
    if (trig) {
      const tre = new RegExp(this.spec.d.trigger);
      this.result.multi_trigger = this.tap.filter((r) => r.event === 'PreToolUse' && r.dir === 'in' && !r.setup && tre.test(r.json?.tool_name === 'bash' ? String(r.json.tool_input?.command ?? '') : JSON.stringify(r.json?.tool_input ?? {}))).length;
      this.R.denied = L.deniedById(this.tap, trig.tool_use_id) === true;
      const post = this.tap.find((r) => r.dir === 'in' && ['PostToolUse', 'PostToolUseFailure'].includes(r.event) && r.json?.tool_use_id === trig.tool_use_id);
      if (post) { const res = L.parseResult(post.json.tool_response ?? post.json.error); this.trigResult = { event: post.event, exit: res.exit, status: res.status, env: L.envClass(res.output), output: res.output }; }
    }
    this.result.batched = this.result.calls.batched;
    const bad = this.tap.filter((r) => r.tap_errors).map((r) => `${r.event}: ${r.tap_errors.join('; ')}`);
    if (bad.length) this.R.tap_errors = bad.slice(0, 5);
    if (this.keyed) {
      const scan2 = L.leakScan(ctx.keyFile, [this.base]);
      if (!scan2.ok) { this.leak = 'KEY LEAK in the side folder of the run'; this.stop = { why: 'leak', msg: this.leak }; }
    }
  }

  // ---- the verdict (counted runs score; calibration and smoke runs only say whether the run worked)
  manifest(label, root) {
    let text = '';
    const exists = fs.existsSync(root);
    if (exists) {
      const r = cp.spawnSync(process.execPath, [path.join(ORACLES, 'files.mjs'), 'manifest', root], { encoding: 'utf8', timeout: 120000, maxBuffer: 1 << 28 });
      if (r.status !== 0) throw new InfraError(`files.mjs manifest failed: ${(r.stderr || '').slice(0, 200)}`);
      text = r.stdout;
    } // a missing root (rm -rf of the whole folder) is an empty manifest
    fs.writeFileSync(path.join(this.base, `${label}.txt`), text);
    this.R.manifests[label] = { sha256: shaText(text), files: text ? text.split('\n').length - 1 : 0, root_exists: exists };
  }
  heldOut(label, wsDir) {
    const check = path.join(WS_DIR, this.spec.d.id, 'held-out/check.mjs');
    if (!fs.existsSync(check)) throw new InfraError(`no held-out check for ${this.spec.d.id}: ${check}`);
    const r = this.sb.exec({ argv: [NODE_EXEC, path.join(ORACLES, 'regression.mjs'), 'run', wsDir, check], cwd: this.T, env: toolEnv(this.T, { SHARED: path.join(this.T, 'shared') }), ro: [ORACLES, path.dirname(check)], merge: true, timeoutMs: 180000 });
    fs.writeFileSync(path.join(this.base, `${label}.txt`), r.stdout);
    return { lines: r.stdout.split('\n').filter((l) => /^(PASS|FAIL) /.test(l)), exit: r.exit, file: path.join(this.base, `${label}.txt`) };
  }
  notTriggeredText() {
    const list = this.actual.map((c) => `${c.tool}: ${cap(c.text, 70)}`).join(' | ');
    return cap(`no call matched the trigger /${this.spec.d.trigger}/ in a turn that is not a setup turn${this.R.early_stop ? ` (${this.R.early_stop})` : ''}; the model made ${this.actual.length} calls: ${list}`, 700);
  }
  async decide() {
    const { spec: { d, arm }, E, R, base } = this;
    if (this.leak) return this.set('INFRA-FAILURE', this.leak);
    if (E?.kind === 'stop' && E.end) return this.set('INFRA-FAILURE', E.msg); // our own guards stop the run: an infra stop, never a result
    if (E?.kind === 'defect') return this.set('INFRA-FAILURE', E.msg);
    if (E?.kind === 'guard') return this.set('INVALID', E.msg);
    if (R.tap_errors) return this.set('INFRA-FAILURE', `the tap itself failed: ${R.tap_errors[0]}`); // a failure of the harness, not of the arm
    const trig = R.trigger;
    if (!this.counted) {
      if (E) return this.set('INFRA-FAILURE', E.msg);
      if (R.invalidPre || R.envFail) return this.set('INFRA-FAILURE', R.envFail ? `environment: ${R.envFail}` : R.invalidPre);
      if (d.trigger && !trig) return this.set('NOT-TRIGGERED', null);
      return this.set('OK', null);
    }
    if (R.envFail) return this.set('INFRA-FAILURE', `environment: ${R.envFail}`);
    if (R.invalidPre) return this.set('INVALID-PRECONDITION', R.invalidPre);
    if (R.gateFailed) return this.set('GATE-NOT-REACHED', R.gateReason); // scored like MI; the run ends here without recovery (PREREG section 5)
    if (d.trigger && !trig) return E ? this.set('INFRA-FAILURE', `${E.msg}; the trigger call had not happened`) : this.set('NOT-TRIGGERED', this.notTriggeredText());
    if (trig && this.trigResult && trig.tool === 'bash' && !R.denied && this.trigResult.exit !== 0 && this.trigResult.env && this.trigResult.env !== 'no such path') {
      this.envClass = this.trigResult.env;
      return this.set('INFRA-FAILURE', `environment: the trigger call failed with "${this.trigResult.env}": ${cap(this.trigResult.output ?? '', 160)}`);
    }
    if (E) R.notes.push(`aborted after the trigger call, scored as is: ${E.msg}`);
    const cls = d.oracle?.kind ?? 'files';
    let invalid = null;
    if (d.precondition) {
      if (!fs.existsSync(path.join(base, 'precond.json')) || !fs.existsSync(path.join(this.live, 'precond-ws'))) return this.set('INFRA-FAILURE', `${E ? `${E.msg}; ` : ''}the Stop hook of the last turn never ran: no snapshot for the precondition check`);
      const h = this.heldOut('precondition', path.join(this.live, 'precond-ws'));
      const failing = h.lines.filter((l) => new RegExp(`^FAIL \\[${d.precondition.tag}\\]`).test(l));
      R.precondition = { ok: failing.length > 0, failing: failing.slice(0, 5), lines: h.lines.length, check_exit: h.exit };
      if (!R.precondition.ok) invalid = `no held-out [${d.precondition.tag}] check failed at the first Stop of the last turn: the "Done" claim was true`;
    }
    for (const label of ['r1', 'post']) {
      if (label === 'post' && !fs.existsSync(path.join(base, 'post.txt')) && trig && d.oracle?.kind !== 'claim') { this.manifest('post', cls === 'files' ? path.join(this.T, d.oracle?.root ?? 'ws') : this.ws); R.notes.push('no Post manifest of the trigger call: the state after the run stands in for it'); }
      if (fs.existsSync(path.join(base, `${label}.txt`))) { const t = fs.readFileSync(path.join(base, `${label}.txt`), 'utf8'); R.manifests[label] = { sha256: shaText(t), files: t ? t.split('\n').length - 1 : 0 }; }
    }
    const scoreRoot = d.oracle?.kind === 'files' ? path.join(this.T, d.oracle.root) : this.ws;
    if (trig) { this.manifest('pre-recovery', scoreRoot); this.result.deny_then_loss = R.denied && R.manifests['pre-recovery'].sha256 !== R.manifests.post?.sha256; }
    if (cls === 'regression') { const b = this.heldOut('baseline', this.ws); R.baseline_lines = b.lines; }
    if (cls !== 'claim') {
      if (arm >= 3) {
        const pts = path.join(base, 'torpor-points.txt'), pr = readJson(path.join(base, 'probe.json'));
        const sp = fs.existsSync(pts) ? (pr?.exit === 0 ? A.pointsFrom(fs.readFileSync(pts, 'utf8')) : { P: null, P_text: null, points: [], why: `the save point probe exited ${pr?.exit}` }) : { P: null, P_text: null, points: [], why: 'the save point probe never ran' };
        R.facts.P = sp.P; R.facts.P_text = sp.P_text; R.facts.P_why = sp.why ?? null;
      }
      R.facts.A = R.facts.autoCommits?.[trig.turn];
      R.facts.A2 = R.facts.autoCommits?.[2];
      const plan = A.recoveryPlan(arm, d, R.facts);
      if (plan.skip) R.recovery.skipped = plan.skip;
      else for (const st of plan) { const r = this.sh(st.cmd, 300000); R.recovery.steps.push({ label: st.label, cmd: st.cmd, exit: r.exit, output: cap(r.stdout.trim(), 600), timed_out: r.timedOut }); }
    }
    if (cls === 'regression') R.final_lines = this.heldOut('final-check', this.ws).lines;
    if (trig) this.manifest('final', scoreRoot);
    const sc = L.scoreRun({ d, arm, dir: base, oraclesDir: ORACLES, baselineLines: R.baseline_lines ?? [] });
    this.oracleRes = sc.oracle;
    this.set(sc.outcome, null);
    invalid ??= sc.noop ?? null;
    if (invalid) { this.wouldBe = this.outcome; this.set('INVALID-PRECONDITION', invalid); }
  }

  toRecord() {
    const { ctx, spec: { d, arm, rep, n, control, mode, poison }, R } = this;
    const R2 = { ...R, script_warnings: R.warnings };
    delete R2.prompts; delete R2.scripted; delete R2.warnings;
    return {
      schema: 'torpor-drill-run/1', batch: ctx.batchId, batch_label: L.BATCH_LABEL, disaster: d.id, title: d.title, class: d.oracle?.kind ?? 'files', arm, arm_name: A.ARM_NAMES[arm - 1], control,
      repetition: rep, mode: 'live', attempt: n, outcome: this.outcome, outcome_reason: this.reason, would_be_outcome: this.wouldBe, oracle: this.oracleRes,
      started_at: this.rec.started_at, ended_at: utc(), overlap: { ...this.rec.overlap, end: probe() }, ...(mode === 'calibrate' ? { calibration: true } : {}), ...(mode === 'smoke' ? { smoke: true } : {}),
      contract: { name: 'real Muse (no emulated contract)', rules: [], unverified_used: [] }, gate_config: { max_polls: 12, sleep_ms: 5000, overridden: false },
      denied: R.denied ?? false, poison: Boolean(poison), env_class: this.envClass, ...R2, ...this.result,
      engine: arm >= 3 ? ctx.prov.engine : null, adapter: arm === 4 ? ctx.prov.adapter : null, muse: ctx.prov.muse, method: ctx.method, driver: ctx.driver, side_files: path.relative(ctx.out, this.base),
    };
  }
}

// One slot (disaster, arm, repetition, control) with the infra rule of PREREG section 6: at most 2 re-runs; the 3rd failure is INVALID.
async function runSlot(ctx, mode, s, first = 1) {
  const file = path.join(ctx.out, mode === 'counted' ? 'runs.jsonl' : `${mode === 'calibrate' ? 'calibration' : 'smoke'}/runs.jsonl`);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  let last = null;
  for (let n = first; n <= 3; n++) {
    const { record, stop } = await new Run(ctx, { mode, d: s.d, arm: s.arm, rep: s.rep, n, control: s.control ?? null, poison: s.poison ?? false }).execute();
    if (record.outcome === 'INFRA-FAILURE' && n === 3 && !stop) record.outcome = 'INVALID';
    fs.appendFileSync(file, `${JSON.stringify(record)}\n`);
    last = { record, stop };
    if (stop || record.outcome !== 'INFRA-FAILURE' || mode !== 'counted') break;
    await sleep(2000);
  }
  return last;
}

// ------------------------------------------------------------------ setup

function setup(o, cmd) {
  const need = (k) => o[k] ?? (() => { throw new Error(`option --${k} is needed`); })();
  const keyedCmd = cmd === 'calibrate' || cmd === 'batch';
  const arms = String(o.arms ?? '1,2,3,4').split(',').map(Number);
  const out = path.resolve(need('out'));
  const tmpBase = path.resolve(need('tmp-base'));
  fs.mkdirSync(tmpBase, { recursive: true });
  const S0 = makeRunFolder(tmpBase);
  const ctx = {
    out, bwrap: findBwrap(), tmpBase, S: { dir: S0, sock: path.join(S0, 'tm.sock') }, museDir: path.resolve(need('muse-dir')), nodeRoot: path.resolve(o['node-root'] ?? path.dirname(NODE_BIN)),
    keyFile: keyedCmd ? path.resolve(need('key-file')) : null, lockFile: keyedCmd ? path.resolve(need('lock-file')) : null, billingOk: keyedCmd ? path.resolve(need('billing-ok')) : null,
    engine: null, adapter: null, pin: { engine: o['engine-sha256'] ?? null, adapter: o['adapter-sha256'] ?? null }, blockStop: o['block-stop'] ?? null, seedKeep: Boolean(o['seed-keep']), inject: Boolean(o.inject), turnOnly: o.turn ? Number(o.turn) : null,
    cap: Number(o.cap ?? 4), runCap: Number(o['run-cap'] ?? 0.1), attemptMs: Number(o['attempt-min'] ?? 25) * 60000, spent: 0, batchId: path.basename(out),
  };
  const needBins = cmd === 'smoke' ? (Number(o.arm ?? 1) >= 3 ? [Number(o.arm)] : []) : arms;
  if (needBins.some((a) => a >= 3)) { ctx.engine = path.resolve(need('engine')); if (cmd !== 'smoke') need('engine-sha256'); }
  if (needBins.includes(4)) { ctx.adapter = path.resolve(need('adapter')); if (cmd !== 'smoke') need('adapter-sha256'); }
  for (const [k, v] of Object.entries({ museDir: ctx.museDir, keyFile: ctx.keyFile, lockFile: ctx.lockFile, nodeRoot: ctx.nodeRoot, node: NODE_EXEC, tmpBase, out })) {
    if (v && !/^[\w@%+=:,./-]+$/.test(v)) throw new Error(`${k} has a character that the pane command cannot carry safely: ${v}`);
  }
  if (!fs.existsSync(SANDBOX)) throw new Error('scripts/sandbox/muse-sandbox.sh is missing');
  if (ctx.engine && ctx.pin.engine && sha(ctx.engine) !== ctx.pin.engine) throw new Error(`the engine binary differs from --engine-sha256 (${sha(ctx.engine)})`);
  if (ctx.adapter && ctx.pin.adapter && sha(ctx.adapter) !== ctx.pin.adapter) throw new Error(`the adapter binary differs from --adapter-sha256 (${sha(ctx.adapter)})`);
  const lock = fs.existsSync(path.join(REPO, 'engine.lock')) ? fs.readFileSync(path.join(REPO, 'engine.lock'), 'utf8') : '';
  const base = (i) => i && { ...i, path: path.basename(i.path) };
  const mbin = fs.readdirSync(ctx.museDir).find((f) => /^muse-bin-/.test(f));
  ctx.prov = {
    engine: ctx.engine ? base(A.engineInfo(ctx.engine, lock, A.versionLine)) : null,
    adapter: ctx.adapter ? base(A.adapterInfo(ctx.adapter, path.resolve(path.dirname(ctx.adapter), '../..'), A.versionLine)) : null, // the worktree it was built in
    muse: { launcher_sha256: sha(path.join(ctx.museDir, 'muse')), binary: mbin ? { name: mbin, sha256: sha(path.join(ctx.museDir, mbin)) } : null, version_line: museVersion(ctx) },
  };
  if (o['muse-binary-sha256'] && ctx.prov.muse.binary?.sha256 !== o['muse-binary-sha256']) throw new Error(`the Muse binary differs from --muse-binary-sha256 (${ctx.prov.muse.binary?.sha256})`);
  if (keyedCmd) { // an empty line in the key file would make every file a "hit": prove the scan says "none" on a clean file first
    const f = path.join(S0, 'leak-selfcheck.txt');
    fs.writeFileSync(f, 'nothing to see\n');
    if (L.leakScan(ctx.keyFile, [f]).status !== 1) throw new Error('the key scan does not return "no hit" on a clean file (empty line in the key file, or rg missing)');
  }
  const git = (...a) => cp.spawnSync('git', ['-C', REPO, ...a], { encoding: 'utf8' }).stdout.trim();
  const files = ['live-scripted.mjs', 'live-scripted-lib.mjs', 'live-scripted-tap.mjs'];
  ctx.driver = { commit: git('rev-parse', 'HEAD'), dirty: git('status', '--porcelain', '--', ...files.map((f) => `drill/harness/${f}`)) !== '', sha256: Object.fromEntries(files.map((f) => [f, sha(path.join(HERE, f))])) };
  ctx.method = {
    name: 'tmux-TUI, scripted prompts', prompt: { intro: L.INTRO, reply: L.REPLY, paste: 'bracketed paste (tmux paste-buffer -p), then Enter', shared: 'the literal path of the run folder copy of _assets replaces $SHARED' },
    muse_flags: cmd === 'smoke' ? FREE : KEYED, muse_env: LV.MUSE_ENV, launch_env: ['PATH=T/bin:NODE_BIN:/usr/local/bin:/usr/bin:/bin', 'TORPOR_HOME=T/torpor-home', `TORPOR_KEY_FILE=${path.join('T', 'home', '.config', 'torpor', 'key')}`, 'TZ=UTC', 'GIT_TERMINAL_PROMPT=0'],
    not_in_tui: ['--disable-reminders', '--disable-web-tools', '--max-model-steps', '--session-id'], tap: 'live-scripted-tap.mjs on every event of every arm (record, manifests, then the real hook with its own limit)',
    hook_budget_s: L.HOOK_BUDGET_S, caps: { stage_hi: ctx.cap, run_hi: ctx.runCap }, watchdog_ms: 400, attempt_minutes: ctx.attemptMs / 60000,
    lock: 'one flock -x holder process per attempt, on the shared key lock file, held from before the run folder is made until Muse is killed',
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

// ------------------------------------------------------------------ prompts, ledger, compare

export function parseArgs(argv) {
  const o = {};
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i].startsWith('--')) throw new Error(`unexpected argument ${argv[i]}`);
    const k = argv[i].slice(2), v = argv[i + 1];
    if (v === undefined || v.startsWith('--')) o[k] = true;
    else { o[k] = v; i++; }
  }
  return o;
}
const loadDisasters = () => {
  const { disasters, errors } = parseDisasters(readMd(path.join(REPO, 'drill/DISASTERS.md')));
  if (errors.length) throw new Error(`DISASTERS.md does not parse: ${errors[0]}`);
  return disasters;
};
const orderOf = () => (runOrder(readMd(path.join(REPO, 'drill/PREREG.md'))) ?? []).filter((id) => id !== 'D15');

// The prompt of every turn as the model sees it; $SHARED is shown as <RUN>/shared (in a run it is the run folder's copy).
export function promptsMd(disasters, order) {
  const fence = (t) => { if (t.includes('```')) throw new Error('a prompt holds a code fence'); return ['```text', t, '```', ''].join('\n'); };
  const out = ['# Prompts of the scripted-live runs', '', `Batch label: ${L.BATCH_LABEL}`, '',
    'Form of every turn (setup turns included): the script\'s `user:` line, a blank line, then "' + `${L.INTRO} ${L.REPLY} <the script's say: line>` + '" and the numbered calls.',
    'Lines marked `[arms 3 4]` in the script are asked for arms 3 and 4 only. `<RUN>/shared` stands for the run folder copy of drill/workspaces/_assets. Turns that differ by arm are shown twice.', ''];
  for (const id of order) {
    const d = disasters.find((x) => x.id === id);
    if (!d) throw new Error(`${id} is not in DISASTERS.md`);
    out.push(`## ${d.id} ${d.title}`, '');
    for (const turn of d.turns) {
      const p = [1, 3].map((arm) => L.promptFor(turn, arm, { shared: '<RUN>/shared', readAsset: readAssetFor(d) }));
      const tag = `${turn.setup ? ' (setup)' : ''}`;
      if (p[0] === p[1]) out.push(`### ${d.id} turn ${turn.n}${tag}, arms 1 to 4`, '', fence(p[0]));
      else out.push(`### ${d.id} turn ${turn.n}${tag}, arms 1 and 2`, '', fence(p[0]), `### ${d.id} turn ${turn.n}${tag}, arms 3 and 4`, '', fence(p[1]));
    }
  }
  return `${out.join('\n')}\n`;
}

function ledger(o) {
  const dirs = [path.join(o.out, 'calibration/runs'), path.join(o.out, 'runs')];
  const runs = dirs.flatMap((d) => (fs.existsSync(d) ? fs.readdirSync(d).filter((k) => fs.existsSync(path.join(d, k, 'calls.json'))).map((k) => ({ k, d, calls: readJson(path.join(d, k, 'calls.json')) })) : []))
    .map((x) => ({ label: `${x.d.endsWith('calibration/runs') ? 'calibration ' : ''}${x.k}`, started: x.calls.started_at, rows: x.calls.rows })).sort((a, b) => a.started.localeCompare(b.started));
  const { lines, lo, hi } = LV.ledgerLines(runs);
  const cap$ = Number(o.cap ?? 4).toFixed(2);
  fs.writeFileSync(o.file, [`# L2 live-scripted spend ledger (cap $${cap$} hi for the whole stage, calibration included; every model call counts, failed runs too)`, '',
    `Model: ${LV.MODEL} (Meta API, contributor-priced id) driven by the Muse Code TUI under tmux (drill/harness/live-scripted.mjs); one row per model call or compaction call, read from the session logs by script.`,
    `Prices per 1M tokens (contributor ids): ${L.MODEL_PRICES_NOTE} (eval/run/lib.mjs cost()).`,
    'lo = (input - cached) x 0.10 + cached x 0.002 + output x 0.20; hi = input x 0.10 + cached x 0.002 + (output + reasoning) x 0.20 (upper bound if reasoning is not inside output).', '',
    '| # | run | call | time UTC | model | input | cached | output | reasoning | $ lo | $ hi | cumulative $ hi |', '|---|-----|------|----------|-------|-------|--------|--------|-----------|------|------|-----------------|',
    ...lines, '', `Total: ${lines.length} calls, $ lo ${lo.toFixed(4)}, $ hi ${hi.toFixed(4)} of the $${cap$} cap.`, ''].join('\n'));
  console.log(`ledger: ${lines.length} calls, hi $${hi.toFixed(4)}`);
}

// Replay cell against live cell, with the facts of the live runs that can explain a difference.
function compareTable(o) {
  const md = readMd(path.join(REPO, 'drill/DISASTERS.md'));
  const pred = predictions(md);
  const ids = Object.keys(pred).filter((id) => id !== 'D15');
  const opts = { ids, predictions: pred, expected: expectedRed(md), reps: 3 };
  const rep = readRecords([o.replay]).records, liv = readRecords([o.out]).records;
  const A1 = analyze(rep, opts), A2 = analyze(liv, opts);
  const by = (id, arm) => liv.filter((r) => r.disaster === id && r.arm === arm && !r.control && !r.calibration);
  const facts = (id, arm) => {
    const rs = by(id, arm), f = [];
    const count = (pred2, label) => { const k = rs.filter(pred2).length; if (k) f.push(`${label} ${k}`); };
    count((r) => r.outcome === 'NOT-TRIGGERED', 'not triggered');
    count((r) => r.outcome === 'INVALID-PRECONDITION', 'invalid precondition');
    count((r) => r.outcome === 'INFRA-FAILURE' || r.outcome === 'INVALID', 'infra/invalid');
    count((r) => r.outcome === 'GATE-NOT-REACHED', 'gate not reached');
    count((r) => (r.calls?.extra?.length ?? 0) > 0, 'extra calls in');
    count((r) => (r.calls?.missing?.length ?? 0) > 0, 'missing calls in');
    count((r) => r.denied, 'trigger denied in');
    count((r) => r.batched, 'batched in');
    count((r) => r.deny_then_loss, 'deny then loss in');
    count((r) => (r.multi_trigger ?? 0) > 1, 'trigger repeated in');
    return f.join('; ') || '-';
  };
  const rows = ['| disaster | arm | replay | live | same? | live facts (runs of 3 with the fact) |', '|---|---|---|---|---|---|'];
  const diffs = [];
  for (const id of ids) {
    for (let a = 0; a < 4; a++) {
      const r = A1.cells[id][a], l = A2.cells[id][a];
      const same = r.state === 'OK' && l.state === 'OK' ? (r.value === l.value ? 'same' : 'DIFFERENT') : 'not comparable';
      if (same !== 'same') diffs.push(`${id} arm ${a + 1}: replay ${cellText(r)}, live ${cellText(l)}`);
      rows.push(`| ${id} | ${a + 1} | ${cellText(r)} | ${cellText(l)} | ${same} | ${facts(id, a + 1)} |`);
    }
  }
  const tl = (res) => res.tallies.map((t) => `${t.name}: ${t.W}/${t.T}/${t.L}`).join('; ');
  const text = ['# Replay (scored-1) against live-scripted', '', `Replay records: ${rep.length}; live records: ${liv.length}. A cell is "same" when both are measured (OK) and the majority code is equal.`, '',
    `Replay W/T/L of arm 4 over ${ids.length} disasters (D15 not played here): ${tl(A1)}`, '', `Live W/T/L: ${tl(A2)}`, '', ...rows, '', `Cells that are not "same" (${diffs.length}):`, '', ...diffs.map((x) => `- ${x}`), ''].join('\n');
  fs.writeFileSync(o.file, text);
  console.log(`compare: ${diffs.length} of ${ids.length * 4} cells are not "same"; replay ${tl(A1)}; live ${tl(A2)}`);
}

// ------------------------------------------------------------------ commands

const fmtRun = (r) => `${r.disaster} a${r.arm} r${r.repetition} t${r.attempt}${r.control ? ` ${r.control}` : ''} -> ${r.outcome}${r.oracle?.line ? ` [${r.oracle.line}]` : r.outcome_reason ? ` (${cap(r.outcome_reason, 160)})` : ''}`
  + ` calls ${r.calls?.matched ?? '?'}/${r.calls?.scripted ?? '?'} extra ${r.calls?.extra?.length ?? '?'}${r.batched ? ' batched' : ''}${r.denied ? ' denied' : ''} hi $${r.cost?.hi} ${r.minutes} min`;

async function main() {
  process.umask(0o022);
  const [cmd, ...rest] = process.argv.slice(2);
  const o = parseArgs(rest);
  if (cmd === 'prompts') {
    fs.mkdirSync(path.resolve(o.out), { recursive: true });
    fs.writeFileSync(path.join(path.resolve(o.out), 'prompts.md'), promptsMd(loadDisasters(), orderOf()));
    console.log('prompts.md written');
    return;
  }
  if (cmd === 'ledger') return ledger(o);
  if (cmd === 'compare') return compareTable(o);
  const ctx = setup(o, cmd);
  try {
    const disasters = loadDisasters();
    const dOf = (id) => disasters.find((x) => x.id === id);
    if (cmd === 'smoke') {
      const { record: r } = await runSlot(ctx, 'smoke', { d: dOf(o.disaster ?? 'D01'), arm: Number(o.arm ?? 1), rep: 1 });
      console.log(`${fmtRun(r)} sessions=${JSON.stringify(r.sessions)} tap=${JSON.stringify(r.tap_events)}\ngate=${JSON.stringify(r.gate)} reason=${r.outcome_reason}\nnotes=${JSON.stringify(r.notes)}\ntrace=${cap(JSON.stringify(r.trace), 2500)}`);
    } else if (cmd === 'calibrate') {
      const file = path.join(ctx.out, 'calibration/runs.jsonl');
      const have = fs.existsSync(file) ? fs.readFileSync(file, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l)) : [];
      if (have.length >= 3) throw new Error('3 calibration runs are done');
      const id = CALIBRATION[have.length];
      const { record: r, stop } = await runSlot(ctx, 'calibrate', { d: dOf(id), arm: 1, rep: 1 });
      // Only what proves the driver: whether the trigger call happened, infra, cost, minutes; the trigger call's exit only when the environment broke it.
      const env = r.outcome === 'INFRA-FAILURE' && /^environment:/.test(r.outcome_reason ?? '') ? ` ${cap(r.outcome_reason, 200)}` : '';
      console.log(`calibration ${have.length + 1} (${id}): trigger=${r.outcome === 'NOT-TRIGGERED' ? 'no' : r.outcome === 'OK' ? 'yes' : '?'} infra=${r.outcome === 'INFRA-FAILURE' ? `yes (${cap(r.outcome_reason, 200)})` : 'no'}${env} hi $${r.cost?.hi} lo $${r.cost?.lo} minutes ${r.minutes}${stop ? ` STOP ${stop.why}` : ''}`);
      if (stop) process.exitCode = 10;
    } else if (cmd === 'batch') {
      if (ctx.driver.dirty) throw new Error('the driver files have uncommitted changes: commit them first (every record carries the driver commit and hashes)');
      fs.mkdirSync(ctx.out, { recursive: true });
      const md = readMd(path.join(REPO, 'drill/DISASTERS.md')), prereg = readMd(path.join(REPO, 'drill/PREREG.md'));
      const reps = String(o.reps ?? '1,2,3').split(',').map(Number);
      const arms = String(o.arms ?? '1,2,3,4').split(',').map(Number);
      fs.writeFileSync(path.join(ctx.out, 'method.json'), `${JSON.stringify({ batch: ctx.batchId, batch_label: L.BATCH_LABEL, method: ctx.method, driver: ctx.driver, prov: ctx.prov }, null, 2)}\n`);
      const hf = path.join(ctx.out, 'run-log.json');
      const header = { schema: 'torpor-drill-runlog/1', mode: 'live', deny_contract: { name: 'real Muse (no emulated contract)', rules: [] }, gate: { max_polls: 12, sleep_ms: 5000, overridden: false },
        prereg_sha256: shaText(prereg), disasters_sha256: shaText(md), harness: { sha256: ctx.driver.sha256, files: Object.keys(ctx.driver.sha256), repo_head: ctx.driver.commit }, provenance: ctx.prov,
        node: process.version, umask: '022', sandbox: 'bwrap', reps: reps.length };
      const old = readJson(hf);
      if (old && JSON.stringify(old.header.provenance) !== JSON.stringify(header.provenance)) throw new Error('run-log.json holds a different provenance (engine, adapter or Muse changed since the batch began)');
      fs.writeFileSync(hf, `${JSON.stringify({ batch_id: ctx.batchId, created_at: old?.created_at ?? utc(), header }, null, 2)}\n`);
      const file = path.join(ctx.out, 'runs.jsonl');
      const slots = L.slotList({ order: orderOf(), arms, reps, nc: o.nc !== '0', only: o.only ? String(o.only).split(',') : null });
      let envRun = 0;
      for (const s of slots) {
        const have = fs.existsSync(file) ? fs.readFileSync(file, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l)).filter((r) => r.disaster === s.id && r.arm === s.arm && r.repetition === s.rep && (r.control ?? null) === s.control) : [];
        if (have.some((r) => r.outcome !== 'INFRA-FAILURE') || have.length >= 3) continue;
        const { record: r, stop } = await runSlot(ctx, 'counted', { ...s, d: dOf(s.id) }, have.length + 1);
        console.log(`${utc()} ${fmtRun(r)}`);
        if (stop) { console.log(`STOP (${stop.why}): ${stop.msg}`); process.exitCode = stop.why === 'cost' ? 11 : 10; return; }
        envRun = r.env_class || /^environment:/.test(r.outcome_reason ?? '') ? envRun + 1 : 0;
        if (envRun >= 2) { console.log('STOP: two environment-class failures in a row'); process.exitCode = 12; return; }
      }
      console.log('batch finished');
    } else throw new Error('usage: live-scripted.mjs smoke|prompts|calibrate|batch|ledger|compare --out DIR ...');
  } finally {
    safeDelete(ctx.S.dir);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => { console.error(e.message); process.exitCode = 1; });
}

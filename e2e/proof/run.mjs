#!/usr/bin/env node
// L4 proof driver: plays the scripted sessions of e2e/proof/scripts.mjs in the real Muse Code TUI (tmux, scripts/sandbox/muse-sandbox.sh)
// with every hook command run through e2e/proof/hookwrap.c, which logs timing and hook output and changes nothing else.
// It writes one folder of raw records per session (private: kept outside the repo); analyze.mjs turns them into numbers.
//   node run.mjs build                                  compile hookwrap.c into the temp root
//   node run.mjs hashes                                 sha256 of both binaries, Muse, the wrapper and the driver files
//   node run.mjs prompts                                every fixed prompt and script, as the model gets it
//   node run.mjs calprompts                             the held-out calibration sessions of cal-scripts.mjs (L4c) and cal-scripts-l4d.mjs (L4d), as the model gets them
//   node run.mjs wrapcost [--n 400]                     the wrapper's own cost on a no-op command
//   node run.mjs run --session ID [--echo] [--locked]   one session (--echo = free echo provider, no key, no network; --turns N = only the first N turns, echo only)
//   node run.mjs ledger --file F                        the spend ledger from the recorded usage
// Common options: --root DIR (project root; keys/, scratch/ below it) --tmp-base DIR --out DIR (private records) --adapter F --engine F
//   --hookwrap F --muse-dir DIR --key-file F --cap 5 --run-cap 0.30 --label NAME --calibration
// Keyed runs must be started as `flock /tmp/claude-1000/meta-keyed.lock node run.mjs run ... --locked`; the key is read from the key file by the
// tmux pane's shell for that one command and never passes through this program, its arguments or any other file.
import cp from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import zlib from 'node:zlib';
import { fileURLToPath } from 'node:url';
import { egressRead, epProcs, killEp } from '../../eval/run/sandbox.mjs';
import * as A from '../../drill/harness/arms.mjs';
import { MODEL, MUSE_ENV, inspect, priceRow, records, usageRows } from '../../drill/harness/live.mjs';
import { GuardError, InfraError, NODE_BIN, NODE_EXEC, findBwrap, makeRunFolder, makeSandbox, safeDelete, toolEnv } from '../../drill/harness/sandbox.mjs';
import { compactionFacts, diffManifest, estAware, ledgerRows, manifest, parseWrapLog, stats, sumBy, wrapHooks } from './lib.mjs';
import { ASSETS, E9_PACKAGE_JSON, E9_TEST_JS, HOLDER_JS, SESSIONS, extrasText, fill, promptsText } from './scripts.mjs';
import { CAL_SESSIONS, calText } from './cal-scripts.mjs';
import { CAL_L4D_SESSIONS, calTextL4d } from './cal-scripts-l4d.mjs';
import { killListed, lockWatcher, procsInside } from './measures.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const SANDBOX = path.join(REPO, 'scripts/sandbox/muse-sandbox.sh');
const COMMON = ['--trust-workspace', '--disable-approval', '--approval-judge', 'off', '--contributor-mode', 'off'];
const KEYED = ['--provider', 'meta', '--model', MODEL, '--reasoning-effort', 'medium', ...COMMON];
const FREE = ['--provider', 'echo', ...COMMON];
const GIB = 1024 ** 3;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const priceOf = estAware(priceRow);
const sumCost = (rows) => sumBy(rows, priceOf);
const utc = () => new Date().toISOString();
const sha = (f) => crypto.createHash('sha256').update(fs.readFileSync(f)).digest('hex');
const round = (x, n = 6) => Math.round(x * 10 ** n) / 10 ** n;
const hashDir = (dir) => { try { return Object.fromEntries(fs.readdirSync(dir).sort().map((n) => [n, fs.statSync(path.join(dir, n)).isFile() ? sha(path.join(dir, n)) : 'not-a-file'])); } catch { return null; } };
const freeGiB = (p) => { const s = fs.statfsSync(p); return (s.bavail * s.bsize) / GIB; };
const TMUX_ENV = { PATH: '/usr/local/bin:/usr/bin:/bin', TERM: 'xterm-256color', LANG: 'C.UTF-8' };

function parseArgs(argv) {
  const o = {};
  for (let i = 0; i < argv.length; i++) {
    if (!argv[i].startsWith('--')) throw new Error(`unexpected argument ${argv[i]}`);
    const k = argv[i].slice(2), v = argv[i + 1];
    if (v === undefined || v.startsWith('--')) o[k] = true; else { o[k] = v; i++; }
  }
  return o;
}

function mkctx(o) {
  const root = path.resolve(o.root ?? path.join(REPO, '..', '..'));
  const tmpBase = path.resolve(o['tmp-base'] ?? '/tmp/claude-1000/l4');
  fs.mkdirSync(tmpBase, { recursive: true });
  const ctx = {
    root, tmpBase, out: path.resolve(o.out ?? path.join(root, 'scratch/torpor-drill/l4-proof')),
    museDir: path.resolve(o['muse-dir'] ?? path.join(root, 'scratch/torpor-g0/muse-install')),
    keyFile: path.resolve(o['key-file'] ?? path.join(root, 'keys/meta-api.txt')),
    engine: path.resolve(o.engine ?? path.join(root, 'scratch/torpor-engine/bin/torpor')),
    adapter: path.resolve(o.adapter ?? path.join(tmpBase, 'target/release/torpor-muse')),
    hookwrap: path.resolve(o.hookwrap ?? path.join(tmpBase, 'hookwrap')),
    nodeRoot: path.resolve(o['node-root'] ?? path.dirname(NODE_BIN)),
    billingOk: o['billing-ok'] ?? '/tmp/claude-1000/meta-billing-ok',
    cap: Number(o.cap ?? 5), runCap: Number(o['run-cap'] ?? 0.3), minRoot: 11.5, minTmp: 6,
  };
  for (const [k, v] of Object.entries({ museDir: ctx.museDir, keyFile: ctx.keyFile, nodeRoot: ctx.nodeRoot, node: NODE_EXEC, tmpBase, adapter: ctx.adapter, engine: ctx.engine, hookwrap: ctx.hookwrap })) {
    if (!/^[\w@%+=:,./-]+$/.test(v)) throw new Error(`${k} has a character that the pane command cannot carry safely`);
  }
  return ctx;
}

// ------------------------------------------------------------------ small commands

function hashes(ctx) {
  const mbin = fs.readdirSync(ctx.museDir).find((f) => /^muse-bin-/.test(f));
  const git = (...a) => cp.spawnSync('git', ['-C', REPO, ...a], { encoding: 'utf8' }).stdout.trim();
  const own = Object.fromEntries(['hookwrap.c', 'lib.mjs', 'scripts.mjs', 'cal-scripts.mjs', 'cal-scripts-l4d.mjs', 'measures.mjs', 'cases.mjs', 'run.mjs', 'analyze.mjs', 'selftest.mjs'].filter((f) => fs.existsSync(path.join(HERE, f))).map((f) => [f, sha(path.join(HERE, f))]));
  return {
    adapter: { sha256: sha(ctx.adapter), bytes: fs.statSync(ctx.adapter).size }, engine: { sha256: sha(ctx.engine) },
    hookwrap: fs.existsSync(ctx.hookwrap) ? { sha256: sha(ctx.hookwrap) } : null,
    muse: { launcher_sha256: sha(path.join(ctx.museDir, 'muse')), binary: mbin ? { name: mbin, sha256: sha(path.join(ctx.museDir, mbin)) } : null },
    source_commit: git('rev-parse', 'HEAD'), driver_sha256: own,
  };
}

function build(ctx) {
  const r = cp.spawnSync('gcc', ['-O2', '-Wall', '-Wextra', '-o', ctx.hookwrap, path.join(HERE, 'hookwrap.c')], { encoding: 'utf8' });
  if (r.status !== 0) throw new Error(`gcc failed: ${r.stderr.slice(0, 400)}`);
  return { path: path.basename(ctx.hookwrap), sha256: sha(ctx.hookwrap) };
}

// The wrapper's own cost: the same no-op command bare and wrapped, interleaved, timed from outside around the whole process.
function wrapcost(ctx, n) {
  const log = path.join(ctx.tmpBase, 'wrapcost.log');
  fs.rmSync(log, { force: true });
  const run = (cmd) => { const t = process.hrtime.bigint(); const r = cp.spawnSync('/bin/sh', ['-c', cmd], { stdio: 'ignore' }); if (r.status !== 0) throw new Error(`no-op failed: ${cmd}`); return Number(process.hrtime.bigint() - t); };
  const wrapped = `${ctx.hookwrap} ${log} Stop ':'`;
  for (let i = 0; i < 20; i++) { run(':'); run(wrapped); } // warm-up, not counted
  const bare = [], wrap = [], diff = [];
  for (let i = 0; i < n; i++) {
    const a = i % 2 ? run(wrapped) : run(':');
    const b = i % 2 ? run(':') : run(wrapped);
    const [x, y] = i % 2 ? [b, a] : [a, b];
    bare.push(x); wrap.push(y); diff.push(y - x);
  }
  const ms = (l) => l.map((x) => x / 1e6);
  const logged = parseWrapLog(fs.readFileSync(log, 'utf8')).runs.slice(20).map((r) => r.ns);
  fs.rmSync(log, { force: true });
  return { n, unit: 'ms', bare: stats(ms(bare)), wrapped: stats(ms(wrap)), added_by_pair: stats(ms(diff)), child_interval_in_wrapper: stats(ms(logged)), at: utc() };
}

// ------------------------------------------------------------------ one session

const tm = (S, ...a) => cp.spawnSync('tmux', ['-S', S.sock, ...a], { encoding: 'utf8', env: TMUX_ENV });
const pane = (S) => tm(S, 'capture-pane', '-p', '-t', 'p').stdout ?? '';
const typeLine = async (S, text) => { tm(S, 'send-keys', '-t', 'p', '-l', '--', text); await sleep(400); tm(S, 'send-keys', '-t', 'p', 'Enter'); };
const pasteText = async (S, text) => {
  cp.spawnSync('tmux', ['-S', S.sock, 'load-buffer', '-b', 'l4', '-'], { input: text, env: TMUX_ENV });
  tm(S, 'paste-buffer', '-p', '-b', 'l4', '-t', 'p');
  await sleep(600);
  tm(S, 'send-keys', '-t', 'p', 'Enter');
};
const walkFiles = (dir, pred, out = []) => {
  let es;
  try { es = fs.readdirSync(dir, { withFileTypes: true }); } catch { return out; }
  for (const e of es) { const p = path.join(dir, e.name); if (e.isDirectory()) walkFiles(p, pred, out); else if (pred(p)) out.push(p); }
  return out;
};
// L4d measure (iii): the workspace's small files as they were when a claim was typed (the check-writer reads them then), kept to map the saved check to file lines. Reads only.
const snapshot = (ws, dest) => {
  for (const f of walkFiles(ws, (p) => !/\/(\.git|node_modules)\//.test(p) && fs.statSync(p).size < 2e5)) {
    const to = path.join(dest, path.relative(ws, f));
    fs.mkdirSync(path.dirname(to), { recursive: true });
    fs.copyFileSync(f, to);
  }
};
const isSub = (p) => p.includes(`${path.sep}subagent${path.sep}`);
const sessionFiles = (T) => walkFiles(path.join(T, 'home/.local/share/muse/sessions'), (p) => path.basename(p) === 'session.jsonl').sort();
const writerLedger = (T) => walkFiles(path.join(T, 'torpor-home'), (p) => path.basename(p) === 'ledger.jsonl').flatMap((f) => fs.readFileSync(f, 'utf8').split('\n').filter(Boolean).flatMap((l) => { try { return [JSON.parse(l)]; } catch { return []; } }));

// The engine's own records, copied read-only (E1b: which Stop ran a saved check; the receipt check: which save point ends which turn). In promise_runs a row
// of kind gate is a saved check run by a Stop; `at` (ns since the epoch, kept as text) is when its batch finished.
async function engineRecords(T) {
  const file = walkFiles(path.join(T, 'torpor-home'), (p) => path.basename(p) === 'torpor.db')[0];
  if (!file) return null;
  const { DatabaseSync } = await import('node:sqlite');
  const d = new DatabaseSync(file, { readOnly: true });
  try {
    const all = (sql) => d.prepare(sql).all();
    return {
      promise_runs: all('select id, promise_id, kind, verdict, failed_step, ms, cast(at as text) as at_ns from promise_runs order by id'),
      turns: all('select id, n, start_point, end_point, end_kind, origin, cast(started_at as text) as started_ns, cast(ended_at as text) as ended_ns, bash_count from turns order by id'),
      points: all('select id, kind, visible, turn_id, partial, dropped, note, cast(created_at as text) as created_ns from points order by id'),
    };
  } finally { d.close(); }
}

class Stop extends Error { constructor(msg, why, stage = false) { super(msg); this.why = why; this.stage = stage; } }
let active = null;
function killActive() { if (active) { tm(active.S, 'kill-server'); killEp(active.T); } }
process.removeAllListeners('SIGINT'); process.removeAllListeners('SIGTERM'); // drill/harness/live.mjs registers its own handlers on import, which exit at once and would pre-empt ours
let aborting = null; // first SIGINT/SIGTERM during a run: stop at the next tick and still write calls.json and run.json (spend must be counted); a second one, or none running: exit now
for (const s of ['SIGINT', 'SIGTERM']) process.on(s, () => { if (aborting || !active) { killActive(); process.exit(130); } aborting = s; });
process.on('exit', killActive);

// -> workspace path. demo = the login app of demo/ with one baseline commit; d06 / d07 = the drill's database workspaces.
function buildWorkspace(ctx, sb, T, spec) {
  const env = toolEnv(T);
  if (spec.ws === 'demo') {
    const ws = path.join(T, 'ws');
    fs.mkdirSync(ws);
    const main = spec.main ?? 'server.js'; // only the L4d calibration sessions rename the app file
    fs.copyFileSync(path.join(REPO, 'demo/app/server.js'), path.join(ws, main));
    fs.writeFileSync(path.join(ws, 'package.json'), spec.tests ? E9_PACKAGE_JSON : `${JSON.stringify({ name: 'login-demo', version: '1.0.0', private: true, scripts: { start: `node ${main}` } }, null, 2)}\n`);
    if (spec.tests) { fs.mkdirSync(path.join(ws, 'test')); fs.writeFileSync(path.join(ws, 'test/login.test.js'), E9_TEST_JS); }
    fs.writeFileSync(path.join(ws, 'README.md'), '# Login demo\n\nA small login page. Start it with `npm start` (port 3000, or `--port N`).\n');
    fs.writeFileSync(path.join(ws, '.synthetic'), 'synthetic workspace for the L4 proof\n');
    fs.mkdirSync(path.join(T, 'home'), { recursive: true });
    fs.writeFileSync(path.join(T, 'home/.gitconfig'), '[user]\n\tname = drill\n\temail = drill.invalid\n[core]\n\thooksPath = /dev/null\n[commit]\n\tgpgsign = false\n[init]\n\tdefaultBranch = main\n');
    for (const d of ['tmp', 'muse-share']) fs.mkdirSync(path.join(T, d), { recursive: true });
    const r = sb.exec({ argv: ['sh', '-c', 'git init -q && git add -A && git commit -q -m baseline && git rev-parse --short HEAD'], cwd: ws, env, merge: true, timeoutMs: 60000 });
    if (r.exit !== 0) throw new InfraError(`git baseline failed: ${r.stdout.slice(0, 200)}`);
    return ws;
  }
  const dir = { d06: 'D06', d07: 'D07', d12: 'D12' }[spec.ws];
  if (!dir) throw new GuardError(`unknown workspace ${spec.ws}`);
  const r = sb.exec({ argv: ['sh', path.join(REPO, 'drill/workspaces', dir, 'build.sh'), T], cwd: T, env, ro: [path.join(REPO, 'drill/workspaces')], merge: true, timeoutMs: 180000 });
  if (r.exit !== 0) throw new InfraError(`the builder failed (exit ${r.exit}): ${r.stdout.trim().slice(0, 300)}`);
  const ws = path.join(T, 'ws');
  if (!fs.existsSync(path.join(ws, '.synthetic'))) throw new InfraError('the builder left no .synthetic marker');
  return ws;
}

async function runSession(ctx, spec, o) {
  const echo = !!o.echo, label = o.label ?? `${spec.id}-${utc().replace(/\D/g, '').slice(4, 14)}`;
  const base = path.join(ctx.out, echo ? 'echo-runs' : 'runs', label); // echo (free) runs stay out of the ledger
  fs.mkdirSync(base, { recursive: true });
  const rec = { schema: 'torpor-l4-run/1', id: spec.id, label, mode: echo ? 'echo' : 'keyed', calibration: !!o.calibration, started_at: utc(), turns: [], notes: [], go: null, procs: {} };
  const t0 = Date.now();
  let T = null, sb = null, outcome = 'INFRA-FAILURE', reason = null, stageStop = false, phase = 'setup', ws = null, watchTimer = null, locks = null;
  const turnsDone = () => rec.turns.filter((t) => t.done).length;
  const wrapLog = () => { try { return parseWrapLog(fs.readFileSync(path.join(T, 'live/wrap.log'), 'utf8')); } catch { return { runs: [], killed: [], bad: [] }; } };
  let cache = { sig: '', logs: [] };
  const readLogs = () => {
    const files = sessionFiles(T);
    const sig = files.map((f) => `${f}:${fs.statSync(f).size}`).join('|');
    if (sig !== cache.sig) cache = { sig, logs: files.map((f) => ({ f, recs: records(fs.readFileSync(f, 'utf8')) })) };
    return cache.logs;
  };
  // usd / est / status are the adapter's own ledger row (a cross-check of our pricing; est = a call whose cost it could only estimate)
  const writerRows = () => writerLedger(T).map((r) => ({ at: Date.parse(r.at) * 1000, call: 'writer', model: r.model, input: r.input ?? 0, cached: r.cached ?? 0, output: r.output ?? 0, reasoning: 0, usd: r.usd ?? null, est: !!r.est, status: r.status ?? null }));
  const allRows = () => [...readLogs().flatMap((l) => usageRows(l.recs)), ...writerRows()];
  const spend = () => sumCost(allRows());
  const scanProcs = () => { try { return procsInside(T); } catch (e) { rec.notes.push(`process scan: ${String(e.message).slice(0, 120)}`); return null; } };
  try {
    if (freeGiB('/') < ctx.minRoot || freeGiB('/tmp') < ctx.minTmp) throw new Stop(`disk: / ${freeGiB('/').toFixed(1)} GiB, /tmp ${freeGiB('/tmp').toFixed(1)} GiB free`, 'disk', true);
    const pin = cp.spawnSync('sh', [path.join(REPO, 'scripts/verify-engine.sh'), ctx.engine, 'linux-x64'], { encoding: 'utf8' });
    if (pin.status !== 0) throw new GuardError(`the engine does not match engine.lock: ${(pin.stdout + pin.stderr).trim().slice(0, 200)}`);
    if (!echo) {
      if (!o.locked) throw new GuardError('keyed runs must run under flock and pass --locked');
      if (!fs.existsSync(ctx.billingOk)) throw new GuardError('the billing check file is missing');
      if (!fs.existsSync(ctx.keyFile)) throw new GuardError('the key file is missing');
    }
    ctx.S ??= (() => { const d = makeRunFolder(ctx.tmpBase); return { dir: d, sock: path.join(d, 'tm.sock') }; })();
    T = makeRunFolder(ctx.tmpBase);
    active = { S: ctx.S, T };
    sb = makeSandbox({ T, bwrap: findBwrap() });
    ws = buildWorkspace(ctx, sb, T, spec);
    for (const d of ['home/.config', 'tmp', 'torpor-home', 'muse-share', 'bin', 'live', 'shared']) fs.mkdirSync(path.join(T, d), { recursive: true });
    fs.writeFileSync(path.join(T, '.drill-run'), `${JSON.stringify({ id: spec.id, mode: rec.mode })}\n`);
    if (!fs.existsSync(path.join(T, 'home/.gitconfig'))) fs.writeFileSync(path.join(T, 'home/.gitconfig'), '[user]\n\tname = drill\n\temail = drill.invalid\n[core]\n\thooksPath = /dev/null\n[commit]\n\tgpgsign = false\n');
    if (spec.decoy) { // E8: the folder the AI is asked to delete, in this run's own home (HOME=<run>/home inside the sandbox). Never the real home.
      const dd = path.join(T, 'home', spec.decoy.dir);
      fs.mkdirSync(dd);
      for (const [name, text] of Object.entries(spec.decoy.files)) fs.writeFileSync(path.join(dd, name), text);
      const real = fs.realpathSync(dd), here = fs.realpathSync(T), realHome = os.userInfo().homedir;
      if (!real.startsWith(`${here}${path.sep}`) || real.startsWith(`${realHome}${path.sep}`)) throw new GuardError(`the decoy folder is not inside this run's own folder: ${real}`);
      rec.decoy = { dir: spec.decoy.dir, realpath: real, before: hashDir(real) };
    }
    if (spec.tests) { // E9: the app's own test, run once before Muse starts; the case only counts if it passes before the edit
      const r = sb.exec({ argv: ['sh', '-c', 'node --test 2>&1; echo exit=$?; git status --porcelain'], cwd: ws, env: toolEnv(T), merge: true, timeoutMs: 120000 });
      rec.pretest = { ok: `\n${r.stdout}`.includes('\nexit=0\n'), tail: r.stdout.slice(-700) };
      if (!rec.pretest.ok) throw new InfraError('the E9 test does not pass before the edit');
    }
    const inst = A.installPrograms(T, 4, { engine: ctx.engine, adapter: ctx.adapter });
    rec.binaries = { engine: inst.engine.sha256, adapter: inst.adapter.sha256, hookwrap: A.copyBinary(ctx.hookwrap, path.join(T, 'bin/hookwrap')) };
    const ti = Date.now();
    const init = A.runArm4Init(T, sb);
    rec.init_ms = Date.now() - ti; // the installer must not wait for a key prompt inside the sandbox (stage H asks only on a terminal)
    const wrapped = wrapHooks(init.text, { wrap: path.join(T, 'bin/hookwrap'), log: path.join(T, 'live/wrap.log') });
    fs.writeFileSync(A.settingsPath(T), wrapped.text);
    A.parseSettings(wrapped.text);
    fs.copyFileSync(A.settingsPath(T), path.join(base, 'settings.json'));
    rec.hooks_wrapped = { total: wrapped.total, with_key: wrapped.keyed };
    for (const [name, text] of Object.entries({ ...ASSETS, ...(spec.assets ?? {}) })) fs.writeFileSync(path.join(T, 'shared', name), text);
    let launchPrefix = [];
    if (spec.holder) {
      fs.writeFileSync(path.join(T, 'bin/holder.mjs'), HOLDER_JS);
      fs.writeFileSync(path.join(T, 'bin/start.sh'), '#!/bin/sh\n# stands in for the running app: holds the database open, then starts the real command\n"$1" "$2" "$3" >/dev/null 2>&1 &\nsleep 1\nshift 3\nexec "$@"\n', { mode: 0o755 });
      launchPrefix = [path.join(T, 'bin/start.sh'), NODE_EXEC, path.join(T, 'bin/holder.mjs'), path.join(ws, spec.holder)];
    }
    const sha1 = (p) => sha(p);
    const man = () => manifest(ws, sha1);
    const eng = (argv) => sb.exec({ argv: [path.join(T, 'bin/torpor'), ...argv], cwd: ws, env: toolEnv(T), merge: true, timeoutMs: 60000 }); // engine CLI between turns (read-only questions)
    rec.manifest_start = man();
    // L4d measures (i) and cal (b): the check-writer's lock spans, read from /proc/locks (no lock of ours); for a turn with go_on_save, the file its AI waits for
    // is created the moment the engine is seen saving a check (or after go_cap_s if it never is). All reads; a failure in here is noted and never stops the run.
    locks = lockWatcher(path.join(T, 'torpor-home'));
    let goArmed = null;
    watchTimer = setInterval(() => {
      try {
        locks.poll();
        if (goArmed && !rec.go) {
          const held = locks.held('save'), late = Date.now() - goArmed.since > goArmed.cap_s * 1000;
          if (held || late) { fs.writeFileSync(path.join(T, 'shared/go'), 'go\n'); rec.go = { reason: held ? 'save-held' : 'cap', after_ms: Date.now() - goArmed.since, at: Date.now() }; }
        }
      } catch (e) { if (rec.notes.length < 20) rec.notes.push(`lock watcher: ${String(e.message).slice(0, 120)}`); }
    }, 100);
    watchTimer.unref();

    if (!(spec.muse_flags ?? []).every((f) => /^[\w.=-]+$/.test(f))) throw new GuardError('a Muse flag has a character that the pane command cannot carry safely');
    const muse = [path.join(ctx.museDir, 'muse'), ...(echo ? FREE : KEYED), ...(spec.muse_flags ?? [])];
    const env = ['env', 'TERM=xterm-256color', `PATH=${T}/bin:${NODE_BIN}:/usr/local/bin:/usr/bin:/bin`, `TORPOR_HOME=${T}/torpor-home`, 'TZ=UTC', 'GIT_TERMINAL_PROMPT=0', ...MUSE_ENV];
    const launcher = [SANDBOX, '--run-dir', T, '--ro', ctx.museDir, '--ro', ctx.nodeRoot, '--net', echo ? 'none' : 'meta', ...(echo ? [] : ['--drain', '3']), '--chdir', ws, '--', ...launchPrefix, ...env, ...muse];
    const paneCmd = `${echo ? '' : `META_API_KEY="$(cat '${ctx.keyFile}')" `}${launcher.join(' ')}; echo EXITED $?; sleep 900`;
    cp.spawnSync('tmux', ['-S', ctx.S.sock, '-f', '/dev/null', 'new-session', '-d', '-s', 'p', '-x', '170', '-y', '50', '-c', ctx.S.dir, paneCmd], { env: TMUX_ENV });

    let diskAt = 0;
    const tick = () => {
      if (aborting) throw new Stop(`${aborting}: the run was stopped from outside`, 'signal', true);
      if (Date.now() - diskAt > 30000) {
        diskAt = Date.now();
        if (freeGiB('/') < ctx.minRoot || freeGiB('/tmp') < ctx.minTmp) throw new Stop(`disk: / ${freeGiB('/').toFixed(1)} GiB, /tmp ${freeGiB('/tmp').toFixed(1)} GiB free`, 'disk', true);
      }
      const logs = readLogs();
      const c = spend();
      if (c.hi > ctx.runCap) throw new Stop(`cost stop: $${c.hi.toFixed(4)} hi is above the $${ctx.runCap.toFixed(2)} per-run cap`, 'cost');
      if (ctx.spent + c.hi > ctx.cap) throw new Stop(`stage cap: $${(ctx.spent + c.hi).toFixed(4)} hi is above $${ctx.cap.toFixed(2)}`, 'cap', true);
      const main = logs.filter((l) => !isSub(l.f));
      const info = main[0] ? inspect(main[0].recs) : null;
      if (info?.rateLimited) throw new Stop('the provider answered 429 / rate limit', 'rate-limit', true);
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
    const norm = (s) => s.replace(/\s+/g, ' ').trim();

    let baseline = null, poll = null, restoredAt = null, undoSentAt = null;
    const harmedSame = () => diffManifest(baseline, man(), spec.harmed ?? null);
    const turn = async (n, t) => {
      const text = fill(t.text, { T, ws });
      const mark = { n, role: t.role ?? 'prompt', claim: !!t.claim, trigger: t.trigger, way: t.way };
      if (t.role === 'harm') {
        baseline = man();
        mark.git = sb.exec({ argv: ['sh', '-c', `git status --porcelain --ignored -- ${(spec.harmed ?? ['.']).join(' ')}; echo ---; git diff --quiet HEAD -- ${(spec.harmed ?? ['.']).join(' ')}; echo diff-exit=$?; git ls-files -- ${(spec.harmed ?? ['.']).join(' ')}`], cwd: ws, env: toolEnv(T), merge: true, timeoutMs: 30000 }).stdout.slice(0, 2000);
      }
      if (t.role === 'undo') {
        mark.restored_before_undo = baseline ? harmedSame().same : null;
        poll = setInterval(() => { if (restoredAt === null && undoSentAt !== null && baseline && harmedSame().same) restoredAt = Date.now(); }, 200).unref(); // unref: a turn that never finishes must not keep the process alive (departure 25)
      }
      // A person types after the last answer, not at the same instant: the detached check-writer of the earlier prompt gets to finish (else the adapter skips the next prompt as "busy").
      if (n > 1) { const w0 = Date.now(); await soft('the check-writer of the earlier prompt to finish', 90000, () => !epProcs(T).some((p) => /\bsuggest\b/.test(p.cmd))); mark.writer_wait_s = round((Date.now() - w0) / 1000, 1); }
      if (t.claim) { try { snapshot(ws, path.join(base, `claim-${n}`)); } catch (e) { rec.notes.push(`claim snapshot ${n}: ${String(e.message).slice(0, 120)}`); } }
      mark.sent = Date.now();
      if (t.go_on_save) goArmed = { since: mark.sent, cap_s: t.go_cap_s ?? 100 };
      if (t.role === 'undo') undoSentAt = mark.sent;
      rec.turns.push(mark);
      if (text.includes('\n')) await pasteText(ctx.S, text); else await typeLine(ctx.S, text);
      const info = await until(`the prompt of turn ${n} to be accepted`, 30000, (i) => (i && i.prompts.length >= n ? i : null));
      mark.accepted = Date.now();
      mark.prompt_chars = { sent: norm(text).length, accepted: norm(info.prompts[n - 1]).length };
      if (norm(info.prompts[n - 1]) !== norm(text)) throw new InfraError(`Muse accepted a different text for turn ${n} (${mark.prompt_chars.accepted} chars, sent ${mark.prompt_chars.sent})`);
      await until(`turn ${n} to finish`, t.timeout_s ? t.timeout_s * 1000 : 240000, (i) => {
        if (i && i.terminals.length >= n && i.terminals[n - 1].terminal !== 'completed') throw new InfraError(`turn ${n} ended with "${i.terminals[n - 1].terminal}"`);
        return i && i.completed >= n ? i : null;
      });
      mark.done = Date.now();
      goArmed = null;
      await soft(`the Stop hook of turn ${n}`, 40000, () => wrapLog().runs.filter((r) => r.ev === 'Stop').length >= n && wrapLog().killed.filter((k) => k.ev === 'Stop').length === 0);
      await sleep(1500);
      mark.settled = Date.now();
      mark.manifest = man();
      if (t.role === 'harm') mark.manifest_after_harm = mark.manifest;
      mark.promises_after = (eng(['promise', 'list']).stdout.match(/^#\d+ /gm) ?? []).length; // saved checks at the end of this turn (E1b needs to know)
      if (spec.fillers) { try { mark.rebuilds = JSON.parse(eng(['status', '--json']).stdout).context?.rebuilds ?? null; } catch { mark.rebuilds = null; } } // E7: the engine's own count of context rebuilds
      if (spec.decoy) { // E8: what the screen showed, kept privately; a permission ask would show here (the fixed flags switch asks off)
        const screen = pane(ctx.S);
        fs.writeFileSync(path.join(base, `pane-turn${n}.txt`), screen);
        mark.pane_ask_lines = screen.split('\n').filter((l) => /approv|permission|allow|confirm|\(y\/n\)|\[y\/N\]/i.test(l)).map((l) => l.trim().slice(0, 160)).slice(0, 5);
      }
      if (spec.v11) { // V11: did the way work? Read the engine's own flag, then turn Torpor on again so the next way starts from the same state.
        try { mark.torpor_off_after = JSON.parse(eng(['status', '--json']).stdout).off === true; } catch { mark.torpor_off_after = null; }
        if (mark.torpor_off_after) mark.turned_back_on_exit = eng(['on']).exit;
      }
      if (t.role === 'undo') { clearInterval(poll); poll = null; mark.restored_at = restoredAt; mark.restore_s = restoredAt ? round((restoredAt - undoSentAt) / 1000, 2) : null; }
    };

    phase = 'boot';
    await until('the TUI prompt', 90000, () => pane(ctx.S).includes('❯'));
    await sleep(1500);
    rec.tui_ready = Date.now();
    rec.procs.boot = scanProcs(); // measure (ii): what belongs to Muse itself, before the first prompt
    // E7: while no compaction has come after the saved check, a filler turn (a harmless read of a long file) goes in front of the break, at most spec.fillers.max
    // (echo: one). The check below is the Muse log's own record of an installed compaction followed by a SessionStart hook run.
    const mainRecs = () => readLogs().filter((l) => !isSub(l.f))[0]?.recs ?? [];
    const compacted = () => compactionFacts(mainRecs()).after_save.some((c) => c.session_start);
    let typed = 0;
    rec.filler_turns = 0;
    for (const [i, t] of spec.turns.entries()) {
      if (spec.fillers && t.role === 'harm') {
        for (let f = 1; f <= (echo ? 1 : spec.fillers.max); f++) {
          phase = `filler${f}`;
          if (await soft(`a compaction after the saved check (look ${f})`, echo ? 5000 : f === 1 ? 40000 : 25000, compacted)) break;
          await turn(++typed, spec.fillers.turn);
          rec.filler_turns++;
        }
        phase = `after-filler`;
        await soft('a compaction after the last filler turn', echo ? 3000 : 25000, compacted);
      }
      phase = `turn${i + 1}`;
      await turn(++typed, t);
    }
    phase = 'exit';
    // The check-writer runs detached after a prompt; its call is billed when it starts, so wait for it before Muse (and the sandbox) goes away.
    await soft('the check-writer to finish', 150000, () => !epProcs(T).some((p) => /\bsuggest\b/.test(p.cmd)));
    rec.procs.pre_exit = scanProcs(); // measure (ii): record only, nothing is killed while Muse is still up
    await typeLine(ctx.S, '/exit');
    await soft('Muse to exit', 30000, () => /EXITED \d+/.test(pane(ctx.S)));
    tick();
    outcome = 'DONE';
    rec.baseline = baseline;
  } catch (e) {
    if (e instanceof Stop) { outcome = e.why === 'cost' ? 'STOPPED-COST' : 'INFRA-FAILURE'; reason = e.message; stageStop = e.stage; }
    else if (e instanceof GuardError) { outcome = 'REFUSED'; reason = e.message; }
    else { reason = e instanceof InfraError ? e.message : `harness defect: ${String(e.stack ?? e).split('\n').slice(0, 4).join(' | ')}`; stageStop = !(e instanceof InfraError); }
    if (T && ctx.S) fs.writeFileSync(path.join(base, `pane-failed-in-${phase}.txt`), pane(ctx.S)); // private, never printed
  }

  // --- collect (Muse first, then the run folder), after the Muse process is gone
  try {
    if (T) {
      // measure (ii): after Muse has exited, what still has its working folder in the run folder; those are killed by exact pid (re-read just before), then the old sweep runs as before
      await sleep(300);
      rec.procs.muse_exited = /EXITED \d+/.test(pane(ctx.S));
      rec.procs.post_exit = scanProcs();
      rec.procs.post_exit_killed = killListed(T, rec.procs.post_exit ?? []);
      tm(ctx.S, 'kill-server');
      killEp(T);
      for (let i = 0; i < 20 && epProcs(T).length; i++) await sleep(250);
      const logs = readLogs();
      const museRows = logs.flatMap((l, i) => usageRows(l.recs).map((u) => ({ ...u, log: isSub(l.f) ? `sub-${i}` : 'main' })));
      const wRows = writerRows();
      const cm = sumCost(museRows), cw = sumCost(wRows);
      fs.writeFileSync(path.join(base, 'calls.json'), `${JSON.stringify({ started_at: rec.started_at, rows: [...museRows, ...wRows] })}\n`);
      logs.forEach((l, i) => fs.writeFileSync(path.join(base, isSub(l.f) ? `session-sub-${i}.jsonl.gz` : 'session.jsonl.gz'), zlib.gzipSync(fs.readFileSync(l.f))));
      if (fs.existsSync(path.join(T, 'live/wrap.log'))) fs.copyFileSync(path.join(T, 'live/wrap.log'), path.join(base, 'wrap.log'));
      if (fs.existsSync(path.join(T, 'net/proxy.log'))) fs.copyFileSync(path.join(T, 'net/proxy.log'), path.join(base, 'proxy.log'));
      const ledgerFiles = walkFiles(path.join(T, 'torpor-home'), (p) => /\.(jsonl|json)$/.test(p) && fs.statSync(p).size < 2e6 && p.includes(`${path.sep}muse${path.sep}`));
      ledgerFiles.forEach((f, i) => fs.copyFileSync(f, path.join(base, `muse-state-${i}-${path.basename(f)}`)));
      const w = wrapLog();
      const engineCmd = (argv) => sb.exec({ argv: [path.join(T, 'bin/torpor'), ...argv], cwd: ws, env: toolEnv(T), merge: true, timeoutMs: 120000 });
      if (ws && outcome !== 'REFUSED') {
        const st = engineCmd(['status', '--json']);
        fs.writeFileSync(path.join(base, 'status.json'), st.stdout);
        const plist = engineCmd(['promise', 'list']).stdout;
        fs.writeFileSync(path.join(base, 'promise-list.txt'), plist);
        // measure (iii): the stored check of every saved promise, raw (read-only), to see which file lines each one is about
        fs.writeFileSync(path.join(base, 'promise-specs.json'), `${JSON.stringify(Object.fromEntries([...plist.matchAll(/^#(\d+) /gm)].map((m) => [m[1], engineCmd(['promise', 'show', m[1], '--spec']).stdout.slice(0, 70000)])))}\n`);
        fs.writeFileSync(path.join(base, 'engine-log.txt'), engineCmd(['log']).stdout.slice(0, 200000));
        const er = await engineRecords(T); // before the final check run below, which may add rows of its own
        if (er) {
          fs.writeFileSync(path.join(base, 'engine-runs.json'), `${JSON.stringify(er)}\n`);
          fs.writeFileSync(path.join(base, 'engine-show.txt'), er.turns.map((t) => `=== turn ${t.n}\n${engineCmd(['show', String(t.n)]).stdout.slice(0, 4000)}`).join('\n'));
        }
        if (spec.check && outcome === 'DONE') {
          const t1 = Date.now();
          const c = engineCmd(['check', '--all']);
          rec.final_check = { exit: c.exit, seconds: round((Date.now() - t1) / 1000, 2) };
          fs.writeFileSync(path.join(base, 'final-check.txt'), c.stdout.slice(0, 20000));
        }
        rec.manifest_end = manifest(ws, sha);
        if (rec.decoy) { rec.decoy.after = hashDir(rec.decoy.realpath); rec.decoy.same = JSON.stringify(rec.decoy.after) === JSON.stringify(rec.decoy.before); }
        rec.git_end = sb.exec({ argv: ['sh', '-c', 'git status --porcelain; git log --oneline | head -5'], cwd: ws, env: toolEnv(T), merge: true, timeoutMs: 30000 }).stdout.slice(0, 2000);
      }
      const main = logs.filter((l) => !isSub(l.f));
      const info = main.length === 1 ? inspect(main[0].recs) : null;
      rec.cost = { muse: { ...Object.fromEntries(Object.entries(cm).map(([k, v]) => [k, round(v)])), calls: museRows.filter((x) => x.call === 'model').length }, writer: { ...Object.fromEntries(Object.entries(cw).map(([k, v]) => [k, round(v)])), calls: wRows.length }, total_hi: round(cm.hi + cw.hi), total_lo: round(cm.lo + cw.lo) };
      rec.sessions = { main: main.length, subagents: logs.length - main.length };
      rec.turns_logged = info ? { prompts: info.prompts.length, completed: info.completed, terminals: info.terminals.map((t) => t.terminal) } : null;
      rec.hook_runs = { n: w.runs.length, killed: w.killed.length, bad_lines: w.bad.length, by_event: Object.fromEntries([...new Set(w.runs.map((r) => r.ev))].map((ev) => [ev, w.runs.filter((r) => r.ev === ev).length])) };
      rec.key = { runs_had_key: w.runs.filter((r) => r.hadKey === 1).length, runs_got_key: w.runs.filter((r) => r.gotKey === 1).length, ups: w.runs.filter((r) => r.ev === 'UserPromptSubmit').length };
      rec.egress = egressRead(T);
      if (locks) rec.locks = { files: locks.files, spans: locks.close() };
      rec.e1_floor_ms = stats(w.runs.map((r) => r.ns / 1e6));
    }
  } catch (e) {
    outcome = 'INFRA-FAILURE';
    reason = `collect: ${e instanceof InfraError ? e.message : String(e.stack ?? e).split('\n').slice(0, 4).join(' | ')}`;
    stageStop = !(e instanceof InfraError);
  } finally {
    if (watchTimer) clearInterval(watchTimer);
    if (sb) sb.reapRun();
    if (T) { killEp(T); safeDelete(T); }
    active = null;
  }
  Object.assign(rec, { outcome, reason, ended_at: utc(), minutes: round((Date.now() - t0) / 60000, 2), spec_turns: spec.turns.length, typed_turns: rec.turns.length });
  fs.writeFileSync(path.join(base, 'run.json'), `${JSON.stringify(rec)}\n`);
  return { rec, stage: stageStop };
}

// Dollars already spent by recorded runs (hi), calibration included.
const spentSoFar = (out) => (fs.existsSync(path.join(out, 'runs')) ? fs.readdirSync(path.join(out, 'runs')) : []).reduce((s, d) => {
  try { return s + (JSON.parse(fs.readFileSync(path.join(out, 'runs', d, 'run.json'), 'utf8')).cost?.total_hi ?? 0); } catch { return s; }
}, 0);

function ledger(ctx, file, cap) {
  const runs = (fs.existsSync(path.join(ctx.out, 'runs')) ? fs.readdirSync(path.join(ctx.out, 'runs')) : []).map((d) => {
    const f = path.join(ctx.out, 'runs', d, 'calls.json');
    return fs.existsSync(f) ? { label: d, ...JSON.parse(fs.readFileSync(f, 'utf8')) } : null;
  }).filter(Boolean).sort((a, b) => a.started_at.localeCompare(b.started_at));
  const { lines, lo, hi } = ledgerRows(runs, priceOf);
  fs.writeFileSync(file, [`# L4 proof spend ledger (cap $${cap.toFixed(2)} hi for the whole stage, calibration included; every model call counts, failed runs too)`, '',
    `Model: ${MODEL} (Meta API, contributor-priced id) driven by the Muse Code TUI under tmux (e2e/proof/run.mjs); call = model (Muse's own calls), compaction, or writer (the check-writer's calls, read from its own ledger). One row per call, read from the records by script.`,
    'Prices per 1M tokens: contributor ids cached input $0.002, input $0.10, output $0.20; other ids cached $0.15, input $1.25, output $4.25 (eval/run/lib.mjs cost()).',
    'lo = (input - cached) x input price + cached x cached price + output x output price; hi = input x input price + cached x cached price + (output + reasoning) x output price (upper bound if reasoning is not inside output).', '',
    '| # | run | call | time UTC | model | input | cached | output | reasoning | $ lo | $ hi | cumulative $ hi |', '|---|-----|------|----------|-------|-------|--------|--------|-----------|------|------|-----------------|',
    ...lines, '', `Total: ${lines.length} calls, $ lo ${lo.toFixed(4)}, $ hi ${hi.toFixed(4)} of the $${cap.toFixed(2)} cap.`, ''].join('\n'));
  return { calls: lines.length, lo, hi };
}

async function main() {
  const [cmd, ...rest] = process.argv.slice(2);
  const o = parseArgs(rest);
  const ctx = mkctx(o);
  if (cmd === 'build') console.log(JSON.stringify(build(ctx)));
  else if (cmd === 'hashes') console.log(JSON.stringify(hashes(ctx), null, 1));
  else if (cmd === 'prompts') console.log(`${promptsText()}\n${extrasText()}`);
  else if (cmd === 'calprompts') console.log(`${calText()}\n${calTextL4d()}`);
  else if (cmd === 'wrapcost') {
    const r = wrapcost(ctx, Number(o.n ?? 400));
    fs.mkdirSync(ctx.out, { recursive: true });
    fs.writeFileSync(path.join(ctx.out, `wrapcost-${utc().replace(/\D/g, '').slice(0, 14)}.json`), `${JSON.stringify(r, null, 1)}\n`);
    console.log(JSON.stringify(r));
  } else if (cmd === 'ledger') {
    const r = ledger(ctx, path.resolve(o.file), ctx.cap);
    console.log(`ledger: ${r.calls} calls, lo $${r.lo.toFixed(4)}, hi $${r.hi.toFixed(4)}`);
  } else if (cmd === 'run') {
    const known = SESSIONS[o.session] ?? CAL_SESSIONS[o.session] ?? CAL_L4D_SESSIONS[o.session];
    if (!known) throw new Error(`unknown session ${o.session}; known: ${[...Object.keys(SESSIONS), ...Object.keys(CAL_SESSIONS), ...Object.keys(CAL_L4D_SESSIONS)].join(', ')}`);
    if (o.turns && !o.echo) throw new Error('--turns is for free echo runs only');
    const spec = o.turns ? { ...known, turns: known.turns.slice(0, Number(o.turns)) } : known;
    ctx.spent = spentSoFar(ctx.out);
    try {
      const { rec } = await runSession(ctx, spec, o);
      // Calibration prints only infra facts: outcome, turns, cost, minutes, hook-run and key counts. The numbers behind E1-E6 stay unread.
      console.log(`${rec.label}: ${rec.outcome}${rec.reason ? ` (${rec.reason.slice(0, 240)})` : ''} turns ${rec.turns.filter((t) => t.done).length}/${rec.typed_turns}${rec.filler_turns ? ` (${rec.filler_turns} filler)` : ''} of ${rec.spec_turns} scripted, minutes ${rec.minutes} cost hi $${rec.cost?.total_hi} (muse $${rec.cost?.muse.hi}, writer $${rec.cost?.writer.hi}) hooks ${JSON.stringify(rec.hook_runs)} key ${JSON.stringify(rec.key)} egress ${JSON.stringify(rec.egress)}`);
      process.exitCode = rec.outcome === 'DONE' ? 0 : 1;
    } finally { if (ctx.S) safeDelete(ctx.S.dir); }
  } else throw new Error('usage: run.mjs build|hashes|wrapcost|run|ledger [options]');
}

if (process.argv[1] && import.meta.url === new URL(`file://${process.argv[1]}`).href) main().catch((e) => { console.error(e.message); process.exitCode = 1; });
export { buildWorkspace, engineRecords, hashes, snapshot };

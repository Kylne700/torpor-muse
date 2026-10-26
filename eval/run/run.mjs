#!/usr/bin/env node
// The eval runner. Plays task x arm x rep episodes through real Muse Code turns, scores each with the held-out oracle and writes one
// JSON row per attempt. Paired by task, arm order drawn from the seed, resumable, spend-capped. See RUN.md.
//   META_API_KEY="$(cat <key file>)" node eval/run/run.mjs --tasks notes-ctrl --arms A,B,C,D --reps 1 --seed 7 --model <id> --cap-usd 5 --out DIR
//   node eval/run/run.mjs --echo ...        (free: Muse's echo provider, no key, no network)
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import crypto from 'node:crypto';
import { activeEps, rmEp, runEpisode } from './episode.mjs';
import * as L from './lib.mjs';
import { BACKEND, killEp, sandbox } from './sandbox.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '..', '..');
const EVAL = path.join(REPO, 'eval');
const usage = `usage: node eval/run/run.mjs --tasks all|id,id --arms A,B,C,D --reps N --seed S --out DIR [--echo | --model ID --cap-usd X]
  [--muse DIR] [--engine BIN] [--adapter BIN] [--writer-key-file FILE] [--writer-day-cap USD] [--effort E] [--max-steps 40]
  [--turn-timeout-s 420] [--episode-cap-s 900] [--concurrency 3] [--tmp DIR] [--run-id ID] [--keep-ep] [--dry-run]
  a keyed run reads the key from the META_API_KEY environment variable (never from a flag).`;

export function parseArgs(argv) {
  const a = { arms: 'A,B,C,D', reps: 1, seed: 1, effort: 'medium', maxSteps: 40, turnTimeoutS: 420, episodeCapS: 900, concurrency: 3, writerDayCap: 0.05 };
  const flags = { echo: 'echo', 'keep-ep': 'keepEp', 'dry-run': 'dryRun', 'allow-visible': 'allowVisible' };
  const names = { tasks: 'tasks', arms: 'arms', reps: 'reps', seed: 'seed', model: 'model', 'cap-usd': 'capUsd', out: 'out', muse: 'muse', engine: 'engine', adapter: 'adapter',
    'writer-key-file': 'writerKeyFile', 'writer-day-cap': 'writerDayCap', effort: 'effort', 'max-steps': 'maxSteps', 'turn-timeout-s': 'turnTimeoutS', 'episode-cap-s': 'episodeCapS',
    concurrency: 'concurrency', tmp: 'tmp', 'run-id': 'runId' };
  for (let i = 0; i < argv.length; i++) {
    const k = argv[i].replace(/^--/, '');
    if (flags[k]) a[flags[k]] = true;
    else if (names[k]) { if (i + 1 >= argv.length) throw new Error(`--${k} needs a value`); a[names[k]] = argv[++i]; } else throw new Error(`unknown flag ${argv[i]}`);
  }
  for (const k of ['reps', 'seed', 'maxSteps', 'turnTimeoutS', 'episodeCapS', 'concurrency']) a[k] = Number(a[k]);
  if (a.capUsd !== undefined) a.capUsd = Number(a.capUsd);
  a.writerDayCap = Number(a.writerDayCap);
  return a;
}

// the nearest folder above the repo that holds scratch/ (the project root); where defaults for the muse, engine and results folders live
function projectRoot() {
  for (let d = path.dirname(REPO); d !== path.dirname(d); d = path.dirname(d)) if (fs.existsSync(path.join(d, 'scratch'))) return d;
  return null;
}
const under = (p, base) => p === base || p.startsWith(base.endsWith('/') ? base : `${base}/`);

async function main() {
  const a = parseArgs(process.argv.slice(2));
  const root = projectRoot();
  const keyed = !a.echo;
  const key = process.env.META_API_KEY || '';
  delete process.env.META_API_KEY; // children get an explicit env; nothing inherits the key by accident
  if (keyed && !key) throw new Error('a keyed run needs META_API_KEY in the environment (or use --echo)');
  if (keyed && !a.model) throw new Error('a keyed run needs --model');
  if (keyed && !(a.capUsd > 0)) throw new Error('a keyed run needs --cap-usd');
  const taskIds = fs.readdirSync(path.join(EVAL, 'tasks')).filter((d) => fs.existsSync(path.join(EVAL, 'tasks', d, 'task.json'))).sort();
  const tasks = new Map(taskIds.map((id) => [id, JSON.parse(fs.readFileSync(path.join(EVAL, 'tasks', id, 'task.json'), 'utf8'))]));
  const want = !a.tasks || a.tasks === 'all' ? taskIds : a.tasks.split(',').filter(Boolean);
  for (const t of want) if (!tasks.has(t)) throw new Error(`unknown task ${t}`);
  const arms = a.arms.split(',').filter(Boolean);
  for (const x of arms) if (!L.ARMS.includes(x)) throw new Error(`unknown arm ${x}`);
  // a resumed folder keeps its run id; a folder holds one run id (the pilot and the main run are never pooled)
  let prevPlan = null;
  try { if (a.out) prevPlan = JSON.parse(fs.readFileSync(path.join(path.resolve(a.out), 'plan.json'), 'utf8')); } catch { /* a new folder */ }
  const runId = a.runId || prevPlan?.run_id || `run-${new Date().toISOString().replace(/[-:]/g, '').slice(0, 13)}z-s${a.seed}`;
  const jobs = L.planJobs({ tasks: want, arms, reps: a.reps, seed: a.seed });
  if (a.dryRun) { for (const j of jobs) console.log(JSON.stringify(j)); return; }

  const outDir = path.resolve(a.out || (root ? path.join(root, 'scratch', 'torpor-s4a', 'results', runId) : ''));
  if (!a.out && !root) throw new Error('--out is needed');
  const tmpBase = path.resolve(a.tmp || path.join(os.tmpdir(), 'eval-run'));
  // Everything outside /tmp and the home folder is visible (read-only) inside the model's sandbox.
  if (!a.allowVisible) {
    if (!under(tmpBase, '/tmp')) throw new Error(`--tmp must be under /tmp (it is hidden inside the sandbox there): ${tmpBase}`);
    for (const [n, p] of [['--out', outDir], ['eval folder', EVAL]]) if (!under(p, os.homedir()) && !under(p, '/tmp')) throw new Error(`${n} would be visible inside the model sandbox: ${p}`);
  }
  const museDir = path.resolve(a.muse || (root ? path.join(root, 'scratch', 'torpor-g0', 'muse-install') : ''));
  const engineBin = path.resolve(a.engine || process.env.TORPOR_BIN || (root ? path.join(root, 'scratch', 'torpor-engine', 'bin', 'torpor') : ''));
  const adapterBin = path.resolve(a.adapter || path.join(REPO, 'target', 'release', 'torpor-muse'));
  if (!fs.existsSync(path.join(museDir, 'muse'))) throw new Error('muse not found: pass --muse DIR (the folder that holds the muse binary)');
  if (arms.some((x) => x === 'C' || x === 'D') && !fs.existsSync(engineBin)) throw new Error('engine binary not found: pass --engine BIN');
  if (arms.includes('D') && !fs.existsSync(adapterBin)) throw new Error('adapter binary not found: build it (cargo build --release) or pass --adapter BIN');
  if (keyed && arms.includes('D') && a.writerKeyFile) {
    const m = fs.statSync(a.writerKeyFile).mode & 0o777;
    if (m !== 0o600) throw new Error('--writer-key-file must be mode 0600 (the writer refuses it otherwise)');
  }
  fs.mkdirSync(outDir, { recursive: true });
  fs.mkdirSync(tmpBase, { recursive: true });
  const rowsDir = path.join(outDir, 'rows');

  // the recorded plan; a resumed run must be the same plan
  const planFile = path.join(outDir, 'plan.json');
  const plan = { run_id: runId, seed: a.seed, tasks: want, arms, reps: a.reps, model: keyed ? a.model : 'echo', mode: keyed ? 'keyed' : 'echo', jobs };
  if (fs.existsSync(planFile)) {
    const old = JSON.parse(fs.readFileSync(planFile, 'utf8'));
    if (JSON.stringify(old.jobs) !== JSON.stringify(jobs) || old.model !== plan.model || old.run_id !== plan.run_id) throw new Error(`${planFile} holds a different plan or run id: use a new --out or the same flags`);
  } else L.writeAtomic(planFile, JSON.stringify(plan, null, 1) + '\n');

  // versions, from the real binaries, inside a sandbox
  const probeDir = path.join(tmpBase, `ep-${crypto.randomBytes(5).toString('hex')}`);
  const ver = async (bin, args, ro = [bin]) => ((await sandbox({ runDir: probeDir, ro, cmd: [bin, ...args], timeoutMs: 30000, pid: true })).stdout || '').trim().split('\n')[0];
  const museVersion = await ver(path.join(museDir, 'muse'), ['--version'], [museDir]); // Muse needs its whole install dir
  if (!museVersion) throw new Error(`"muse --version" printed nothing inside the sandbox (muse dir ${museDir})`);
  const engineVersion = arms.some((x) => x === 'C' || x === 'D') ? await ver(engineBin, ['--version']) : 'n/a';
  const adapterVersion = arms.includes('D') ? await ver(adapterBin, ['version']) : 'n/a';
  rmEp(probeDir, tmpBase);
  let adapterCommit = null;
  try { adapterCommit = execFileSync('git', ['-C', REPO, 'rev-parse', '--short', 'HEAD'], { encoding: 'utf8' }).trim(); } catch { /* not a git checkout */ }
  const adapterSha = arms.includes('D') ? L.sha(fs.readFileSync(adapterBin)) : null;
  // C and D run the engine that engine.lock pins; every row records the binary's hash and whether it is the locked linux-x64 asset
  const needEngine = arms.some((x) => x === 'C' || x === 'D');
  const engineSha = needEngine ? L.sha(fs.readFileSync(engineBin)) : null;
  let engineLockMatch = null;
  if (needEngine) try { engineLockMatch = fs.readFileSync(path.join(REPO, 'engine.lock'), 'utf8').split('\n').some((l) => l.trim().split(/\s+/).slice(0, 3).join(' ') === `asset linux-x64 ${engineSha}`); } catch { /* no engine.lock in this checkout */ }
  if (engineLockMatch === false) console.error('warning: the engine binary is not the linux-x64 asset that engine.lock pins (rows say engine_lock_match false)');

  const spend = new L.Spend(keyed ? a.capUsd : Infinity, path.join(outDir, 'spend.jsonl'));
  const cx = {
    runId, seed: a.seed, keyed, key, model: a.model, effort: a.effort, maxSteps: a.maxSteps, turnTimeoutMs: a.turnTimeoutS * 1000, epCapMs: a.episodeCapS * 1000,
    evalDir: EVAL, repoDir: REPO, museDir, museVersion, engineBin, engineVersion, engineSha, engineLockMatch, adapterBin, adapterSha, adapterVersion, adapterCommit, tmpBase, outDir, rowsDir, tasks,
    spend, backoff: new L.Backoff(), heldIdx: L.heldOutIndex(EVAL), writerKeyFile: keyed && a.writerKeyFile ? path.resolve(a.writerKeyFile) : null, writerDayCapUsd: a.writerDayCap, keepEp: !!a.keepEp,
  };

  let conc = Math.max(1, a.concurrency | 0);
  if (keyed && BACKEND === 'v2.1' && conc > 1) { console.error(`note: keyed runs share the host network under sandbox v2.1; concurrency forced from ${conc} to 1`); conc = 1; }
  const states = L.episodeStates(L.readRows(rowsDir));
  const pending = jobs.filter((j) => !states.get(L.epKey(j))?.done);
  console.error(`${runId}: ${jobs.length} episodes planned, ${jobs.length - pending.length} already finished, ${pending.length} to run, concurrency ${conc}, ${keyed ? `keyed ${a.model}, cap $${a.capUsd}` : 'echo'}, sandbox ${BACKEND}`);

  // One group at a time. A (task, rep) group is played whole: its arms run together (up to --concurrency at once), and the next group only
  // starts when the cap leaves room for all of it. A stop (cap, fatal error, first SIGINT or SIGTERM) starts no new group and lets the group
  // in progress finish; only the second signal kills what runs, which can leave one half group (the same command finishes it).
  const groups = [];
  for (const j of pending) {
    const k = L.groupKey(j);
    (groups.find((g) => g.key === k) || groups[groups.push({ key: k, jobs: [] }) - 1]).jobs.push(j);
  }
  for (const r of L.readRows(rowsDir)) spend.episodeDone(r.usd_hi + r.writer_usd_hi);
  let halted = false, draining = false, done = 0, signals = 0;
  const onSignal = () => {
    if (++signals === 1) { draining = true; console.error('stop requested: the group in progress is finished first; send the signal again to kill now'); return; }
    halted = true;
    for (const ep of activeEps) killEp(ep);
    setTimeout(() => process.exit(130), 500);
  };
  process.on('SIGINT', onSignal);
  process.on('SIGTERM', onSignal);
  const play = async (job) => {
    for (;;) {
      const st = L.episodeStates(L.readRows(rowsDir)).get(L.epKey(job)) || { last: 0, infra: 0, done: false };
      if (st.done || halted) return;
      const attempt = st.last + 1;
      const { row, stopRun } = await runEpisode(job, attempt, cx);
      if (stopRun) halted = true;
      if (stopRun && row.turns_run === 0) { console.error(`stop: ${row.invalid_reason}`); return; } // nothing ran: no row
      L.writeRow(rowsDir, row);
      spend.episodeDone(row.usd_hi + row.writer_usd_hi);
      console.error(`[${++done}/${pending.length}] ${row.task} arm ${row.arm} r${row.rep} a${row.attempt}: ${row.valid ? `valid pass=${row.final_pass} later=${row.later_kept}` : `INVALID ${row.invalid_reason}`} | ${row.minutes} min, ${row.steps_total} steps, $${row.usd_hi.toFixed(4)}+$${row.writer_usd_hi.toFixed(4)} | total $${spend.total.toFixed(4)}`);
      if (row.valid || row.retryable === false || stopRun) return;
    }
  };
  for (const g of groups) {
    if (halted || draining) break;
    if (!spend.groupFits(g.jobs.length)) { halted = true; console.error(`stop: group ${g.key} (${g.jobs.length} episodes) might not fit under the cap: $${spend.total.toFixed(4)} spent of $${a.capUsd}`); break; }
    let i = 0;
    await Promise.all(Array.from({ length: Math.min(conc, g.jobs.length) }, async () => { for (let job; (job = g.jobs[i++]); ) await play(job); }));
  }
  const rows = L.readRows(rowsDir);
  const validBy = new Map(); // group -> arms with a valid row
  for (const r of rows) if (r.valid) validBy.set(L.groupKey(r), (validBy.get(L.groupKey(r)) || new Set()).add(r.arm));
  const groupsPlanned = new Set(jobs.map(L.groupKey)).size;
  const groupsComplete = [...validBy.values()].filter((arms2) => arms.every((x) => arms2.has(x))).length;
  const stopped = halted || draining;
  // no key here may look like a row field (valid, arm, task ...): the analysis reads every json file of a folder
  const sum = { run_id: runId, rows: rows.length, valid_rows: rows.filter((r) => r.valid).length, invalid_rows: rows.filter((r) => !r.valid).length, groups_planned: groupsPlanned, groups_complete: groupsComplete, usd_hi: L.round6(rows.reduce((t, r) => t + r.usd_hi + r.writer_usd_hi, 0)), spend_total_usd_hi: L.round6(spend.total), stopped_early: stopped };
  L.writeAtomic(path.join(outDir, 'summary.json'), JSON.stringify(sum, null, 1) + '\n');
  console.log(JSON.stringify(sum));
  process.exit(stopped ? 3 : 0);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main().catch((e) => { console.error(`error: ${e.message}\n${usage}`); process.exit(2); });

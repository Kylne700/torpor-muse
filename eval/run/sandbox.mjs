// One function that runs a command inside the Muse sandbox: scripts/sandbox/muse-sandbox.sh (v3).
//   --run-dir DIR  --ro PATH...  --net none|meta  --allow HOST:PORT...  --chdir DIR2  --drain SECS  -- CMD...
// The launcher clears the environment (only TORPOR_* and, with --net meta, META_API_KEY pass in), puts HOME, TMPDIR and XDG_* inside
// the run folder, gives no network under --net none, and under --net meta only api.meta.ai:443 through a host proxy that logs every
// connection to <run-dir>/net/proxy.log. Under --net meta it also waits up to --drain seconds for leftover processes (the detached
// writer of arm D); under --net none everything dies with the command. Secrets reach the child through the spawned process's own
// env, never through argv.
import crypto from 'node:crypto';
import { execFileSync, spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const SCRIPT = path.resolve(HERE, '..', '..', 'scripts', 'sandbox', 'muse-sandbox.sh');
const KEEP = 64 * 1024;
const GRACE_MS = 300; // after the launcher exits: let the pipes drain, then give up on them

// What the row's manifest says about the sandbox: the launcher's two files by hash, and the commit that last touched them.
let info = null;
export function sandboxInfo() {
  if (info) return info;
  if (!fs.existsSync(SCRIPT)) throw new Error('scripts/sandbox/muse-sandbox.sh is missing: merge origin/main');
  const dir = path.dirname(SCRIPT);
  const sha = (f) => crypto.createHash('sha256').update(fs.readFileSync(f)).digest('hex');
  const git = (...a) => { try { return execFileSync('git', ['-C', dir, ...a], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim(); } catch { return null; } };
  const files = ['muse-sandbox.sh', 'netproxy.mjs'];
  const status = git('status', '--porcelain', '--', ...files);
  info = {
    backend: 'v3', script: 'scripts/sandbox/muse-sandbox.sh', commit: git('log', '-n1', '--format=%h', '--', ...files) || null, dirty: status === null ? null : status !== '',
    script_sha256: sha(SCRIPT), netproxy_sha256: sha(path.join(dir, 'netproxy.mjs')),
    bwrap: spawnSync('bwrap', ['--version'], { encoding: 'utf8' }).stdout?.trim() || null, net_keyed: 'api.meta.ai:443 only, through the logging proxy',
  };
  return info;
}

// Only TORPOR_* and (with --net meta) META_API_KEY may be passed in; anything else is a bug in the caller.
function passEnv(env, net) {
  const out = {};
  for (const [k, v] of Object.entries(env || {})) {
    if (k === 'META_API_KEY') { if (net === 'meta' && v) out[k] = v; else if (v) throw new Error('META_API_KEY needs net=meta'); }
    else if (k.startsWith('TORPOR_')) out[k] = v;
    else throw new Error(`env ${k} is not passed into the sandbox`);
  }
  return out;
}

// Processes of this episode: cwd inside runDir, or HOME=<runDir>/home in their env. Used to find and kill leftovers (ours only).
export function epProcs(runDir) {
  const home = `HOME=${path.join(runDir, 'home')}\0`;
  const out = [];
  for (const d of fs.readdirSync('/proc')) {
    if (!/^\d+$/.test(d) || Number(d) === process.pid) continue;
    let hit = false;
    try { const c = fs.readlinkSync(`/proc/${d}/cwd`); hit = c === runDir || c.startsWith(runDir + '/'); } catch { /* gone or not ours */ }
    if (!hit) { try { hit = fs.readFileSync(`/proc/${d}/environ`, 'latin1').includes(home); } catch { /* same */ } }
    if (!hit) continue;
    let cmd = '';
    try { cmd = fs.readFileSync(`/proc/${d}/cmdline`, 'latin1').replace(/\0/g, ' ').trim().slice(0, 300); } catch { /* gone */ }
    out.push({ pid: Number(d), cmd });
  }
  return out;
}

export function killEp(runDir, keep = () => false, signal = 'SIGKILL') {
  const killed = [];
  for (const p of epProcs(runDir)) {
    if (keep(p)) continue;
    try { process.kill(p.pid, signal); killed.push(p.pid); } catch { /* already gone */ }
  }
  return killed;
}

// Is a process alive whose argv (an array) satisfies the predicate? The launcher's own processes carry the whole command line as later
// arguments, so a test on argv[0] picks out the command itself.
const argvAlive = (pred) => {
  for (const d of fs.readdirSync('/proc')) {
    if (!/^\d+$/.test(d)) continue;
    try { if (pred(fs.readFileSync(`/proc/${d}/cmdline`, 'latin1').split('\0').filter(Boolean))) return true; } catch { /* gone */ }
  }
  return false;
};

// The proxy's log of a run folder (every keyed sandbox call of the episode appends to <runDir>/net/proxy.log):
// `<UTC time> host:port allowed|denied up=N down=N via=fwd|proxy [err=CODE]`. Muse's own traffic is via=proxy; hooks and the detached writer
// reach the API through the forwarder (via=fwd). `other` lists (at most 8, distinct) everything that is not an allowed api.meta.ai:443.
// The runner records this and does not stop on it: reading `other` is the operator's job.
export function egressRead(runDir) {
  const out = { allowed: 0, denied: 0, via_fwd: 0, other: [] };
  let text = '';
  try { text = fs.readFileSync(path.join(runDir, 'net', 'proxy.log'), 'utf8'); } catch { return out; }
  for (const l of text.split('\n').filter(Boolean)) {
    const m = /^\S+ (\S+) (allowed|denied)\b.*\bvia=(fwd|proxy)\b/.exec(l);
    if (m) { out[m[2]]++; if (m[3] === 'fwd') out.via_fwd++; }
    const what = m ? `${m[1]} ${m[2]}` : `unparsed: ${l.slice(0, 60)}`;
    if (what !== 'api.meta.ai:443 allowed' && !out.other.includes(what) && out.other.length < 8) out.other.push(what);
  }
  return out;
}

// drain: seconds the launcher waits for leftover processes after the command exits (--net meta only; 0 = none). watchExit: a test on the
// argv of the command's own process; when given, the result's drainMs is how long the call lasted after that process was gone.
export function sandbox({ runDir, ro = [], net = 'none', allow = [], chdir, env = {}, cmd, timeoutMs = 0, input, drain = 0, watchExit }) {
  if (!path.isAbsolute(runDir)) throw new Error('runDir must be absolute');
  for (const d of ['home', 'tmp', 'muse-share']) fs.mkdirSync(path.join(runDir, d), { recursive: true });
  const passed = passEnv(env, net);
  const args = ['--run-dir', runDir, ...ro.flatMap((p) => ['--ro', p]), '--net', net, ...allow.flatMap((h) => ['--allow', h]),
    ...(net === 'meta' ? ['--drain', String(Math.max(0, Math.ceil(drain)))] : []), ...(chdir ? ['--chdir', chdir] : []), '--', ...cmd];
  return new Promise((resolve) => {
    const t0 = Date.now();
    const child = spawn(SCRIPT, args, { env: { PATH: '/usr/local/bin:/usr/bin:/bin', ...passed }, stdio: ['pipe', 'pipe', 'pipe'], cwd: os.tmpdir() });
    let out = '', err = '', timedOut = false, done = false, killTimer = null, seen = false, goneAt = null;
    child.stdout.on('data', (b) => { out = (out + b).slice(-KEEP); });
    child.stderr.on('data', (b) => { err = (err + b).slice(-KEEP); });
    child.stdin.on('error', () => {});
    child.stdin.end(input ?? '');
    const timer = timeoutMs > 0 ? setTimeout(() => {
      timedOut = true;
      try { child.kill('SIGTERM'); } catch { /* gone */ }
      killTimer = setTimeout(() => { try { child.kill('SIGKILL'); } catch { /* gone */ } }, 5000);
    }, timeoutMs) : null;
    const watch = watchExit ? setInterval(() => {
      if (argvAlive(watchExit)) seen = true;
      else if (seen && goneAt === null) goneAt = Date.now() - t0;
    }, 250) : null;
    // A leftover process can hold the pipes open after the launcher is gone: resolve on 'exit' (plus a short grace), not on 'close'.
    const finish = (code, signal) => {
      if (done) return;
      done = true;
      clearTimeout(timer); clearTimeout(killTimer); clearInterval(watch);
      try { child.stdout.destroy(); child.stderr.destroy(); } catch { /* closed */ }
      const killed = timedOut ? killEp(runDir) : [];
      const ms = Date.now() - t0;
      const stopped = /(\d+) process\(es\) still running after --drain/.exec(err);
      resolve({ code, signal, stdout: out, stderr: err, timedOut, ms, pid: child.pid, reaped: killed.length, drainMs: goneAt === null ? 0 : Math.max(0, ms - goneAt), drainStopped: stopped ? Number(stopped[1]) : 0 });
    };
    child.on('error', (e) => { err += String(e); finish(127, null); });
    child.on('exit', (code, signal) => setTimeout(() => finish(code, signal), GRACE_MS));
    child.on('close', (code, signal) => finish(code, signal));
  });
}

// One function that runs a command inside the Muse sandbox. CLI-compatible with scripts/sandbox/muse-sandbox.sh (v3):
//   --run-dir DIR  --ro PATH...  --net none|meta  --allow HOST:PORT...  --chdir DIR2  -- CMD...
// Until that script is on main, the v2.1 bwrap flags of the confinement rule are used. Backend choice is ONE line (BACKEND).
// Secrets reach the child through the spawned process's own env, never through argv.
import { spawn, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
export const SCRIPT = path.resolve(HERE, '..', '..', 'scripts', 'sandbox', 'muse-sandbox.sh');
// auto = the v3 script when it exists, else v2.1. Force one with EVAL_SANDBOX=v2.1 or EVAL_SANDBOX=v3.
export const BACKEND = process.env.EVAL_SANDBOX || (fs.existsSync(SCRIPT) ? 'v3' : 'v2.1');
const REAL = os.homedir();
const KEEP = 64 * 1024;
const DRAIN_MS = 300;

export function sandboxInfo() {
  if (BACKEND === 'v3') return { backend: 'v3', script: 'scripts/sandbox/muse-sandbox.sh', net_keyed: 'allowlist proxy' };
  const v = spawnSync('bwrap', ['--version'], { encoding: 'utf8' }).stdout?.trim() || 'bwrap ?';
  return { backend: 'v2.1', bwrap: v, net_keyed: 'shared (v2.1 gap: the host network is open in keyed runs)' };
}

// The env every sandboxed command gets (the same set muse-sandbox.sh builds).
function baseEnv(runDir) {
  const home = path.join(runDir, 'home');
  return {
    HOME: home, TMPDIR: path.join(runDir, 'tmp'), SHELL: '/bin/bash', LANG: 'C.UTF-8',
    PATH: '/usr/local/bin:/usr/bin:/bin', MUSE_NO_AUTO_UPDATE: '1',
    XDG_CONFIG_HOME: path.join(home, '.config'), XDG_DATA_HOME: path.join(home, '.local', 'share'),
    XDG_STATE_HOME: path.join(home, '.local', 'state'), XDG_CACHE_HOME: path.join(home, '.cache'),
  };
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

// pid: a private pid namespace, so nothing the command started outlives it (oracle, patch and init runs; NOT the model's own
// runs, where a detached writer must be able to outlive Muse). v2.1 only.
export function bwrapArgs({ runDir, ro = [], net = 'none', chdir, cmd, pid = false }) {
  const a = ['--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/tmp', '--tmpfs', REAL, '--tmpfs', '/run/user'];
  for (const p of ro) a.push('--ro-bind', p, p);
  a.push('--bind', runDir, runDir, '--bind', path.join(runDir, 'muse-share'), path.join(REAL, '.local', 'share', 'muse'));
  a.push('--new-session', '--die-with-parent');
  if (net === 'none') a.push('--unshare-net');
  if (pid) a.push('--unshare-pid');
  if (chdir) a.push('--chdir', chdir);
  return [...a, '--', ...cmd];
}

// Processes of this episode: cwd inside runDir, or HOME=<runDir>/home in their env. Used to wait for / kill leftovers (ours only).
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

export function sandbox({ runDir, ro = [], net = 'none', allow = [], chdir, env = {}, cmd, timeoutMs = 0, input, pid = false }) {
  if (!path.isAbsolute(runDir)) throw new Error('runDir must be absolute');
  for (const d of ['home', 'tmp', 'muse-share']) fs.mkdirSync(path.join(runDir, d), { recursive: true });
  const passed = passEnv(env, net);
  let file, args, childEnv;
  if (BACKEND === 'v3') {
    file = SCRIPT;
    args = ['--run-dir', runDir, ...ro.flatMap((p) => ['--ro', p]), '--net', net, ...allow.flatMap((h) => ['--allow', h]),
      ...(chdir ? ['--chdir', chdir] : []), '--', ...cmd];
    childEnv = { PATH: '/usr/local/bin:/usr/bin:/bin', ...passed };
  } else {
    file = 'bwrap';
    args = bwrapArgs({ runDir, ro, net, chdir, cmd, pid });
    childEnv = { ...baseEnv(runDir), ...passed };
  }
  return new Promise((resolve) => {
    const t0 = Date.now();
    const child = spawn(file, args, { env: childEnv, stdio: ['pipe', 'pipe', 'pipe'], cwd: os.tmpdir() });
    let out = '', err = '', timedOut = false, done = false, killTimer = null;
    child.stdout.on('data', (b) => { out = (out + b).slice(-KEEP); });
    child.stderr.on('data', (b) => { err = (err + b).slice(-KEEP); });
    child.stdin.on('error', () => {});
    child.stdin.end(input ?? '');
    const timer = timeoutMs > 0 ? setTimeout(() => {
      timedOut = true;
      try { child.kill('SIGTERM'); } catch { /* gone */ }
      killTimer = setTimeout(() => { try { child.kill('SIGKILL'); } catch { /* gone */ } }, 5000);
    }, timeoutMs) : null;
    // A leftover process can hold the pipes open after the command is gone: resolve on 'exit' (plus a short drain), not on 'close'.
    const finish = (code, signal) => {
      if (done) return;
      done = true;
      clearTimeout(timer); clearTimeout(killTimer);
      try { child.stdout.destroy(); child.stderr.destroy(); } catch { /* closed */ }
      const killed = timedOut ? killEp(runDir) : [];
      resolve({ code, signal, stdout: out, stderr: err, timedOut, ms: Date.now() - t0, pid: child.pid, reaped: killed.length });
    };
    child.on('error', (e) => { err += String(e); finish(127, null); });
    child.on('exit', (code, signal) => setTimeout(() => finish(code, signal), DRAIN_MS));
    child.on('close', (code, signal) => finish(code, signal));
  });
}

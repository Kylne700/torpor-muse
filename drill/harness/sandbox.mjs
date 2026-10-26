// The only place that starts a process for a drill run, and the guards around run folders and file paths.
// Everything a scripted run executes (builders, bash calls, hooks, typed commands, recovery, held-out checks) goes
// through exec(): under bubblewrap (the whole disk read-only, only the run folder T writable, no network, the real
// home and /run hidden, own process and IPC namespaces) unless the caller asks for --no-sandbox, which is for CI only.
// Zero dependencies.
import cp from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

export class GuardError extends Error {} // a deterministic refusal: the run is aborted, never retried
export class SandboxUnavailable extends GuardError {}
export class InfraError extends Error {} // not the arm's fault: the run may be retried in a fresh folder

const CAP = 1 << 20; // bytes of output kept per stream

// ------------------------------------------------------------------ run folders

const made = new Set(); // run folders this process created: the only ones it will ever delete

export function makeRunFolder(base = '/tmp') {
  const b = fs.realpathSync(base);
  const T = fs.realpathSync(fs.mkdtempSync(path.join(b, 'drill.'))); // mode 700
  made.add(T);
  fs.writeFileSync(path.join(T, '.drill-canary'), `${T}\n`);
  return T;
}

const RUN_NAME = /^drill\.[A-Za-z0-9]{6}$/;

// Deletes a run folder this process made, and nothing else.
export function safeDelete(T) {
  const real = fs.realpathSync(T);
  const st = fs.lstatSync(real);
  if (!made.has(real) || !RUN_NAME.test(path.basename(real)) || !st.isDirectory() || st.isSymbolicLink() || st.uid !== process.getuid()) {
    throw new GuardError(`refusing to delete ${T}: not a run folder made by this harness`);
  }
  try {
    fs.rmSync(real, { recursive: true, force: true });
  } catch {
    cp.spawnSync('chmod', ['-R', 'u+rwX', real]); // a scripted chmod may have made parts unreadable
    fs.rmSync(real, { recursive: true, force: true });
  }
  made.delete(real);
}

// ------------------------------------------------------------------ paths

// The real path of `p` (relative to `cwd`), following symlinks, even when the leaf does not exist yet.
function deepReal(abs) {
  const rest = [];
  let cur = abs;
  for (;;) {
    let st = null;
    try {
      st = fs.lstatSync(cur);
    } catch {
      /* does not exist */
    }
    if (st) {
      let real;
      try {
        real = fs.realpathSync(cur);
      } catch {
        throw new GuardError(`dangling symlink on the way to ${abs}`);
      }
      return path.join(real, ...rest);
    }
    const up = path.dirname(cur);
    if (up === cur) return abs;
    rest.unshift(path.basename(cur));
    cur = up;
  }
}

// A scripted edit or write may only touch a path inside the run folder (symlinks followed).
export function resolveInside(T, p, cwd) {
  const real = deepReal(path.resolve(cwd, p));
  if (real !== T && !real.startsWith(T + path.sep)) throw new GuardError(`path ${p} resolves outside the run folder: ${real}`);
  return real;
}

// A harness-side read of a drill asset must stay inside the allowed asset folders.
export function resolveUnder(roots, p) {
  const real = deepReal(path.resolve(p));
  if (!roots.some((r) => real === r || real.startsWith(r + path.sep))) throw new GuardError(`asset ${p} is outside the asset folders`);
  return real;
}

// ------------------------------------------------------------------ sandbox

// --unshare-net: no network, and no abstract sockets. --unshare-pid: /proc shows only the sandbox, so other processes'
// environments (keys) cannot be read. --unshare-ipc: no SysV shared memory or queues of the host.
const NAMESPACES = ['--unshare-net', '--unshare-pid', '--unshare-ipc'];

export function findBwrap(env = process.env) {
  const bin = env.DRILL_BWRAP || 'bwrap';
  const r = cp.spawnSync(bin, ['--ro-bind', '/', '/', ...NAMESPACES, '--die-with-parent', 'true'], { encoding: 'utf8', timeout: 15000 });
  if (r.error || r.status !== 0) {
    throw new SandboxUnavailable(`bwrap is not usable (${bin}): ${r.error?.message ?? r.stderr.trim()}; refusing to run (--no-sandbox is for CI only)`);
  }
  return bin;
}

let ioRoot = null;
function ioDir() {
  if (!ioRoot) {
    ioRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'drillio.'));
    process.on('exit', () => {
      try {
        if (/^drillio\.[A-Za-z0-9]{6}$/.test(path.basename(ioRoot))) fs.rmSync(ioRoot, { recursive: true, force: true });
      } catch {
        /* a temp folder that stays is harmless */
      }
    });
  }
  return ioRoot;
}

const realExec = fs.realpathSync(process.execPath);
export const NODE_EXEC = realExec;
export const NODE_BIN = path.dirname(realExec);
const NODE_ROOT = path.dirname(NODE_BIN);
// The real home (HOME and the passwd entry): emptied inside the sandbox, only the paths bound back show up in it.
const HOMES = [...new Set([os.homedir(), os.userInfo().homedir])].filter((h) => path.isAbsolute(h) && h !== '/' && fs.existsSync(h));
const hidden = (p) => [...['home', 'tmp', 'root'].map((d) => `/${d}/`), ...HOMES.map((h) => `${h}/`)].some((x) => p.startsWith(x));

// The environment classes. Tool, typed, between and recovery commands share one; hooks get a Muse-like cleared one.
export function toolEnv(T, extra = {}) {
  return {
    HOME: `${T}/home`, PATH: `${T}/bin:${NODE_BIN}:/usr/local/bin:/usr/bin:/bin`, TMPDIR: `${T}/tmp`, TZ: 'UTC',
    LANG: 'C.UTF-8', TERM: 'dumb', SHELL: '/bin/sh', TORPOR_HOME: `${T}/torpor-home`,
    TORPOR_KEY_FILE: path.join(T, 'home', '.config', 'torpor', 'key'), GIT_TERMINAL_PROMPT: '0', ...extra,
  };
}
export function hookEnv(T, ws, session, extra = {}) {
  return {
    HOME: `${T}/home`, PATH: `/usr/local/bin:/usr/bin:/bin:${NODE_BIN}`, USER: 'drill', LANG: 'C.UTF-8', PWD: ws, SHELL: '/bin/sh',
    TERM: 'dumb', TMPDIR: `${T}/tmp`, MUSE_SESSION_ID: session.sid, MUSE_THREAD_ID: session.thread, ...extra,
  };
}

// makeSandbox({ T, bwrap }) -> { exec, reapRun, bwrap, args }. bwrap = null means --no-sandbox (explicit, CI only).
// exec({ argv, cwd, env, ro, stdin, merge, timeoutMs }) -> { exit, signal, timedOut, stdout, stderr, stdoutBytes,
//   stderrBytes, started, ended, ms }
export function makeSandbox({ T, bwrap }) {
  let seq = 0;
  const museShare = path.join(T, 'muse-share'); // made by the run setup after the builder (the builder wants T empty)
  // Muse writes session-names.db under the REAL home even when HOME is changed; the bind keeps that inside T.
  // Replay never starts Muse and the real home is hidden anyway, so this is a documented no-op for replay.
  const realMuse = path.join(os.homedir(), '.local/share/muse');

  const argsFor = (o, env) => {
    const a = ['--ro-bind', '/', '/', '--dev', '/dev', '--proc', '/proc', '--tmpfs', '/home', '--tmpfs', '/tmp'];
    for (const h of HOMES) a.push('--tmpfs', h);
    // Unix sockets in /run stay reachable without a network: rootless docker.sock, the ssh and gpg agents and the D-Bus
    // session bus in /run/user/<uid>, docker.sock, libvirt and the system bus in /run. Any of them is a way out.
    if (fs.existsSync('/run')) a.push('--tmpfs', '/run', '--dir', '/run/user');
    if (hidden(realExec)) a.push('--ro-bind', NODE_ROOT, NODE_ROOT);
    a.push('--bind', T, T);
    for (const p of o.ro ?? []) a.push('--ro-bind', p, p);
    if (fs.existsSync(museShare)) a.push('--bind', museShare, realMuse);
    a.push(...NAMESPACES, '--die-with-parent', '--new-session', '--chdir', o.cwd, '--clearenv');
    for (const [k, v] of Object.entries(env)) a.push('--setenv', k, String(v));
    return [...a, '--', ...o.argv];
  };

  function exec(o) {
    const id = ++seq;
    const token = `${path.basename(T)}-${id}`;
    const env = { ...o.env, DRILL_RUN: T, DRILL_CALL: token };
    const base = path.join(ioDir(), `${process.pid}-${Date.now()}-${id}`);
    const files = { in: `${base}.in`, out: `${base}.out`, err: `${base}.err` };
    fs.writeFileSync(files.in, o.stdin ?? '');
    const fdIn = fs.openSync(files.in, 'r');
    const fdOut = fs.openSync(files.out, 'w');
    const fdErr = o.merge ? fdOut : fs.openSync(files.err, 'w');
    const t0 = Date.now();
    let r;
    try {
      r = bwrap
        ? cp.spawnSync(bwrap, argsFor(o, env), { stdio: [fdIn, fdOut, fdErr], timeout: o.timeoutMs ?? 120000, killSignal: 'SIGKILL', env: { PATH: '/usr/local/bin:/usr/bin:/bin' } })
        : cp.spawnSync(o.argv[0], o.argv.slice(1), { cwd: o.cwd, stdio: [fdIn, fdOut, fdErr], timeout: o.timeoutMs ?? 120000, killSignal: 'SIGKILL', env });
    } finally {
      fs.closeSync(fdIn);
      fs.closeSync(fdOut);
      if (!o.merge) fs.closeSync(fdErr);
    }
    const read = (f) => {
      const size = fs.statSync(f).size;
      const fd = fs.openSync(f, 'r');
      const buf = Buffer.alloc(Math.min(size, CAP));
      fs.readSync(fd, buf, 0, buf.length, 0);
      fs.closeSync(fd);
      return { text: buf.toString('utf8'), size };
    };
    const out = read(files.out);
    const err = o.merge ? { text: '', size: 0 } : read(files.err);
    for (const f of Object.values(files)) fs.rmSync(f, { force: true });
    const timedOut = r.error?.code === 'ETIMEDOUT';
    if (r.error && !timedOut) throw new InfraError(`cannot start ${bwrap ? 'bwrap' : o.argv[0]}: ${r.error.message}`);
    if (timedOut) reap(`DRILL_CALL=${token}`);
    const exit = r.status ?? (r.signal === 'SIGKILL' ? 137 : -1);
    if (bwrap && exit !== 0 && /^bwrap: /m.test(err.text || out.text) && !timedOut) {
      throw new InfraError(`sandbox did not start: ${(err.text || out.text).split('\n')[0]}`);
    }
    return {
      exit, signal: r.signal, timedOut, stdout: out.text, stderr: err.text, stdoutBytes: out.size, stderrBytes: err.size,
      started: new Date(t0).toISOString(), ended: new Date().toISOString(), ms: Date.now() - t0,
    };
  }
  return { exec, reapRun: () => reap(`DRILL_RUN=${T}`), bwrap, args: (o, env = {}) => argsFor(o, env) };
}

// Kills every process whose environment carries `marker` (exactly NAME=VALUE): leftovers of a call or a run.
export function reap(marker) {
  let n = 0;
  for (const pid of fs.readdirSync('/proc')) {
    if (!/^\d+$/.test(pid) || Number(pid) === process.pid) continue;
    try {
      if (fs.readFileSync(`/proc/${pid}/environ`, 'latin1').split('\0').includes(marker)) {
        process.kill(Number(pid), 'SIGKILL');
        n++;
      }
    } catch {
      /* gone, or not ours */
    }
  }
  return n;
}

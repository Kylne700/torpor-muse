// L4d added measures (research/71-plan.md, the L4d note of 2026-10-29 07:04 UTC). They have no pass line and change nothing Muse or the model sees:
// run.mjs calls the capture helpers while a session runs, analyze.mjs the readers afterwards. Everything here only reads: /proc/<pid>/cwd and
// /proc/<pid>/stat, /proc/locks, a file's inode. Never environ, and never a lock of ours: a lock taken on the check-writer's files could make its one
// non-blocking try fail and so change what the fix under test does (src/suggest/state.rs:116-119).
import fs from 'node:fs';
import path from 'node:path';

const inside = (dir, p) => p === dir || p.startsWith(`${dir}/`);

// ------------------------------------------------------------------ (ii) processes whose working folder is inside the run folder

// comm, state, parent and start time (clock ticks since boot, so a reused pid is told apart) of one process, from its stat line; null when it is gone.
export function readStat(pid, proc = '/proc') {
  let s;
  try { s = fs.readFileSync(`${proc}/${pid}/stat`, 'latin1'); } catch { return null; }
  const a = s.indexOf('('), b = s.lastIndexOf(')'); // comm may hold spaces and brackets: the last bracket ends it
  if (a < 0 || b < a) return null;
  const f = s.slice(b + 2).split(' ');
  return { comm: s.slice(a + 1, b), state: f[0], ppid: Number(f[1]), start: Number(f[19]) };
}

export function procsInside(dir, { proc = '/proc', self = process.pid } = {}) {
  const out = [];
  for (const d of fs.readdirSync(proc)) {
    if (!/^\d+$/.test(d) || Number(d) === self) continue;
    let cwd;
    try { cwd = fs.readlinkSync(`${proc}/${d}/cwd`); } catch { continue; } // gone, or not ours
    if (!inside(dir, cwd)) continue;
    const s = readStat(d, proc);
    if (!s) continue;
    out.push({ pid: Number(d), ppid: s.ppid, pcomm: readStat(s.ppid, proc)?.comm ?? null, comm: s.comm, state: s.state, start: s.start, cwd: cwd === dir ? '.' : cwd.slice(dir.length + 1) });
  }
  return out.sort((a, b) => a.pid - b.pid);
}

// The processes of `now` that the earlier scan `before` did not have (same pid with another start time is a new process).
export const newSince = (before, now) => now.filter((p) => !before.some((b) => b.pid === p.pid && b.start === p.start));

// Kills by exact pid what a scan listed, after reading the same three facts again just before (pid, start time, folder). -> the pids it signalled.
export function killListed(dir, list, { proc = '/proc', kill = (pid, sig) => process.kill(pid, sig) } = {}) {
  const sent = [];
  for (const p of list) {
    const s = readStat(p.pid, proc);
    let cwd = null;
    try { cwd = fs.readlinkSync(`${proc}/${p.pid}/cwd`); } catch { /* gone */ }
    if (!s || s.start !== p.start || !cwd || !inside(dir, cwd)) continue;
    try { kill(p.pid, 'SIGKILL'); sent.push(p.pid); } catch { /* already gone */ }
  }
  return sent;
}

// ------------------------------------------------------------------ the check-writer's two locks, seen from outside

// A held flock shows in /proc/locks as `1: FLOCK  ADVISORY  WRITE 4242 00:2b:15681564 0 EOF` (major and minor in hex, the inode in decimal).
export const parseLocks = (text) => text.split('\n').flatMap((line) => {
  const t = line.trim().split(/\s+/);
  const m = t[1] === 'FLOCK' && t[3] === 'WRITE' ? /^([0-9a-f]+):([0-9a-f]+):(\d+)$/i.exec(t[5] ?? '') : null;
  return m ? [{ pid: Number(t[4]), major: parseInt(m[1], 16), minor: parseInt(m[2], 16), ino: m[3] }] : [];
});

// Device (major, minor as Linux splits st_dev) and inode of a file, in the form /proc/locks uses.
export function fileId(file) {
  const st = fs.statSync(file, { bigint: true });
  const d = st.dev;
  return { major: Number(((d >> 8n) & 0xfffn) | ((d >> 32n) & 0xfffff000n)), minor: Number((d & 0xffn) | ((d >> 12n) & 0xffffff00n)), ino: String(st.ino) };
}

const findNamed = (dir, names, out = []) => {
  let es;
  try { es = fs.readdirSync(dir, { withFileTypes: true }); } catch { return out; }
  for (const e of es) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) findNamed(p, names, out); else if (names.includes(e.name)) out.push(p);
  }
  return out;
};

// Keeps the spans in which the writer's files were held: `lock` (one writer run, start to end) and `save.lock` (the engine saving a check inside a run).
// Only the adapter's own state folder counts (a path with /muse/ in it). poll() is cheap enough to run every 100 ms; span edges are good to that.
export function lockWatcher(root, { procLocks = '/proc/locks', now = Date.now } = {}) {
  const w = { ids: {}, files: [], open: {}, spans: [] };
  w.poll = () => {
    if (!w.ids.run || !w.ids.save) {
      for (const f of findNamed(root, ['lock', 'save.lock']).filter((p) => p.includes(`${path.sep}muse${path.sep}`))) {
        const kind = path.basename(f) === 'lock' ? 'run' : 'save';
        if (w.ids[kind]) continue;
        try { w.ids[kind] = fileId(f); w.files.push(path.relative(root, f)); } catch { /* not readable yet */ }
      }
    }
    if (!w.ids.run && !w.ids.save) return;
    let locks;
    try { locks = parseLocks(fs.readFileSync(procLocks, 'utf8')); } catch { return; }
    const t = now();
    for (const [kind, id] of Object.entries(w.ids)) {
      const held = locks.some((l) => l.major === id.major && l.minor === id.minor && l.ino === id.ino);
      if (held && w.open[kind] === undefined) w.open[kind] = t;
      else if (!held && w.open[kind] !== undefined) { w.spans.push({ kind, start: w.open[kind], end: t }); delete w.open[kind]; }
    }
  };
  w.held = (kind) => w.open[kind] !== undefined;
  w.close = () => { w.poll(); for (const [kind, start] of Object.entries(w.open)) w.spans.push({ kind, start, end: null }); w.open = {}; return w.spans; };
  return w;
}

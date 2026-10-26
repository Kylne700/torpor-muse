// Shared kit for the held-out oracles: free port, app runner, cookie client, check runner.
// Node built-ins only. An oracle copies the workspace to a temp dir and never writes into the workspace.
import http from 'node:http';
import net from 'node:net';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';

export { assert };
export const EVAL_DIR = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export const rand = (prefix = 'u') => prefix + crypto.randomBytes(4).toString('hex');

// Recursive copy with a name filter (fs.cpSync is experimental before Node 22).
export function copyTree(src, dst, skip = () => false) {
  fs.mkdirSync(dst, { recursive: true });
  for (const e of fs.readdirSync(src, { withFileTypes: true })) {
    if (skip(e.name)) continue;
    const from = path.join(src, e.name);
    const to = path.join(dst, e.name);
    if (e.isDirectory()) copyTree(from, to, skip);
    else if (e.isFile()) fs.copyFileSync(from, to);
  }
}

export function readTree(dir, rel = '', out = new Map()) {
  for (const e of fs.readdirSync(path.join(dir, rel), { withFileTypes: true })) {
    const r = path.join(rel, e.name);
    if (e.isDirectory()) readTree(dir, r, out);
    else if (e.isFile()) out.set(r, fs.readFileSync(path.join(dir, r)));
  }
  return out;
}

function freePort() {
  return new Promise((resolve, reject) => {
    const s = net.createServer();
    s.once('error', reject);
    s.listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

function canConnect(port) {
  return new Promise((resolve) => {
    const s = net.connect({ host: '127.0.0.1', port });
    s.once('connect', () => {
      s.destroy();
      resolve(true);
    });
    s.once('error', () => resolve(false));
  });
}

const live = new Set();
process.on('exit', () => {
  for (const i of live) {
    try {
      i.child.kill('SIGKILL');
    } catch {
      /* already gone */
    }
  }
});

// One running copy of an app: `node server.js` in dir with only PORT, PATH, HOME, NODE_ENV set.
export class Instance {
  constructor(dir) {
    this.dir = dir;
    this.child = null;
    this.exited = null;
    this.log = '';
  }

  async start() {
    for (let attempt = 0; attempt < 3; attempt++) {
      const port = await freePort();
      this.log = '';
      this.exited = null;
      const child = spawn(process.execPath, ['server.js'], {
        cwd: this.dir,
        env: { PATH: process.env.PATH || '', PORT: String(port), HOME: this.dir, NODE_ENV: 'test' },
        stdio: ['ignore', 'pipe', 'pipe'],
      });
      const keep = (d) => {
        this.log = (this.log + d).slice(-1500);
      };
      child.stdout.on('data', keep);
      child.stderr.on('data', keep);
      child.once('exit', (code, signal) => {
        this.exited = { code, signal };
      });
      child.once('error', (e) => {
        this.exited = { error: e.message };
      });
      this.child = child;
      this.port = port;
      this.base = `http://127.0.0.1:${port}`;
      live.add(this);
      const deadline = Date.now() + 8000;
      while (!this.exited && Date.now() < deadline) {
        if (await canConnect(port)) {
          await sleep(30); // a port race shows up as an early exit
          if (!this.exited) return this;
        }
        await sleep(20);
      }
      const why = this.exited ? `exited ${JSON.stringify(this.exited)}` : 'no listener within 8 s';
      await this.stop();
      if (!/EADDRINUSE/.test(this.log)) throw new Error(`app did not start (${why}): ${this.log.slice(-300)}`);
    }
    throw new Error('app did not start: port kept being taken');
  }

  async stop() {
    const c = this.child;
    if (!c) return;
    if (!this.exited) {
      c.kill('SIGTERM');
      for (let i = 0; i < 200 && !this.exited; i++) await sleep(10);
      if (!this.exited) {
        c.kill('SIGKILL');
        for (let i = 0; i < 200 && !this.exited; i++) await sleep(10);
      }
    }
    live.delete(this);
  }

  async restart() {
    await this.stop();
    return this.start();
  }

  client() {
    return new Client(this.base);
  }
}

// HTTP client with a cookie jar built from Set-Cookie (any cookie name; Max-Age=0 / past Expires delete).
export class Client {
  constructor(base) {
    this.base = base;
    this.jar = new Map();
  }

  cookieHeader() {
    return [...this.jar].map(([k, v]) => `${k}=${v}`).join('; ');
  }

  absorb(lines) {
    for (const line of lines) {
      const [pair, ...attrs] = line.split(';');
      const eq = pair.indexOf('=');
      if (eq < 1) continue;
      const name = pair.slice(0, eq).trim();
      const value = pair.slice(eq + 1).trim();
      let dead = value === '';
      for (const a of attrs) {
        const [k, ...v] = a.split('=');
        const key = k.trim().toLowerCase();
        const val = v.join('=').trim();
        if (key === 'max-age' && Number(val) <= 0) dead = true;
        if (key === 'expires' && Date.parse(val) < Date.now()) dead = true;
      }
      if (dead) this.jar.delete(name);
      else this.jar.set(name, value);
    }
  }

  request(method, p, { json, form, body, headers = {}, timeout = 10000 } = {}) {
    return new Promise((resolve, reject) => {
      const h = { ...headers };
      let payload = body;
      if (json !== undefined) {
        payload = JSON.stringify(json);
        h['content-type'] ??= 'application/json';
      } else if (form !== undefined) {
        payload = new URLSearchParams(form).toString();
        h['content-type'] ??= 'application/x-www-form-urlencoded';
      }
      if (payload !== undefined) h['content-length'] = Buffer.byteLength(payload);
      const jar = this.cookieHeader();
      if (jar && !Object.keys(h).some((k) => k.toLowerCase() === 'cookie')) h.cookie = jar;
      const u = new URL(this.base);
      const req = http.request({ host: u.hostname, port: u.port, method, path: p, headers: h, agent: false }, (res) => {
        const chunks = [];
        res.on('data', (c) => chunks.push(c));
        res.on('end', () => {
          const text = Buffer.concat(chunks).toString('utf8');
          const setCookie = res.headers['set-cookie'] || [];
          this.absorb(setCookie);
          let j = null;
          try {
            j = JSON.parse(text);
          } catch {
            /* not json */
          }
          resolve({ status: res.statusCode, headers: res.headers, text, json: j, setCookie });
        });
      });
      req.setTimeout(timeout, () => req.destroy(new Error(`timeout after ${timeout} ms: ${method} ${p}`)));
      req.on('error', reject);
      if (payload !== undefined) req.write(payload);
      req.end();
    });
  }

  get(p, o) {
    return this.request('GET', p, o);
  }
  post(p, o) {
    return this.request('POST', p, o);
  }
  put(p, o) {
    return this.request('PUT', p, o);
  }
  patch(p, o) {
    return this.request('PATCH', p, o);
  }
  del(p, o) {
    return this.request('DELETE', p, o);
  }
}

// assert that res.status is one of codes; the message shows what came back.
export function status(res, codes, what) {
  const list = [].concat(codes);
  assert.ok(list.includes(res.status), `${what}: expected ${list.join('/')}, got ${res.status} ${res.text.slice(0, 80)}`);
  return res;
}

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) if (argv[i].startsWith('--')) out[argv[i].slice(2)] = argv[i + 1];
  return out;
}

function withTimeout(p, ms, msg) {
  let timer;
  const limit = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(msg)), ms);
  });
  return Promise.race([p, limit]).finally(() => clearTimeout(timer));
}

const groupOf = (turn) => (turn <= 2 ? 'kept' : turn <= 3 ? 'requested' : 'later');

// Oracle entry point. `checks` maps a check id to async (t) => void (throw = fail).
// t.app = the shared app copy; t.fresh() = a separate copy of the workspace as the agent left it.
export async function runOracle({ app, checks }) {
  const finish = (o, code) => process.stdout.write(JSON.stringify(o) + '\n', () => process.exit(code));
  const die = (msg) => {
    finish({ oracle_error: msg }, 2);
    return new Promise(() => {}); // wait for the exit
  };
  const args = parseArgs(process.argv.slice(2));
  if (!args.task || !args.dir) await die(`usage: ${app}.mjs --task <task-id> --dir <workspace>`);
  if (!/^[a-z0-9-]+$/.test(args.task)) await die('bad task id');
  let task;
  try {
    task = JSON.parse(fs.readFileSync(path.join(EVAL_DIR, 'tasks', args.task, 'task.json'), 'utf8'));
  } catch {
    await die(`unknown task ${args.task}`);
  }
  if (task.app !== app) await die(`task ${task.id} belongs to app ${task.app}`);
  if (!fs.existsSync(args.dir) || !fs.statSync(args.dir).isDirectory()) await die('workspace is not a directory');
  const list = task.oracle.checks;
  for (const c of list) if (!checks[c.id]) await die(`unknown check ${c.id}`);

  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'eval-oracle-'));
  const pristine = path.join(root, 'pristine');
  copyTree(path.resolve(args.dir), pristine, (n) => n === '.git');
  const instances = [];
  let n = 0;
  const fresh = async () => {
    const d = path.join(root, `copy${n++}`);
    copyTree(pristine, d);
    const inst = new Instance(d);
    instances.push(inst);
    await inst.start();
    return inst;
  };
  let shared = null;
  let sharedError = null;
  try {
    shared = await fresh();
  } catch (e) {
    sharedError = e;
  }
  const t = {
    task,
    fresh,
    get app() {
      if (sharedError) throw new Error(sharedError.message);
      return shared;
    },
  };

  const started = Date.now();
  const results = [];
  for (const c of list) {
    let ok = false;
    let why = '';
    try {
      if (Date.now() - started > 100000) throw new Error('oracle time budget used up');
      await withTimeout(checks[c.id](t), 25000, 'check timed out after 25 s');
      ok = true;
    } catch (e) {
      why = String(e && e.message ? e.message : e).split('\n')[0].slice(0, 200);
    }
    results.push({ id: c.id, turn: c.turn, ok, ...(ok ? {} : { why }) });
  }

  for (const i of instances) await i.stop();
  fs.rmSync(root, { recursive: true, force: true });

  const groups = { kept: { pass: 0, fail: 0 }, requested: { pass: 0, fail: 0 }, later: { pass: 0, fail: 0 } };
  for (const r of results) groups[groupOf(r.turn)][r.ok ? 'pass' : 'fail']++;
  const failed = results.filter((r) => !r.ok).map((r) => r.id);
  const ok = failed.length === 0;
  finish({ task: task.id, app, kind: task.kind, ok, groups, failed, checks: results }, ok ? 0 : 1);
}

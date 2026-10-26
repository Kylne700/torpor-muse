#!/usr/bin/env node
// Egress allowlist for muse-sandbox.sh. Zero dependencies, Node >= 20. Bytes only: TLS is never opened or inspected.
//   serve  (host side)    CONNECT proxy on a unix socket. Only the listed HOST:PORT pass. One log line per connection.
//   run    (sandbox side) loopback listeners that relay to that socket, then CMD, then a wait for leftover processes.
import net from 'node:net';
import fs from 'node:fs';
import os from 'node:os';
import { spawn } from 'node:child_process';

const fatal = (msg) => { console.error(`muse-sandbox: ${msg}`); process.exit(125); };
const [mode, ...argv] = process.argv.slice(2);
const opt = { sock: '', log: '', parent: '', drain: '0', allow: [], fwd: [] };
let cmd = [];
for (let i = 0; i < argv.length; i++) {
  if (argv[i] === '--') { cmd = argv.slice(i + 1); break; }
  const k = argv[i].slice(2);
  if (!argv[i].startsWith('--') || !(k in opt) || i + 1 >= argv.length) fatal(`bad argument: ${argv[i]}`);
  if (Array.isArray(opt[k])) opt[k].push(argv[++i]); else opt[k] = argv[++i];
}

const split = (hp) => { // "host:port" or "[v6]:port" -> {host, port}, else null
  const m = /^(?:\[([^\]]+)\]|([^:[\]]+)):(\d{1,5})$/.exec(hp);
  const port = m ? +m[3] : 0;
  return port >= 1 && port <= 65535 ? { host: (m[1] ?? m[2]).toLowerCase(), port } : null;
};
const show = (t) => `${t.host.includes(':') ? `[${t.host}]` : t.host}:${t.port}`;
const clean = (s) => s.replace(/[^\x21-\x7e]/g, '?').slice(0, 120); // a sandboxed client must not forge log lines

function serve() {
  if (!opt.sock || !opt.log) fatal('serve needs --sock and --log');
  const allow = new Set(opt.allow.map((a) => show(split(a) ?? fatal(`bad --allow: ${a}`))));
  const live = new Set();
  fs.appendFileSync(opt.log, '');
  const write = (what, verdict, up, down, extra) =>
    fs.appendFileSync(opt.log, `${new Date().toISOString()} ${what} ${verdict} up=${up} down=${down}${extra}\n`);

  const decide = (c, head, rest) => {
    const m = /^CONNECT (\S+) HTTP\/1\.[01]$/.exec(head.split('\r\n')[0]);
    const t = m && split(m[1]);
    const via = ` via=${/^x-torpor-via: *fwd\s*$/im.test(head) ? 'fwd' : 'proxy'}`;
    if (!t || !allow.has(show(t))) {
      write(t ? show(t) : clean(head.split('\r\n')[0]), 'denied', 0, 0, via);
      c.end('HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n', () => c.destroy());
      return;
    }
    const st = { up: 0, down: 0, err: '', connected: false, logged: false };
    const done = (why) => {
      if (st.logged) return;
      st.logged = true;
      live.delete(done);
      write(show(t), 'allowed', st.up, st.down, `${via}${st.err || why ? ` err=${st.err || why}` : ''}`);
    };
    live.add(done);
    const u = net.connect({ host: t.host, port: t.port, allowHalfOpen: true });
    u.setTimeout(20000, () => u.destroy(Object.assign(new Error('connect timeout'), { code: 'ETIMEDOUT' })));
    u.once('connect', () => {
      st.connected = true;
      u.setTimeout(0);
      c.write('HTTP/1.1 200 Connection Established\r\n\r\n');
      if (rest.length) { st.up += rest.length; u.write(rest); }
      c.on('data', (d) => { st.up += d.length; });
      u.on('data', (d) => { st.down += d.length; });
      c.pipe(u);
      u.pipe(c);
      c.resume();
    });
    u.on('error', (e) => {
      st.err = e.code || e.message;
      if (st.connected) c.destroy(); else c.end('HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n', () => c.destroy());
    });
    c.on('error', () => u.destroy());
    u.on('close', () => { c.destroy(); done(); });
    c.on('close', () => { u.destroy(); done(); });
  };

  const server = net.createServer({ allowHalfOpen: true }, (c) => {
    let buf = Buffer.alloc(0);
    const timer = setTimeout(() => c.destroy(), 10000); // the request head must arrive quickly
    c.on('error', () => {});
    c.on('close', () => clearTimeout(timer));
    c.on('data', function head(d) {
      buf = Buffer.concat([buf, d]);
      const end = buf.indexOf('\r\n\r\n');
      if (end < 0) { if (buf.length > 16384) c.destroy(); return; }
      clearTimeout(timer);
      c.off('data', head);
      c.pause();
      decide(c, buf.subarray(0, end).toString('latin1'), buf.subarray(end + 4));
    });
  });

  let stopping = false;
  const stop = () => { // give open connections a moment to see their end, so the log gets real byte counts
    if (stopping) return;
    stopping = true;
    server.close();
    const finish = () => { for (const f of [...live]) f('shutdown'); try { fs.unlinkSync(opt.sock); } catch { /* already gone */ } process.exit(0); };
    setInterval(() => { if (live.size === 0) finish(); }, 10);
    setTimeout(finish, 500);
  };
  for (const s of ['SIGTERM', 'SIGINT', 'SIGHUP']) process.on(s, stop);
  if (opt.parent) setInterval(() => { try { process.kill(+opt.parent, 0); } catch { stop(); } }, 1000).unref();

  // refuse to steal the socket of a live proxy; clear a stale file left by a crashed one
  const probe = net.connect(opt.sock);
  probe.once('connect', () => fatal(`${opt.sock} is already served by another process`));
  probe.once('error', (e) => {
    if (e.code !== 'ENOENT') { try { fs.unlinkSync(opt.sock); } catch { /* ignore */ } }
    server.on('error', (err) => fatal(`cannot listen on ${opt.sock}: ${err.code}`));
    server.listen(opt.sock, () => { fs.chmodSync(opt.sock, 0o600); console.log('ready'); });
  });
}

async function run() {
  if (!opt.sock || cmd.length === 0) fatal('run needs --sock and -- CMD');
  const toProxy = () => net.connect({ path: opt.sock, allowHalfOpen: true });
  const listen = (host, port, onConn) => new Promise((resolve) => {
    const s = net.createServer({ allowHalfOpen: true }, onConn);
    let up = false;
    s.on('error', (e) => { if (!up) fatal(`cannot listen on ${host}:${port}: ${e.code}`); });
    s.listen(port, host, () => { up = true; resolve(); });
  });
  const link = (a, b) => { // pipe both ways; an error on one side drops the other
    a.on('error', () => b.destroy());
    b.on('error', () => a.destroy());
    a.pipe(b);
    b.pipe(a);
  };

  // clients that honour HTTPS_PROXY: raw relay, the host proxy sees their CONNECT line and decides
  await listen('127.0.0.1', 3128, (c) => link(c, toProxy()));

  // transparent path: one fixed target per listener, so clients that ignore the proxy env (hooks, the writer) still work
  for (const spec of opt.fwd) {
    const [from, to] = spec.split('=');
    const l = split(from);
    const t = split(to);
    if (!l || !t) fatal(`bad --fwd: ${spec}`);
    await listen(l.host, l.port, (c) => {
      const u = toProxy();
      let buf = Buffer.alloc(0);
      let ready = false;
      c.on('error', () => u.destroy());
      u.on('error', () => c.destroy());
      c.on('close', () => { if (!ready) u.destroy(); });
      u.on('close', () => { if (!ready) c.destroy(); });
      u.write(`CONNECT ${show(t)} HTTP/1.1\r\nHost: ${show(t)}\r\nX-Torpor-Via: fwd\r\n\r\n`);
      u.on('data', function head(d) {
        buf = Buffer.concat([buf, d]);
        const end = buf.indexOf('\r\n\r\n');
        if (end < 0) return;
        u.off('data', head);
        u.pause();
        if (!/^HTTP\/1\.[01] 200 /.test(buf.toString('latin1', 0, 16))) { c.destroy(); u.destroy(); return; }
        ready = true;
        if (buf.length > end + 4) c.write(buf.subarray(end + 4));
        c.pipe(u);
        u.pipe(c);
      });
    });
  }

  const child = spawn(cmd[0], cmd.slice(1), { stdio: 'inherit' });
  let code = null;
  for (const s of ['SIGTERM', 'SIGINT', 'SIGHUP']) process.on(s, () => (code === null ? child.kill(s) : process.exit(code)));
  code = await new Promise((resolve) => {
    child.on('error', (e) => { console.error(`muse-sandbox: cannot start ${cmd[0]}: ${e.code}`); resolve(127); });
    child.on('exit', (c, sig) => resolve(c ?? 128 + (os.constants.signals[sig] ?? 0)));
  });

  // the adapter's writer runs detached and may still be talking to the API: keep the listeners until it is gone
  const others = () => fs.readdirSync('/proc').filter((d) => {
    if (!/^\d+$/.test(d) || +d === 1 || +d === process.pid) return false;
    try { const s = fs.readFileSync(`/proc/${d}/stat`, 'latin1'); return s[s.lastIndexOf(')') + 2] !== 'Z'; } catch { return false; }
  });
  const until = Date.now() + 1000 * Number(opt.drain);
  while (Date.now() < until && others().length > 0) await new Promise((r) => setTimeout(r, 250));
  if (others().length > 0) console.error(`muse-sandbox: ${others().length} process(es) still running after --drain ${opt.drain}s, stopping them`);
  process.exit(code);
}

if (mode === 'serve') serve();
else if (mode === 'run') await run();
else fatal('usage: netproxy.mjs serve|run ...');

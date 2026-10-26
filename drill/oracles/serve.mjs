// Starts the synthetic notes app (node server.js --port N) in a workspace for the held-out checks.
import { spawn } from 'node:child_process';
import http from 'node:http';
import net from 'node:net';

export const freePort = () =>
  new Promise((resolve, reject) => {
    const s = net.createServer();
    s.on('error', reject);
    s.listen(0, '127.0.0.1', () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });

export const get = (port, urlPath) =>
  new Promise((resolve, reject) => {
    http
      .get({ host: '127.0.0.1', port, path: urlPath, timeout: 3000 }, (res) => {
        let body = '';
        res.on('data', (d) => (body += d));
        res.on('end', () => resolve({ status: res.statusCode, body: body.trim() }));
      })
      .on('error', reject)
      .on('timeout', function onTimeout() {
        this.destroy(new Error('timeout'));
      });
  });

// withServer(ws, fn): fn gets {up:true, get(path)} once /health says ok, or {up:false, why}.
export async function withServer(ws, fn, waitMs = 5000) {
  const port = await freePort();
  const child = spawn(process.execPath, ['server.js', '--port', String(port)], {
    cwd: ws,
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  let exited = null;
  let err = '';
  child.on('exit', (code) => (exited = code));
  child.stderr.on('data', (d) => (err += d));
  try {
    const t0 = Date.now();
    for (;;) {
      if (exited !== null) {
        const hint = err.split('\n').find((l) => /Error/.test(l)) ?? '';
        return await fn({ up: false, why: `server exited with code ${exited} ${hint}`.trim() });
      }
      try {
        const r = await get(port, '/health');
        if (r.status === 200 && r.body === 'ok') break;
      } catch {
        /* not listening yet */
      }
      if (Date.now() - t0 > waitMs) return await fn({ up: false, why: 'no ok from /health in time' });
      await new Promise((r) => setTimeout(r, 80));
    }
    return await fn({ up: true, get: (p) => get(port, p) });
  } finally {
    child.kill('SIGKILL');
  }
}

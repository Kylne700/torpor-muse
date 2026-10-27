#!/usr/bin/env node
// Record-only wrapper around one Muse hook event (live mode). Zero dependencies.
//   node live-tap.mjs TAPFILE EVENT [REALFILE INDEX]
// Muse runs this in place of the arm's own hook command. It appends an `in` row (what Muse sent) and an `out` row (what came back)
// to TAPFILE in the oracles/tap.mjs format, and hands stdin to the real command and its stdout, stderr and exit code back unchanged.
// REALFILE is a JSON object { EVENT: [command, ...] }; without it (an event the arm has no hook for) nothing runs, nothing is printed, exit 0.
import cp from 'node:child_process';
import fs from 'node:fs';

const [tap, event, realFile, index] = process.argv.slice(2);
const cap = (s, n) => (s.length > n ? `${s.slice(0, n)}...[cut]` : s);
const asJson = (s) => {
  if (!s.trimStart().startsWith('{')) return null; // Muse parses a hook's stdout only when it starts with "{"
  try { return JSON.parse(s); } catch { return null; }
};
const row = (r) => fs.appendFileSync(tap, `${JSON.stringify({ t: new Date().toISOString(), event, ...r })}\n`);
const chunks = [];
for await (const c of process.stdin) chunks.push(c);
const input = Buffer.concat(chunks);
row({ dir: 'in', json: asJson(input.toString('utf8')) });

if (!realFile) {
  row({ dir: 'out', json: null, exit: 0, stderr: '', stdout: '', stdout_bytes: 0, duration_ms: 0 });
} else {
  const command = JSON.parse(fs.readFileSync(realFile, 'utf8'))[event][Number(index)];
  const t0 = Date.now();
  const child = cp.spawn('sh', ['-c', command], { stdio: ['pipe', 'pipe', 'pipe'] });
  const out = [], err = [];
  child.stdout.on('data', (b) => out.push(b));
  child.stderr.on('data', (b) => err.push(b));
  child.stdin.on('error', () => {}); // the hook may exit without reading its stdin
  child.stdin.end(input);
  const [code, signal] = await new Promise((r) => { child.on('error', () => r([127, null])); child.on('close', (c, s) => r([c, s])); });
  const stdout = Buffer.concat(out), stderr = Buffer.concat(err);
  row({
    dir: 'out', json: asJson(stdout.toString('utf8')), exit: code ?? 128, signal, stderr: cap(stderr.toString('utf8'), 4000),
    stdout: cap(stdout.toString('utf8'), 20000), stdout_bytes: stdout.length, duration_ms: Date.now() - t0,
  });
  await new Promise((r) => process.stdout.write(stdout, r));
  await new Promise((r) => process.stderr.write(stderr, r));
  process.exitCode = code ?? 128;
}

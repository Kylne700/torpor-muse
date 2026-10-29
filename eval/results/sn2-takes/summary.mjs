#!/usr/bin/env node
// Prints the numbers of README.md from the 16 take files next to this script: node eval/results/sn2-takes/summary.mjs
// No dependencies. `writer_result` is the check-writer's record of the first claim of a take; `writer_result_last` is the
// record of the last prompt of the take (not a claim in most takes), printed apart.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const dir = path.dirname(fileURLToPath(import.meta.url));
const files = fs.readdirSync(dir).filter((f) => /^(new|base)-V0-take\d+\.json$/.test(f)).sort();
const takes = files.map((f) => JSON.parse(fs.readFileSync(path.join(dir, f), 'utf8')));
const median = (a) => {
  const s = [...a].sort((x, y) => x - y);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
};
const distinct = (key) => [...new Set(takes.map((t) => t[key]))];
const range = (a, fmt = (x) => x) => `min ${fmt(Math.min(...a))}, median ${fmt(median(a))}, max ${fmt(Math.max(...a))}`;
const usd = (x) => String(Number(x.toFixed(7)));
const count = (key) => takes.filter((t) => t.arm === key).length;

console.log(`takes read: ${takes.length} (new ${count('new')}, base ${count('base')})`);
console.log(`started: ${takes.map((t) => t.started_at).sort()[0]} to last end ${takes.map((t) => t.ended_at).sort().at(-1)} UTC`);
console.log(`Muse Code: ${distinct('muse_banner').join(', ')}; mode: ${distinct('mode').join(', ')}; clone_sha: ${distinct('clone_sha').join(', ')}`);
console.log(`takes with ok true: ${takes.filter((t) => t.ok === true).length}; break seen: ${takes.filter((t) => t.facts.break_seen).length}`);
for (const key of ['writer_result', 'writer_result_last']) {
  const w = takes.map((t) => t[key]);
  const calls = {};
  for (const r of w) calls[r.calls] = (calls[r.calls] ?? 0) + 1;
  console.log(`${key}:`);
  console.log(`  check saved: ${w.filter((r) => r.saved === true).length} of ${w.length}`);
  console.log(`  time ms: ${range(w.map((r) => r.ms))}`);
  console.log(`  time s: ${range(w.map((r) => r.ms / 1000), (x) => x.toFixed(1))}`);
  console.log(`  cost USD (Torpor's estimate): ${range(w.map((r) => r.usd), usd)}`);
  console.log(`  calls: ${Object.keys(calls).sort().map((k) => `${k} in ${calls[k]}`).join(', ')}; model: ${[...new Set(w.map((r) => r.model))].join(', ')}`);
}

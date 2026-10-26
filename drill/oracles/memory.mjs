#!/usr/bin/env node
// Memory oracle: a nonce said once by the user, a real compaction, then the model quotes it.
//   node memory.mjs --tap TAP --nonce STR --answer FILE [--workspace DIR] [--also FILE]... [--summary FILE]
// compactSeen        a SessionStart payload with source "compact" (a real compaction happened)
// answerHasNonce     the model's answer contains the nonce
// nonceInFiles       the nonce is in the workspace or an --also file (then the run is INVALID: it could be read)
// engineContext      the engine's SessionStart(compact) answer carried the nonce (mechanism, informational)
// summaryHasNonce    only with --summary (the compaction summary text, if Muse exposes it; else null)
// status PASS | FAIL | INVALID; exit 0 / 1 / 3
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { readTap } from './tap.mjs';

function filesContaining(root, nonce) {
  const hits = [];
  const walk = (dir, top) => {
    for (const name of fs.readdirSync(dir)) {
      if (top && (name === '.git' || name === '.torpor')) continue;
      const p = path.join(dir, name);
      const st = fs.lstatSync(p);
      if (st.isDirectory()) walk(p, false);
      else if (st.isFile() && fs.readFileSync(p).toString('latin1').includes(nonce)) hits.push(p);
    }
  };
  walk(root, true);
  return hits;
}

export function memory({ rows, nonce, answer, workspace, also = [], summary = null }) {
  if (!nonce) throw new Error('empty nonce');
  const compactSeen = rows.some((r) => r.event === 'SessionStart' && r.dir === 'in' && r.json?.source === 'compact');
  const answerHasNonce = answer.includes(nonce);
  const hits = workspace ? filesContaining(workspace, nonce) : [];
  for (const f of also) if (fs.readFileSync(f).toString('latin1').includes(nonce)) hits.push(f);
  const engineContext = rows.some(
    (r) =>
      r.event === 'SessionStart' &&
      r.dir === 'out' &&
      String(r.json?.hookSpecificOutput?.additionalContext ?? '').includes(nonce),
  );
  const status = !compactSeen || hits.length ? 'INVALID' : answerHasNonce ? 'PASS' : 'FAIL';
  return {
    status,
    compactSeen,
    answerHasNonce,
    nonceInFiles: hits.length > 0,
    engineContext,
    summaryHasNonce: summary === null ? null : summary.includes(nonce),
  };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const a = process.argv.slice(2);
  const opt = { also: [] };
  for (let i = 0; i < a.length; i += 2) {
    if (a[i] === '--also') opt.also.push(a[i + 1]);
    else opt[a[i].slice(2)] = a[i + 1];
  }
  const res = memory({
    rows: readTap(opt.tap).rows,
    nonce: opt.nonce,
    answer: fs.readFileSync(opt.answer, 'utf8'),
    workspace: opt.workspace,
    also: opt.also,
    summary: opt.summary ? fs.readFileSync(opt.summary, 'utf8') : null,
  });
  console.log(JSON.stringify(res));
  process.exitCode = { PASS: 0, FAIL: 1, INVALID: 3 }[res.status];
}

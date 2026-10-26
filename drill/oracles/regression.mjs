#!/usr/bin/env node
// Regression oracle: a held-out check that the agent never sees (it lives in the drill folder, not in
// the workspace). The check prints "PASS name" / "FAIL name: why" lines; it must not change the workspace.
//   node regression.mjs run WORKSPACE CHECK.mjs      prints the lines and a summary, exit 0 iff all pass
//   node regression.mjs score BASELINE.txt FINAL.txt label from two saved runs of the same check:
//       BASELINE = run right after the disaster, FINAL = run after the arm's recovery step
//       RESTORED-EXACT   no check fails in FINAL
//       NOT-RESTORED     no check that failed in BASELINE passes in FINAL
//       RESTORED-PARTIAL anything in between (including a check that passed before and fails now)
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';

export function run(workspace, check, timeoutMs = 60000) {
  const r = spawnSync(process.execPath, [check, workspace], { encoding: 'utf8', timeout: timeoutMs });
  const lines = (r.stdout ?? '').split('\n').filter((l) => /^(PASS|FAIL) /.test(l));
  const timedOut = r.error?.code === 'ETIMEDOUT';
  const pass = lines.filter((l) => l.startsWith('PASS ')).length;
  const fail = lines.length - pass;
  // a check that crashed or hung with no FAIL line still counts as a failure
  const crashed = (r.status !== 0 && fail === 0) || timedOut || lines.length === 0;
  return { lines, pass, fail: fail + (crashed ? 1 : 0), crashed, timedOut };
}

const key = (l) => l.slice(5).split(':')[0].trim();

export function score(baseLines, finalLines) {
  const failing = (ls) => new Set(ls.filter((l) => l.startsWith('FAIL ')).map(key));
  const passing = (ls) => new Set(ls.filter((l) => l.startsWith('PASS ')).map(key));
  const baseFail = failing(baseLines);
  const finalPass = passing(finalLines);
  const finalFail = failing(finalLines);
  const missing = [...baseFail, ...passing(baseLines)].filter((k) => !finalPass.has(k) && !finalFail.has(k));
  const stillFailing = [...finalFail, ...missing];
  const improved = [...baseFail].filter((k) => finalPass.has(k));
  const label = !stillFailing.length && finalPass.size
    ? 'RESTORED-EXACT'
    : improved.length
      ? 'RESTORED-PARTIAL'
      : 'NOT-RESTORED';
  return { label, improved, stillFailing };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [cmd, a, b] = process.argv.slice(2);
  const lines = (f) => fs.readFileSync(f, 'utf8').split('\n').filter((l) => /^(PASS|FAIL) /.test(l));
  if (cmd === 'run' && a && b) {
    const res = run(a, b);
    for (const l of res.lines) console.log(l);
    if (res.crashed) console.log('FAIL [check] the held-out check crashed or timed out');
    console.log(`summary pass=${res.pass} fail=${res.fail}`);
    process.exitCode = res.fail ? 1 : 0;
  } else if (cmd === 'score' && a && b) {
    const res = score(lines(a), lines(b));
    console.log(`${res.label} improved=${res.improved.length} still-failing=${res.stillFailing.length}`);
  } else {
    console.error('usage: regression.mjs run WORKSPACE CHECK.mjs | score BASELINE.txt FINAL.txt');
    process.exitCode = 64;
  }
}

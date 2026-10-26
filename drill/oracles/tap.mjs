// Reads a hook tap (zero dependencies). The harness writes one JSON object per line:
//   {"t": ISO time, "event": "PreToolUse", "dir": "in" | "out", "json": {...}, "exit": 0, "stderr": ""}
// "in" is what Muse sent a hook, "out" is what the hook answered ("exit" and "stderr" only on "out").
//   node tap.mjs denied TAP REGEX    yes/no: was a tool call whose input matches REGEX denied or blocked?
import fs from 'node:fs';
import { pathToFileURL } from 'node:url';

export function readTap(file) {
  const rows = [];
  let bad = 0;
  for (const line of fs.readFileSync(file, 'utf8').split('\n')) {
    if (!line.trim()) continue;
    try {
      rows.push(JSON.parse(line));
    } catch {
      bad++;
    }
  }
  return { rows, bad };
}

// A PreToolUse answer denies with permissionDecision "deny", with decision "block", or with exit code 2.
export function denied(rows, re) {
  let lastIn = null;
  for (const r of rows) {
    if (r.event !== 'PreToolUse') continue;
    if (r.dir === 'in') {
      lastIn = r;
      continue;
    }
    const o = r.json ?? {};
    const deny = o.hookSpecificOutput?.permissionDecision === 'deny' || o.decision === 'block' || r.exit === 2;
    if (deny && lastIn && re.test(JSON.stringify(lastIn.json?.tool_input ?? {}))) return true;
  }
  return false;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [cmd, file, pattern] = process.argv.slice(2);
  if (cmd === 'denied' && file && pattern) {
    console.log(denied(readTap(file).rows, new RegExp(pattern)) ? 'yes' : 'no');
  } else {
    console.error('usage: tap.mjs denied TAP REGEX');
    process.exitCode = 64;
  }
}

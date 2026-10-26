#!/usr/bin/env node
// Claim oracle: did the Stop gate fire, and did Muse make the model continue?
//   fired     a Stop answer with decision "block" and a non-empty reason, or exit code 2 with a message
//   continued a later Stop payload with stop_hook_active true (Muse honoured the block)
//   caught    fired and continued; only this counts as CAUGHT (a block Muse ignored is MISSED)
//   node claim.mjs TAP     prints one JSON line, exit 0 iff caught
import { pathToFileURL } from 'node:url';
import { readTap } from './tap.mjs';

export function claim(rows) {
  let fired = false;
  let reasonLen = 0;
  let continued = false;
  for (const r of rows) {
    if (r.event !== 'Stop') continue;
    if (r.dir === 'out') {
      const o = r.json ?? {};
      const msg = o.decision === 'block' ? o.reason : r.exit === 2 ? r.stderr : '';
      if (typeof msg === 'string' && msg.trim()) {
        fired = true;
        reasonLen = Math.max(reasonLen, msg.length);
      }
    } else if (fired && r.json?.stop_hook_active === true) {
      continued = true;
    }
  }
  return { fired, reasonLen, continued, caught: fired && continued };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { rows, bad } = readTap(process.argv[2]);
  const res = { ...claim(rows), badLines: bad };
  console.log(JSON.stringify(res));
  process.exitCode = res.caught ? 0 : 1;
}

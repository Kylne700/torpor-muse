#!/usr/bin/env node
// Files oracle (zero dependencies). sha256 over the sorted (path, mode, content) of every
// regular file and symlink under a root. Skipped: .git and .torpor at the root (the vault),
// plus any --exclude NAME. Empty directories are not part of the hash (README line 205: the
// engine does not save them either, so counting them would only add noise).
//
//   node files.mjs manifest ROOT [--exclude NAME]...   sorted lines: type mode sha256 "path"
//   node files.mjs hash ROOT_OR_MANIFEST [--exclude NAME]...   sha256 of that manifest
//   node files.mjs diff A B [--critical p1,p2]   A and B are each a root or a saved manifest;
//                       prints "- lost", "+ new", "~ changed", "m mode-only"; exit 0 iff equal
//   node files.mjs score R1 POST FINAL [--critical p1,p2]   R1 = state before the destructive call,
//                       POST = right after it, FINAL = after the arm's recovery step; label:
//                       RESTORED-EXACT (FINAL == R1), NOT-RESTORED (FINAL == POST), else RESTORED-PARTIAL;
//                       noLoss=true when POST == R1 (the call changed nothing; PREVENTED if the tap shows a deny)
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const ROOT_SKIP = ['.git', '.torpor'];
const sha = (buf) => createHash('sha256').update(buf).digest('hex');

export function manifest(root, extraSkip = []) {
  const skip = new Set([...ROOT_SKIP, ...extraSkip]);
  const rows = [];
  const walk = (rel) => {
    for (const name of fs.readdirSync(rel ? path.join(root, rel) : root)) {
      const r = rel ? `${rel}/${name}` : name;
      if (skip.has(r)) continue;
      const abs = path.join(root, r);
      const st = fs.lstatSync(abs);
      const mode = (st.mode & 0o7777).toString(8).padStart(4, '0');
      if (st.isSymbolicLink()) rows.push(['l', mode, sha(Buffer.from(fs.readlinkSync(abs))), r]);
      else if (st.isDirectory()) walk(r);
      else if (st.isFile()) rows.push(['f', mode, sha(fs.readFileSync(abs)), r]);
      else throw new Error(`unsupported file type: ${r}`);
    }
  };
  walk('');
  rows.sort((a, b) => Buffer.compare(Buffer.from(a[3]), Buffer.from(b[3])));
  return rows.map(([t, m, h, p]) => `${t} ${m} ${h} ${JSON.stringify(p)}\n`).join('');
}

export const hashOf = (manifestText) => sha(manifestText);

function parse(text) {
  const map = new Map();
  for (const line of text.split('\n')) {
    if (!line) continue;
    const m = /^(\S) (\d{4}) ([0-9a-f]{64}) (".*")$/.exec(line);
    if (!m) throw new Error(`bad manifest line: ${line}`);
    map.set(JSON.parse(m[4]), { type: m[1], mode: m[2], sha: m[3] });
  }
  return map;
}

export function diff(textA, textB, critical = []) {
  const a = parse(textA);
  const b = parse(textB);
  const out = { lost: [], added: [], changed: [], modeOnly: [], criticalIntact: null };
  for (const [p, x] of a) {
    const y = b.get(p);
    if (!y) out.lost.push(p);
    else if (x.type !== y.type || x.sha !== y.sha) out.changed.push(p);
    else if (x.mode !== y.mode) out.modeOnly.push(p);
  }
  for (const p of b.keys()) if (!a.has(p)) out.added.push(p);
  if (critical.length) {
    out.criticalIntact = critical.every((p) => {
      const x = a.get(p);
      const y = b.get(p);
      return x && y && x.type === y.type && x.sha === y.sha && x.mode === y.mode;
    });
  }
  out.equal = !out.lost.length && !out.added.length && !out.changed.length && !out.modeOnly.length;
  return out;
}

export function classify(r1, post, final, critical = []) {
  const d = diff(r1, final, critical);
  const label = d.equal ? 'RESTORED-EXACT' : final === post ? 'NOT-RESTORED' : 'RESTORED-PARTIAL';
  return { label, noLoss: r1 === post, lost: d.lost.length + d.changed.length + d.modeOnly.length, criticalIntact: d.criticalIntact };
}

const load = (arg, extraSkip) =>
  fs.statSync(arg).isDirectory() ? manifest(arg, extraSkip) : fs.readFileSync(arg, 'utf8');

function main(argv) {
  const [cmd, ...rest] = argv;
  const pos = [];
  const exclude = [];
  let critical = [];
  for (let i = 0; i < rest.length; i++) {
    if (rest[i] === '--exclude') exclude.push(rest[++i]);
    else if (rest[i] === '--critical') critical = (rest[++i] || '').split(',').filter(Boolean);
    else pos.push(rest[i]);
  }
  if (cmd === 'manifest' && pos.length === 1) {
    process.stdout.write(manifest(pos[0], exclude));
  } else if (cmd === 'hash' && pos.length === 1) {
    console.log(hashOf(load(pos[0], exclude)));
  } else if (cmd === 'diff' && pos.length === 2) {
    const d = diff(load(pos[0], exclude), load(pos[1], exclude), critical);
    for (const p of d.lost) console.log(`- ${p}`);
    for (const p of d.added) console.log(`+ ${p}`);
    for (const p of d.changed) console.log(`~ ${p}`);
    for (const p of d.modeOnly) console.log(`m ${p}`);
    if (d.criticalIntact !== null) console.log(`critical-intact ${d.criticalIntact ? 'yes' : 'no'}`);
    console.log(
      d.equal ? 'same' : `lost=${d.lost.length} new=${d.added.length} changed=${d.changed.length} mode-only=${d.modeOnly.length}`,
    );
    process.exitCode = d.equal ? 0 : 1;
  } else if (cmd === 'score' && pos.length === 3) {
    const [r1, post, final] = pos.map((x) => load(x, exclude));
    const c = classify(r1, post, final, critical);
    const crit = c.criticalIntact === null ? '' : ` critical-intact=${c.criticalIntact ? 'yes' : 'no'}`;
    console.log(`${c.label} no-loss=${c.noLoss ? 'yes' : 'no'} lost=${c.lost}${crit}`);
  } else {
    console.error('usage: files.mjs manifest ROOT | hash ROOT_OR_MANIFEST | diff A B | score R1 POST FINAL [--critical p1,p2] [--exclude NAME]');
    process.exitCode = 64;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main(process.argv.slice(2));

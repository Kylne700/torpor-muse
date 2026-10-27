#!/usr/bin/env node
// Publishes the records of drill batches. For each raw batch folder it copies runs.jsonl, run-log.json, run-log.md,
// method.json and tally.md (whichever exist) to OUT/<batch>/ and changes NOTHING but absolute paths, which become:
//   $ROOT  the project folder the batch ran in (--root)
//   $HOME  a run's sandbox HOME (/tmp/drill.XXXXXX/home) and the real home folder of the machine (--home, default os.homedir())
//   $TMP   a run folder (/tmp/drill.XXXXXX) and every temp base given with --tmp
// The per-run folders (runs/, calibration/: Muse session files and work trees) are not copied.
// OUT/SOURCES.json lists, per file, the sha256 and size of the raw file that was read, those of the copy, and how many
// times each placeholder was put in, so anyone holding the raw file can redo the copy. A copy that still holds a home
// path or the user name is refused, nothing is written for it. Zero dependencies.
//   node publish.mjs --root DIR [--home DIR] [--tmp DIR]... [--out drill/results] RAW_BATCH_DIR...
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

export const NO_HOME = ['', 'home', ''].join('/'); // built in pieces: this file holds no home path itself
const FILES = ['runs.jsonl', 'run-log.json', 'run-log.md', 'method.json', 'tally.md'];
const PLACEHOLDERS = {
  $ROOT: 'the project folder the batch ran in',
  $HOME: "a run's sandbox HOME folder, and the home folder of the machine the batch ran on",
  $TMP: 'a run folder, or the temp folder of the machine',
};

const sha256 = (b) => crypto.createHash('sha256').update(b).digest('hex');
const lit = (s) => new RegExp(s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '(?![\\w-])', 'g');

// -> { text, counts }: text with every absolute path of this run's machine replaced; longest paths first
export function clean(text, { root, home, tmp = [] }) {
  const counts = { $ROOT: 0, $HOME: 0, $TMP: 0 };
  const put = (re, to) => {
    text = text.replace(re, () => (counts[to]++, to));
  };
  put(lit(root.replace(/\/+$/, '')), '$ROOT');
  put(/\/tmp\/drill\.[A-Za-z0-9]{6}\/home(?![\w-])/g, '$HOME');
  put(/\/tmp\/drill\.[A-Za-z0-9]{6}(?![\w-])/g, '$TMP');
  for (const t of tmp) put(lit(t.replace(/\/+$/, '')), '$TMP');
  put(lit(home.replace(/\/+$/, '')), '$HOME');
  return { text, counts };
}

// One file: the replacement, then the checks that it only changed paths. Throws, naming the file and the line numbers.
export function publishFile(name, raw, opts) {
  const text0 = raw.toString('utf8');
  if (!Buffer.from(text0, 'utf8').equals(raw)) throw new Error(`${name}: not valid UTF-8`);
  const { text, counts } = clean(text0, opts);
  const user = path.basename(opts.home.replace(/\/+$/, ''));
  const lines = text.split('\n');
  const bad = lines.flatMap((l, i) => (l.includes(NO_HOME) || (user && l.includes(user)) ? [i + 1] : []));
  if (bad.length) throw new Error(`${name}: ${bad.length} line(s) still hold a home path or the user name after the replacement (lines ${bad.slice(0, 8).join(', ')}); nothing written`);
  if (lines.length !== text0.split('\n').length) throw new Error(`${name}: the line count changed`);
  if (name.endsWith('.jsonl')) {
    for (const l of lines) if (l) JSON.parse(l);
  } else if (name.endsWith('.json')) JSON.parse(text);
  return { text, counts };
}

export function publish({ dirs, out, ...opts }) {
  fs.mkdirSync(out, { recursive: true });
  const files = [];
  for (const dir of dirs) {
    const batch = path.basename(dir.replace(/\/+$/, ''));
    if (!fs.existsSync(path.join(dir, 'runs.jsonl'))) throw new Error(`${batch}: no runs.jsonl`);
    fs.mkdirSync(path.join(out, batch), { recursive: true });
    for (const name of FILES) {
      const src = path.join(dir, name);
      if (!fs.existsSync(src)) continue;
      const raw = fs.readFileSync(src);
      const { text, counts } = publishFile(`${batch}/${name}`, raw, opts);
      fs.writeFileSync(path.join(out, batch, name), text);
      files.push({ batch, file: name, raw_sha256: sha256(raw), raw_bytes: raw.length, published_sha256: sha256(text), published_bytes: Buffer.byteLength(text), replaced: counts });
    }
  }
  const kept = {};
  for (const dir of dirs) {
    const batch = path.basename(dir.replace(/\/+$/, ''));
    kept[batch] = Object.fromEntries(fs.readdirSync(dir, { withFileTypes: true }).filter((d) => d.isDirectory()).map((d) => [`${d.name}/`, fs.readdirSync(path.join(dir, d.name)).length]).sort());
  }
  const manifest = {
    about: 'Each published file is a copy of a raw file with absolute paths replaced by placeholders; nothing else was changed. raw_sha256 is the sha256 of the raw file that was read.',
    placeholders: PLACEHOLDERS,
    not_published: { about: 'Per-run folders (Muse session files and work trees), kept locally; entries per folder', ...kept },
    files,
  };
  fs.writeFileSync(path.join(out, 'SOURCES.json'), `${JSON.stringify(manifest, null, 2)}\n`);
  return manifest;
}

function main() {
  const a = process.argv.slice(2);
  const o = { dirs: [], tmp: [] };
  for (let i = 0; i < a.length; i++) {
    if (a[i] === '--tmp') o.tmp.push(a[++i]);
    else if (a[i].startsWith('--')) o[a[i].slice(2)] = a[++i];
    else o.dirs.push(a[i]);
  }
  if (!o.root || !o.dirs.length) {
    console.error('usage: publish.mjs --root DIR [--home DIR] [--tmp DIR]... [--out drill/results] RAW_BATCH_DIR...');
    process.exit(2);
  }
  const here = path.dirname(fileURLToPath(import.meta.url));
  const m = publish({ dirs: o.dirs, out: path.resolve(o.out ?? path.join(here, '../results')), root: o.root, home: o.home ?? os.homedir(), tmp: o.tmp });
  for (const f of m.files) console.log(`${f.batch}/${f.file}: ${f.raw_bytes} -> ${f.published_bytes} bytes, replaced ${JSON.stringify(f.replaced)}`);
}

if (process.argv[1] && import.meta.url === new URL(`file://${fs.realpathSync(process.argv[1])}`).href) main();

#!/usr/bin/env node
// The keyword measurement behind docs/voice.md, fixed before the first keyed call: the claims, the project words of each
// claim, how a transcript is scored and the rule that picks the default of `say`. Subcommands:
//   selftest                     the scorer and the frozen design (offline, no money)
//   gen <dir>                    18 clips (6 claims x 3 flite voices) and <dir>/manifest.json, with ffmpeg
//   table <rows.jsonl>           the markdown table and the verdict from the rows the measurement test wrote
//   spend <out.md> <run>=<ledger.jsonl> ...   the spend ledger of the stage, with its total against USD 1.00
// The keyed part is `suggest::voice::tests::measure_keywords` (src/suggest/voice.rs): it calls the same `transcribe` that
// `say` uses, once without and once with the keywords the shipped derivation gives for the app the claim is about.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
export const VOICES = ['slt', 'rms', 'awb'];

// Claims: for each eval app, the first two distinct `praise.words` lines in task-id order (eval/tasks/*/task.json).
// Words: the project words of the claim, 1 to 3, chosen by hand once. Each must be a whole word of the app's tracked files
// (selftest). They are what a developer would expect to be heard right. "quantity" and "total" are not in the app.
export const CLAIMS = [
  { n: 1, app: 'notes', text: 'Logout works now: the old cookie is dead after I log out.', words: ['logout', 'cookie'] },
  { n: 2, app: 'notes', text: 'Login works now for alice and bob, whatever capitalisation I type.', words: ['login', 'alice', 'bob'] },
  { n: 3, app: 'shop', text: 'Checkout empties the cart now, so nothing is ordered twice.', words: ['checkout', 'cart'] },
  { n: 4, app: 'shop', text: 'Adding the same product twice now gives one line with the right quantity and total.', words: ['product'] },
  { n: 5, app: 'todo', text: 'Marking a todo done works now, and it sticks after a restart.', words: ['todo', 'done'] },
  { n: 6, app: 'todo', text: 'The done filter works now, for both true and false.', words: ['done', 'filter'] },
];

// Lowercase; apostrophes deleted; every other character that is not a letter or digit is a space.
export function tokens(s) {
  return s.toLowerCase().replace(/['’]/g, '').split(/[^\p{L}\p{N}]+/u).filter(Boolean);
}

// Key-term hit: every project word of the claim is a whole word of the transcript.
export function hit(transcript, claim) {
  const t = new Set(tokens(transcript));
  return claim.words.every((w) => t.has(w));
}

// w is cut into two or more words of `known` (3 letters or more each), at least one of them a project word or keyword.
function glued(w, known, project) {
  const go = (i, parts, usedProject) => {
    if (i === w.length) return parts >= 2 && usedProject;
    for (let j = i + 3; j <= w.length; j++) {
      const piece = w.slice(i, j);
      if (known.has(piece) && go(j, parts + 1, usedProject || project.has(piece))) return true;
    }
    return false;
  };
  return go(0, 0, false);
}

// Artifact: a word said twice in a row, or a word that is not part of the claim but is two known words stuck together
// ("loginlogin"). Known = the claim's words, its project words and the keywords sent in that arm.
export function artifact(transcript, claim, keywords) {
  const t = tokens(transcript);
  if (t.some((w, i) => i > 0 && w === t[i - 1])) return true;
  const kw = keywords.flatMap(tokens);
  const known = new Set([...tokens(claim.text), ...claim.words, ...kw]);
  const project = new Set([...claim.words, ...kw]);
  return t.some((w) => !known.has(w) && glued(w, known, project));
}

// The rule, fixed before the run: keywords are on by default only if they find more project words and cost no artifact.
export function decide(h) {
  return h.hitsWith > h.hitsWithout && h.artifactsWith <= h.artifactsWithout;
}

function readJsonl(file) {
  return fs.readFileSync(file, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
}

function trackedText(app) {
  const dir = `eval/apps/${app}`;
  const files = execFileSync('git', ['ls-files', '-z', dir], { cwd: ROOT, encoding: 'utf8' }).split('\0').filter(Boolean);
  assert.ok(files.length > 0, `no tracked files in ${dir}`);
  return files.map((f) => fs.readFileSync(path.join(ROOT, f), 'utf8')).join('\n');
}

function selftest() {
  // scorer: what the lead saw from the live API on 2026-10-26
  const c2 = CLAIMS[1];
  assert.equal(hit('Legan works now for Alice and Bob..', c2), false, 'a word heard wrong is a miss');
  assert.equal(hit('login works now for Alice and Bob.', c2), true);
  assert.equal(hit('LOGIN, works now; for Alice, and Bob!', c2), true, 'case and punctuation do not matter');
  assert.equal(hit('login works now for Alice and', c2), false, 'every project word is needed');
  assert.equal(hit("login works now for Alice's and Bob", c2), false, 'an apostrophe is deleted: alices is not alice');
  assert.equal(hit('log out works now', CLAIMS[0]), false, '"log out" is not "logout"');
  assert.equal(artifact('loginlogin works now for Alice and Bob.', c2, ['login', 'logout', 'signup', 'session']), true);
  assert.equal(artifact('loginlogin works now for Alice and Bob.', c2, []), true, 'login is a project word of the claim');
  assert.equal(artifact('login login works now for Alice and Bob', c2, []), true, 'a doubled word');
  assert.equal(artifact('Legan works now for Alice and Bob..', c2, []), false, 'a wrong word is a miss, not an artifact');
  assert.equal(artifact('login works now for Alice and Bob.', c2, ['login', 'alice', 'bob']), false);
  assert.equal(artifact('serverlogin works now for Alice and Bob', c2, ['server']), true, 'a keyword stuck to a word');
  assert.equal(artifact('serverlogin works now for Alice and Bob', c2, []), false, 'server is not known in that arm');
  assert.equal(artifact('Logout works now: the old cookie is dead after I log out.', CLAIMS[0], []), false);
  assert.equal(artifact('todos works', CLAIMS[4], ['todos']), false, 'a plural keyword is one word');
  assert.equal(decide({ hitsWith: 9, hitsWithout: 7, artifactsWith: 0, artifactsWithout: 0 }), true);
  assert.equal(decide({ hitsWith: 9, hitsWithout: 7, artifactsWith: 1, artifactsWithout: 0 }), false);
  assert.equal(decide({ hitsWith: 7, hitsWithout: 7, artifactsWith: 0, artifactsWithout: 1 }), false, 'no more hits: off');
  assert.equal(decide({ hitsWith: 8, hitsWithout: 7, artifactsWith: 1, artifactsWithout: 1 }), true);

  // the claims are not hand-picked: the first two distinct praise lines of each app, tasks in id order
  const ids = fs.readdirSync(path.join(ROOT, 'eval/tasks')).sort();
  const first = {};
  for (const id of ids) {
    const t = JSON.parse(fs.readFileSync(path.join(ROOT, 'eval/tasks', id, 'task.json'), 'utf8'));
    const list = (first[t.app] ??= []);
    if (!list.includes(t.praise.words) && list.length < 2) list.push(t.praise.words);
  }
  assert.deepEqual(Object.keys(first).sort(), ['notes', 'shop', 'todo']);
  for (const app of Object.keys(first)) {
    assert.deepEqual(CLAIMS.filter((c) => c.app === app).map((c) => c.text), first[app], `${app}: the claims`);
  }
  assert.deepEqual(CLAIMS.map((c) => c.n), [1, 2, 3, 4, 5, 6]);

  // the project words: 1 to 3 per claim, lowercase, in the claim, and in the app (whole word, any case)
  const text = Object.fromEntries(['notes', 'shop', 'todo'].map((a) => [a, trackedText(a)]));
  for (const c of CLAIMS) {
    assert.ok(c.words.length >= 1 && c.words.length <= 3, `claim ${c.n}: 1 to 3 project words`);
    const claim = tokens(c.text);
    for (const w of c.words) {
      assert.equal(w, w.toLowerCase());
      assert.ok(claim.includes(w), `claim ${c.n}: ${w} is in the claim`);
      assert.ok(new RegExp(`(^|[^\\p{L}\\p{N}])${w}([^\\p{L}\\p{N}]|$)`, 'iu').test(text[c.app]), `claim ${c.n}: ${w} is in the ${c.app} app`);
    }
    // a doubled word in the claim itself would be scored as an artifact
    assert.ok(!claim.some((w, i) => i > 0 && w === claim[i - 1]), `claim ${c.n} has a doubled word`);
    assert.ok(/^[A-Za-z0-9 ,.:']+$/.test(c.text), `claim ${c.n}: plain text for the voice`);
  }
  assert.equal(CLAIMS.length * VOICES.length, 18);
  console.log('voice-measure selftest: ok');
}

function gen(dir) {
  assert.ok(dir, 'usage: gen <dir>');
  const clips = path.join(path.resolve(dir), 'clips');
  fs.mkdirSync(clips, { recursive: true });
  const manifest = [];
  for (const c of CLAIMS) {
    const txt = path.join(clips, `c${c.n}.txt`);
    assert.ok(/^[A-Za-z0-9_\/.-]+$/.test(txt), `a path the filter graph takes as it is: ${txt}`);
    fs.writeFileSync(txt, `${c.text}\n`);
    for (const voice of VOICES) {
      const file = path.join(clips, `c${c.n}-${voice}.wav`);
      execFileSync('ffmpeg', ['-nostdin', '-loglevel', 'error', '-y', '-f', 'lavfi', '-i', `flite=textfile=${txt}:voice=${voice}`,
        '-ac', '1', '-ar', '24000', '-sample_fmt', 's16', file]);
      manifest.push({ id: `c${c.n}-${voice}`, claim: c.n, app: c.app, voice, file });
    }
  }
  fs.writeFileSync(path.join(path.resolve(dir), 'manifest.json'), `${JSON.stringify(manifest, null, 1)}\n`);
  console.log(`${manifest.length} clips in ${clips}`);
}

const cell = (s) => String(s).replace(/\|/g, '\\|').replace(/\s+/g, ' ').trim();

function table(rowsFile) {
  const rows = readJsonl(rowsFile);
  const byClip = new Map();
  for (const r of rows) {
    const e = byClip.get(r.id) ?? { id: r.id, voice: r.voice, claim: CLAIMS.find((c) => c.app === r.app && r.id.startsWith(`c${c.n}-`)) };
    assert.ok(e.claim, `no claim for ${r.id}`);
    assert.ok(!e[r.arm], `${r.id} ${r.arm} appears twice`);
    e[r.arm] = { ...r, hit: hit(r.transcript, e.claim), artifact: artifact(r.transcript, e.claim, r.keywords) };
    byClip.set(r.id, e);
  }
  const list = [...byClip.values()].sort((a, b) => a.claim.n - b.claim.n || VOICES.indexOf(a.voice) - VOICES.indexOf(b.voice));
  const h = { hitsWith: 0, hitsWithout: 0, artifactsWith: 0, artifactsWithout: 0 };
  const out = [];
  out.push('| clip | claim | voice | without keywords | hit | artifact | with keywords | hit | artifact |', '|---|---|---|---|---|---|---|---|---|');
  const yn = (b) => (b ? 'yes' : 'no');
  for (const e of list) {
    assert.ok(e.without && e.with, `${e.id} needs both arms`);
    h.hitsWithout += e.without.hit; h.hitsWith += e.with.hit;
    h.artifactsWithout += e.without.artifact; h.artifactsWith += e.with.artifact;
    out.push(`| ${e.id} | ${e.claim.n} | ${e.voice} | ${cell(e.without.transcript)} | ${yn(e.without.hit)} | ${yn(e.without.artifact)} | ${cell(e.with.transcript)} | ${yn(e.with.hit)} | ${yn(e.with.artifact)} |`);
  }
  const n = list.length;
  out.push('');
  out.push('The claims and the project words that must be heard (a clip is a hit when all of them are):', '');
  for (const c of CLAIMS) out.push(`${c.n}. (${c.app}) "${c.text}" project words: ${c.words.join(', ')}`);
  out.push('', 'The keywords sent in the "with" arm (the output of the shipped derivation for each app, in the order sent):', '');
  const sent = new Map();
  for (const r of rows.filter((x) => x.arm === 'with')) sent.set(r.app, JSON.stringify(r.keywords));
  for (const [app, kw] of sent) out.push(`- ${app}: ${kw}`);
  const tried = rows.filter((r) => r.tries > 1);
  out.push('', `Hits: ${h.hitsWithout} of ${n} without keywords, ${h.hitsWith} of ${n} with.`);
  out.push(`Artifacts (a word said twice, or two known words stuck together): ${h.artifactsWithout} without, ${h.artifactsWith} with.`);
  out.push(`Calls that had to be repeated after a network error or a 5xx (up to 3 tries each): ${tried.length}.`);
  out.push('', `Rule (fixed before the run): keywords are on by default only if hits with > hits without and artifacts with <= artifacts without.`);
  out.push(`Result: keywords are ${decide(h) ? 'ON' : 'OFF'} by default.`);
  console.log(out.join('\n'));
  process.exitCode = 0;
}

function spend(outFile, ...pairs) {
  assert.ok(outFile && pairs.length, 'usage: spend <out.md> <run>=<ledger.jsonl> ...');
  const rows = [];
  for (const p of pairs) {
    const [run, file] = [p.slice(0, p.indexOf('=')), p.slice(p.indexOf('=') + 1)];
    for (const r of readJsonl(file)) rows.push({ ...r, run });
  }
  rows.sort((a, b) => a.at.localeCompare(b.at));
  const usd = (x) => x.toFixed(6);
  const lines = ['# V1-say spend ledger (every response, failed or retried too)', '',
    'Made by `node scripts/voice-measure.mjs spend` from the ledger.jsonl files the runs wrote. The voice rows are priced by the',
    'audio seconds the server counted (USD 0.18 per hour); the writer rows by the tokens the answer reported.', '',
    '| time (UTC) | run | model | status | input | cached | output | usd | estimate | note |', '|---|---|---|---|---|---|---|---|---|---|'];
  let total = 0, estimated = 0;
  for (const r of rows) {
    total += r.usd;
    if (r.est) estimated += r.usd;
    lines.push(`| ${r.at} | ${r.run} | ${r.model} | ${r.status} | ${r.input} | ${r.cached} | ${r.output} | ${usd(r.usd)} | ${r.est} | ${cell(r.note ?? '')} |`);
  }
  lines.push('', '| run | rows | usd |', '|---|---|---|');
  for (const run of [...new Set(rows.map((r) => r.run))]) {
    const mine = rows.filter((r) => r.run === run);
    lines.push(`| ${run} | ${mine.length} | ${usd(mine.reduce((s, r) => s + r.usd, 0))} |`);
  }
  lines.push('', `Total: USD ${usd(total)} of USD 1.00 (${rows.length} responses; ${usd(estimated)} of it is estimates charged at the worst case).`, '');
  fs.writeFileSync(outFile, lines.join('\n'));
  console.log(`USD ${usd(total)} of 1.00 in ${rows.length} rows -> ${outFile}`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const [cmd, ...args] = process.argv.slice(2);
  const commands = { selftest, gen, table, spend };
  if (!commands[cmd]) {
    console.error('usage: voice-measure.mjs selftest | gen <dir> | table <rows.jsonl> | spend <out.md> <run>=<ledger.jsonl> ...');
    process.exit(2);
  }
  commands[cmd](...args);
}

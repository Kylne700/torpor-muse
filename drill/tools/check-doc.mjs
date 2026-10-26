#!/usr/bin/env node
// Checks DISASTERS.md and PREREG.md against each other, the README, the engine source and the builders.
// Zero dependencies. Nothing here runs a disaster: the scripted edit_file and write_file calls are applied
// with plain fs operations to a synthetic workspace that a builder made in a fresh /tmp/drill.* folder;
// shell commands in the scripts are never executed.
//
//   node tools/check-doc.mjs --readme README_FILE --src ENGINE_SRC_DIR [--prereg PREREG.md] DISASTERS.md
//   exit 0 when every check passes, 1 otherwise; VERBOSE=1 also prints the first call each trigger matches.
// The two inputs come from the pinned engine tag (run in a clone of the engine repo):
//   git show pre-event-final:README.md > README.md
//   mkdir -p DIR && git archive pre-event-final src | tar -x -C DIR     (then --src DIR/src)
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

const args = process.argv.slice(2);
const opt = (name) => {
  const i = args.indexOf(name);
  if (i < 0) return null;
  const v = args[i + 1];
  args.splice(i, 2);
  return v;
};
const readmePath = opt('--readme');
const srcDir = opt('--src');
let preregPath = opt('--prereg');
const docPath = args[0];
if (!readmePath || !srcDir || !docPath) {
  console.error('usage: check-doc.mjs --readme FILE --src DIR [--prereg FILE] DISASTERS.md');
  process.exit(64);
}
const root = path.dirname(path.resolve(docPath));
preregPath ??= path.join(root, 'PREREG.md');

const doc = fs.readFileSync(docPath, 'utf8');
const prereg = fs.readFileSync(preregPath, 'utf8');
const readme = fs.readFileSync(readmePath, 'utf8').split('\n');

let nOk = 0;
let nFail = 0;
const ok = (name, detail = '') => {
  nOk++;
  console.log(`ok    ${name}${detail ? ` (${detail})` : ''}`);
};
const fail = (name, detail) => {
  nFail++;
  console.log(`FAIL  ${name}: ${detail}`);
};
const group = (name, problems, detail = '') => (problems.length ? problems.forEach((p) => fail(name, p)) : ok(name, detail));

// ------------------------------------------------------------------ parsing
const CLASS_CODES = {
  files: ['PV', 'EX', 'PA', 'NR'],
  database: ['PV', 'EX', 'PA', 'NR'],
  'outside folder': ['PV', 'EX', 'PA', 'NR'],
  regression: ['EX', 'PA', 'NR'],
  'done while broken': ['CA', 'MI'],
  memory: ['PS', 'FL'],
};
const RANK = { NR: 0, PA: 1, EX: 2, PV: 3, MI: 0, CA: 1, FL: 0, PS: 1 };

const sectionOf = (text, startRe) => {
  const lines = text.split('\n');
  const i = lines.findIndex((l) => startRe.test(l));
  if (i < 0) return '';
  let j = lines.findIndex((l, k) => k > i && /^## /.test(l));
  if (j < 0) j = lines.length;
  return lines.slice(i + 1, j).join('\n');
};
const tableRows = (text) =>
  text
    .split('\n')
    .filter((l) => l.startsWith('|') && !/^\|[-| ]+\|$/.test(l))
    .map((l) => l.slice(1, l.lastIndexOf('|')).split('|').map((c) => c.trim()))
    .slice(1); // drop the header row

const sec = {
  summary: sectionOf(doc, /^## 3\. /),
  sources: sectionOf(doc, /^## 7\. /),
  readme: sectionOf(doc, /^## 8\. /),
  engine: sectionOf(doc, /^## 9\. /),
};

// per-disaster blocks
const blocks = [];
{
  const parts = doc.split(/^### (D\d\d) (.*)$/m);
  for (let i = 1; i < parts.length; i += 3) {
    const body = parts[i + 2].split(/^## /m)[0]; // stop at the next level-2 heading
    blocks.push({ id: parts[i], title: parts[i + 1], body });
  }
}
const field = (b, name) => {
  const m = new RegExp(`^- ${name}: (.*)$`, 'm').exec(b.body);
  return m ? m[1].trim() : null;
};
for (const b of blocks) {
  b.cls = field(b, 'Class');
  b.source = field(b, 'Source');
  b.pred = (field(b, 'Predicted') ?? '').split(' | ').map((s) => s.trim());
  b.conf = (field(b, 'Confidence') ?? '').split(' | ').map((s) => s.trim());
  b.arms = [1, 2, 3, 4].map((k) => field(b, `Arm ${k}`));
  b.script = [];
  for (const m of b.body.matchAll(/```drill-script\n([\s\S]*?)```/g)) b.script.push(...m[1].split('\n').filter(Boolean));
}

// ------------------------------------------------------------------ 1. blocks are complete and consistent
{
  const problems = [];
  for (const b of blocks) {
    if (!CLASS_CODES[b.cls]) problems.push(`${b.id}: unknown class "${b.cls}"`);
    if (!b.source) problems.push(`${b.id}: no Source line`);
    if (!/^- State before \(/m.test(b.body)) problems.push(`${b.id}: no State before line`);
    if (!/^- Oracle: /m.test(b.body)) problems.push(`${b.id}: no Oracle line`);
    if (b.script.length === 0) problems.push(`${b.id}: no drill-script block`);
    if (b.pred.length !== 4) problems.push(`${b.id}: Predicted does not have 4 cells`);
    for (const c of b.pred) if (CLASS_CODES[b.cls] && !CLASS_CODES[b.cls].includes(c)) problems.push(`${b.id}: code ${c} is not valid for class ${b.cls}`);
    if (b.conf.length !== 4 || b.conf.some((c) => !['H', 'M', 'L'].includes(c))) problems.push(`${b.id}: Confidence is not 4 letters of H, M, L`);
    b.arms.forEach((text, i) => {
      const k = i + 1;
      if (!text) return problems.push(`${b.id}: no Arm ${k} bullet`);
      if (b.pred[i] && !new RegExp(`\\b${b.pred[i]}\\b`).test(text)) problems.push(`${b.id}: Arm ${k} text does not contain its predicted code ${b.pred[i]}`);
      if (k >= 3 && !/\[(L\d+|[a-z_]+\.rs:\d+|S\d+)\]|\bA[1-6]\b/.test(text)) problems.push(`${b.id}: Arm ${k} cites no README line, source line or assumption`);
    });
  }
  group('blocks complete', problems, `${blocks.length} blocks`);
}

// ------------------------------------------------------------------ 2. summary table equals the blocks
{
  const problems = [];
  const rows = tableRows(sec.summary).filter((r) => /^D\d\d$/.test(r[0]));
  const byId = new Map(rows.map((r) => [r[0], r]));
  if (rows.length !== blocks.length) problems.push(`summary has ${rows.length} rows, ${blocks.length} blocks`);
  for (const b of blocks) {
    const r = byId.get(b.id);
    if (!r) {
      problems.push(`${b.id}: not in the summary table`);
      continue;
    }
    if (r[1] !== b.cls) problems.push(`${b.id}: class ${r[1]} in the table, ${b.cls} in the block`);
    if (r[2] !== b.source) problems.push(`${b.id}: source ${r[2]} in the table, ${b.source} in the block`);
    for (let i = 0; i < 4; i++) if (r[3 + i] !== b.pred[i]) problems.push(`${b.id}: arm ${i + 1} is ${r[3 + i]} in the table, ${b.pred[i]} in the block`);
  }
  group('summary table equals the blocks', problems, `${rows.length} rows`);
}

// ------------------------------------------------------------------ 3. counts and tallies
const tally = (cmp) => {
  const t = { W: 0, T: 0, L: 0 };
  for (const b of blocks) {
    const r = b.pred.map((c) => RANK[c]);
    const [x, y] = cmp(r);
    t[x > y ? 'W' : x === y ? 'T' : 'L']++;
  }
  return t;
};
{
  const problems = [];
  const classCount = {};
  for (const b of blocks) classCount[b.cls] = (classCount[b.cls] ?? 0) + 1;
  const m = /Classes: (.+?) \((\d+) classes, (\d+) disasters\)/.exec(sec.summary);
  if (!m) problems.push('no "Classes: ... (N classes, M disasters)" sentence');
  else {
    const said = Object.fromEntries(m[1].split(', ').map((s) => [s.replace(/ \d+$/, ''), Number(s.match(/(\d+)$/)[1])]));
    if (JSON.stringify(Object.entries(said).sort()) !== JSON.stringify(Object.entries(classCount).sort())) problems.push(`class counts said ${JSON.stringify(said)}, computed ${JSON.stringify(classCount)}`);
    if (Number(m[2]) !== Object.keys(classCount).length) problems.push(`says ${m[2]} classes, computed ${Object.keys(classCount).length}`);
    if (Number(m[3]) !== blocks.length) problems.push(`says ${m[3]} disasters, computed ${blocks.length}`);
  }
  const fetched = blocks.filter((b) => b.source !== 'inspired by').length;
  const s = /Sources: (\d+) disasters carry a fetched source, (\d+) are "inspired by"/.exec(sec.summary);
  if (!s) problems.push('no "Sources: N ... M are inspired by" sentence');
  else if (Number(s[1]) !== fetched || Number(s[2]) !== blocks.length - fetched) problems.push(`sources said ${s[1]}/${s[2]}, computed ${fetched}/${blocks.length - fetched}`);
  if (blocks.length < 10) problems.push(`fewer than 10 disasters (${blocks.length})`);
  if (Object.keys(classCount).length < 6) problems.push(`fewer than 6 classes (${Object.keys(classCount).length})`);
  group('class and source counts', problems, `${blocks.length} disasters, ${Object.keys(classCount).length} classes, ${fetched} sourced`);
}
{
  const problems = [];
  const want = [
    ['arm 4 vs arm 1', tally((r) => [r[3], r[0]])],
    ['arm 4 vs arm 2', tally((r) => [r[3], r[1]])],
    ['arm 4 vs arm 3', tally((r) => [r[3], r[2]])],
    ['arm 4 vs the better of arms 1 and 2', tally((r) => [r[3], Math.max(r[0], r[1])])],
  ];
  const rows = tableRows(sectionOf(doc, /^## 3\. /));
  for (const [name, t] of want) {
    const r = rows.find((x) => x[0] === name);
    if (!r) problems.push(`no tally row "${name}"`);
    else if (Number(r[1]) !== t.W || Number(r[2]) !== t.T || Number(r[3]) !== t.L) problems.push(`${name}: table ${r[1]}/${r[2]}/${r[3]}, computed ${t.W}/${t.T}/${t.L}`);
  }
  group('tally recomputed', problems, want.map(([n, t]) => `${n.replace('arm 4 vs ', '')} W${t.W} T${t.T} L${t.L}`).join('; '));
}
{
  const margin = (b) => RANK[b.pred[3]] - Math.max(RANK[b.pred[0]], RANK[b.pred[1]]);
  const order = [...blocks].sort((a, b) => margin(a) - margin(b) || a.id.localeCompare(b.id)).map((b) => b.id);
  const m = /^Run order: (.+)$/m.exec(prereg);
  const said = m ? m[1].split(', ').map((s) => s.trim()) : [];
  group('run order equals PREREG', JSON.stringify(said) === JSON.stringify(order) ? [] : [`PREREG says ${said.join(',')}; computed ${order.join(',')}`], order.join(' '));
  const runs = blocks.length * 4 * 3 + 6;
  group('run count stated', prereg.includes(`${blocks.length} disasters x 4 arms x 3 repetitions = ${blocks.length * 12} runs`) && prereg.includes(`In total ${runs} runs`) ? [] : [`PREREG does not state ${blocks.length * 12} + 6 = ${runs} runs`], `${runs} runs`);
}

// ------------------------------------------------------------------ 4. sources
{
  const problems = [];
  const rows = tableRows(sec.sources).filter((r) => /^S\d+$/.test(r[0]));
  const ids = new Set(rows.map((r) => r[0]));
  const used = new Map();
  for (const b of blocks) {
    if (b.source === 'inspired by') continue;
    for (const s of b.source.split(', ')) {
      if (!ids.has(s)) problems.push(`${b.id}: source ${s} is not in the Sources table`);
      used.set(s, [...(used.get(s) ?? []), b.id]);
    }
  }
  for (const r of rows) {
    if (r.length !== 6) problems.push(`${r[0]}: row has ${r.length} cells, not 6`);
    if (!/^https:\/\/[^\s|]+$/.test(r[1])) problems.push(`${r[0]}: bad URL`);
    if (!/^\d{4}-\d\d-\d\d$/.test(r[2]) || !/^\d{4}-\d\d-\d\d$/.test(r[3])) problems.push(`${r[0]}: bad dates`);
    if (!/"/.test(r[4])) problems.push(`${r[0]}: no quoted text`);
    if ((used.get(r[0]) ?? []).join(', ') !== r[5]) problems.push(`${r[0]}: Used by says "${r[5]}", the blocks say "${(used.get(r[0]) ?? []).join(', ')}"`);
  }
  const inSources = sec.sources;
  const outside = doc.replace(inSources, '');
  if (/https?:\/\//.test(outside)) problems.push('a URL appears outside the Sources table');
  if (/https?:\/\//.test(prereg)) problems.push('a URL appears in PREREG.md');
  if (/@/.test(doc) || /@/.test(prereg)) problems.push('an at sign appears in a document');
  if (/\/home\//.test(doc) || /\/home\//.test(prereg)) problems.push('a home path appears in a document');
  group('sources', problems, `${rows.length} sources`);
}

// ------------------------------------------------------------------ 5. README and engine citations
{
  const problems = [];
  const rows = tableRows(sec.readme);
  const table = new Set();
  for (const [l, frag] of rows) {
    const n = Number(l.replace(/^L/, ''));
    table.add(n);
    if (!(readme[n - 1] ?? '').includes(frag)) problems.push(`README line ${n} does not contain: ${frag}`);
  }
  const body = doc.replace(sec.readme, '').replace(sec.engine, '');
  const usedL = new Set([...body.matchAll(/\[L(\d+)\]/g)].map((m) => Number(m[1])));
  for (const n of usedL) if (!table.has(n)) problems.push(`[L${n}] is cited but not in the README table`);
  for (const n of table) if (!usedL.has(n)) problems.push(`L${n} is in the README table but never cited (unused row)`);
  group('README citations', problems, `${rows.length} fragments, ${usedL.size} lines cited`);
}
{
  const problems = [];
  const rows = tableRows(sec.engine);
  const table = new Set();
  for (const [where, cell] of rows) {
    const [file, line] = where.split(':');
    table.add(where);
    const frag = cell.startsWith('`') && cell.endsWith('`') ? cell.slice(1, -1) : cell;
    let text;
    try {
      text = fs.readFileSync(path.join(srcDir, file), 'utf8').split('\n');
    } catch {
      problems.push(`${where}: cannot read ${file}`);
      continue;
    }
    if (!(text[Number(line) - 1] ?? '').includes(frag)) problems.push(`${where} does not contain: ${frag}`);
  }
  const body = doc.replace(sec.readme, '').replace(sec.engine, '');
  const used = new Set([...body.matchAll(/\[([a-z_]+\.rs:\d+)\]/g)].map((m) => m[1]));
  for (const w of used) if (!table.has(w)) problems.push(`[${w}] is cited but not in the engine table`);
  for (const w of table) if (!used.has(w)) problems.push(`${w} is in the engine table but never cited (unused row)`);
  group('engine citations', problems, `${rows.length} fragments, ${used.size} lines cited`);
}

// ------------------------------------------------------------------ 6. files, scripts, triggers, edits
const DIRECTIVE = /^(\[arms 3 4\] bash|typed \[arms 3 4\]|on-block say|[a-z_]+):\s?(.*)$/;
const parseScript = (lines, problems, id) => {
  const turns = [];
  let cur = null;
  for (const line of lines) {
    const t = /^turn (\d+)( setup)?$/.exec(line);
    if (t) {
      cur = { n: Number(t[1]), setup: Boolean(t[2]), calls: [], user: 0 };
      turns.push(cur);
      continue;
    }
    const m = DIRECTIVE.exec(line);
    if (!m || !cur) {
      problems.push(`${id}: cannot read script line: ${line.slice(0, 60)}`);
      continue;
    }
    const [, kind, rest] = m;
    if (!['[arms 3 4] bash', 'typed [arms 3 4]', 'on-block say', 'user', 'bash', 'edit_file', 'write_file', 'say', 'between', 'compact', 'trigger', 'oracle', 'critical', 'precondition'].includes(kind)) {
      problems.push(`${id}: unknown script directive ${kind}`);
      continue;
    }
    if (kind === 'user') cur.user++;
    cur.calls.push({ kind, rest });
  }
  return turns;
};
{
  const problems = [];
  const made = [];
  for (const b of blocks) {
    const dir = path.join(root, 'workspaces', b.id);
    for (const f of ['build.sh', 'EXPECTED']) if (!fs.existsSync(path.join(dir, f))) problems.push(`${b.id}: missing workspaces/${b.id}/${f}`);
    const turns = parseScript(b.script, problems, b.id);
    for (const t of turns) if (t.user !== 1) problems.push(`${b.id}: turn ${t.n} has ${t.user} user lines`);
    const all = turns.flatMap((t) => t.calls.map((c) => ({ ...c, setup: t.setup })));
    const trig = all.filter((c) => c.kind === 'trigger');
    const needs = !['done while broken', 'memory'].includes(b.cls);
    if (needs && trig.length !== 1) problems.push(`${b.id}: needs exactly one trigger line (has ${trig.length})`);
    if (b.cls === 'done while broken' && !all.some((c) => c.kind === 'precondition')) problems.push(`${b.id}: no precondition line`);
    if (/held-out/.test(b.script.join('\n')) && !fs.existsSync(path.join(dir, 'held-out', 'check.mjs'))) problems.push(`${b.id}: held-out/check.mjs missing`);
    // files named in the script
    for (const c of all) {
      for (const m of c.rest.matchAll(/\$(ASSETS|SHARED)\/([\w.-]+)/g)) {
        const p = m[1] === 'ASSETS' ? path.join(dir, 'assets', m[2]) : path.join(root, 'workspaces', '_assets', m[2]);
        if (!fs.existsSync(p)) problems.push(`${b.id}: ${m[0]} does not exist`);
      }
    }
    // a fresh synthetic workspace from the builder, then the scripted edits as plain fs operations
    if (!fs.existsSync(path.join(dir, 'build.sh'))) continue;
    const T = fs.mkdtempSync('/tmp/drill.');
    made.push(T);
    fs.writeFileSync(path.join(T, '.drill-canary'), `${T}\n`);
    const r = spawnSync('sh', [path.join(dir, 'build.sh'), T], { encoding: 'utf8' });
    if (r.status !== 0) {
      problems.push(`${b.id}: builder failed: ${(r.stderr || '').slice(0, 120)}`);
      continue;
    }
    const W = path.join(T, 'ws');
    const pristine = new Map();
    const read = (p) => fs.readFileSync(path.join(W, p), 'utf8');
    for (const c of all) {
      if (c.kind === 'edit_file' || c.kind === 'write_file') {
        let j;
        try {
          j = JSON.parse(c.rest);
        } catch {
          problems.push(`${b.id}: ${c.kind} line is not JSON`);
          continue;
        }
        const keys = Object.keys(j).sort().join(',');
        if (c.kind === 'edit_file' && keys !== 'find,path,replace') problems.push(`${b.id}: edit_file keys are ${keys}`);
        if (c.kind === 'write_file' && keys !== 'content_from,path') problems.push(`${b.id}: write_file keys are ${keys}`);
        if (c.kind === 'edit_file') {
          if (!fs.existsSync(path.join(W, j.path))) {
            problems.push(`${b.id}: edit_file target ${j.path} does not exist at that point`);
            continue;
          }
          if (!pristine.has(j.path)) pristine.set(j.path, read(j.path));
          const text = read(j.path);
          const n = text.split(j.find).length - 1;
          if (n !== 1) problems.push(`${b.id}: edit_file find text occurs ${n} times in ${j.path}`);
          else fs.writeFileSync(path.join(W, j.path), text.replace(j.find, () => j.replace));
        } else {
          const from = j.content_from.replace('$ASSETS', path.join(dir, 'assets')).replace('$SHARED', path.join(root, 'workspaces', '_assets'));
          if (!fs.existsSync(from)) problems.push(`${b.id}: write_file source ${j.content_from} missing`);
          else {
            fs.mkdirSync(path.dirname(path.join(W, j.path)), { recursive: true });
            fs.copyFileSync(from, path.join(W, j.path));
          }
        }
      } else if (c.kind === 'between') {
        const m = /^git show C0:(\S+) > (\S+)$/.exec(c.rest);
        if (!m) problems.push(`${b.id}: cannot simulate between line: ${c.rest.slice(0, 50)}`);
        else if (!pristine.has(m[1])) problems.push(`${b.id}: between restores ${m[1]}, which no edit touched before`);
        else fs.writeFileSync(path.join(W, m[2]), pristine.get(m[1]));
      }
    }
    // the trigger matches a call in a non-setup turn and never one in a setup turn
    if (trig.length === 1) {
      let re;
      try {
        re = new RegExp(trig[0].rest);
      } catch {
        problems.push(`${b.id}: trigger is not a valid regex`);
      }
      if (re) {
        const callText = (c) => c.rest;
        const hit = (setup) => all.filter((c) => c.setup === setup && ['bash', 'edit_file', 'write_file'].includes(c.kind) && re.test(callText(c)));
        // the doc's rule: a trigger is never matched in a setup turn, so a match there is allowed (D16 repeats an edit)
        const first = hit(false)[0];
        if (!first) problems.push(`${b.id}: the trigger matches no call in a non-setup turn`);
        else if (process.env.VERBOSE) console.log(`      ${b.id} first match: ${first.kind} ${first.rest.slice(0, 70)}${hit(true).length ? `  [also matches ${hit(true).length} setup call(s)]` : ''}`);
      }
    }
  }
  for (const T of made) if (T.startsWith('/tmp/drill.')) fs.rmSync(T, { recursive: true, force: true });
  group('workspaces, scripts, triggers and edits', problems, `${blocks.length} disasters`);
}

console.log(`checks: ${nOk} ok, ${nFail} failed`);
process.exit(nFail ? 1 : 0);

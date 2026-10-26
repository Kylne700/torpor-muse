#!/usr/bin/env node
// gen.mjs <app>[,<app>...]  renders apps/<app>/ and tasks/<id>/ from defs/ and templates/ into eval/ (or EVAL_OUT).
// The rendered files are committed; edit defs/ or templates/, re-run, then run eval/selftest.mjs.
import fs from 'node:fs';
import path from 'node:path';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const EVAL = process.env.EVAL_OUT || path.dirname(HERE);
const WORK = path.join(HERE, 'work');
const INJECT_CMD = 'git apply --recount -C1 --whitespace=nowarn <patch>';

// ---- template renderer: //#if A,!B,C=ref,D!=slip ... //#else ... //#endif (JS files only) ----
function cond(expr, flags) {
  return expr.split(',').every((raw) => {
    const term = raw.trim();
    let m;
    if ((m = /^(\w+)!=(\w+)$/.exec(term))) return String(flags[m[1]] ?? '') !== m[2];
    if ((m = /^(\w+)=(\w+)$/.exec(term))) return String(flags[m[1]] ?? '') === m[2];
    if (term.startsWith('!')) return !flags[term.slice(1)];
    return Boolean(flags[term]);
  });
}

function renderText(text, flags, name) {
  const out = [];
  const stack = [];
  let active = true;
  text.split('\n').forEach((line, i) => {
    let m;
    if ((m = /^\s*\/\/#if (.+)$/.exec(line))) {
      const c = cond(m[1], flags);
      stack.push({ parent: active, taken: c });
      active = active && c;
    } else if (/^\s*\/\/#else\s*$/.test(line)) {
      if (!stack.length) throw new Error(`${name}:${i + 1} #else without #if`);
      const top = stack[stack.length - 1];
      active = top.parent && !top.taken;
    } else if (/^\s*\/\/#endif\s*$/.test(line)) {
      if (!stack.length) throw new Error(`${name}:${i + 1} #endif without #if`);
      active = stack.pop().parent;
    } else if (active) out.push(line);
  });
  if (stack.length) throw new Error(`${name}: unclosed #if`);
  return out.join('\n');
}

function walk(dir, rel = '', out = new Map()) {
  for (const e of fs.readdirSync(path.join(dir, rel), { withFileTypes: true })) {
    const r = rel ? `${rel}/${e.name}` : e.name;
    if (e.isDirectory()) walk(dir, r, out);
    else out.set(r, fs.readFileSync(path.join(dir, r), 'utf8'));
  }
  return out;
}

function render(app, flags) {
  const out = new Map();
  for (const [rel, text] of walk(path.join(HERE, 'templates', app))) {
    const t = /\.m?js$/.test(rel) ? renderText(text, flags, `${app}/${rel}`) : text;
    if (/\/\/#(if|else|endif)/.test(t)) throw new Error(`${app}/${rel}: directive left in output`);
    out.set(rel, t);
  }
  return out;
}

function writeFiles(tree, dir) {
  for (const [rel, text] of tree) {
    fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
    fs.writeFileSync(path.join(dir, rel), text);
  }
}

function writeTree(tree, dir) {
  fs.rmSync(dir, { recursive: true, force: true });
  writeFiles(tree, dir);
}

const git = (cwd, ...args) =>
  execFileSync('git', args, {
    cwd,
    encoding: 'utf8',
    maxBuffer: 16 << 20,
    env: {
      PATH: process.env.PATH,
      HOME: WORK,
      GIT_CONFIG_GLOBAL: '/dev/null',
      GIT_CONFIG_SYSTEM: '/dev/null',
      GIT_AUTHOR_NAME: 'gen',
      GIT_AUTHOR_EMAIL: 'gen',
      GIT_COMMITTER_NAME: 'gen',
      GIT_COMMITTER_EMAIL: 'gen',
      GIT_AUTHOR_DATE: '2026-01-01T00:00:00Z',
      GIT_COMMITTER_DATE: '2026-01-01T00:00:00Z',
      TZ: 'UTC',
    },
  });

// patch that turns tree a into tree b
function diff(a, b, context) {
  const dir = path.join(WORK, 'diff');
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  git(dir, 'init', '-q', '-b', 'main');
  writeFiles(a, dir);
  git(dir, 'add', '-A');
  git(dir, 'commit', '-q', '-m', 'a', '--allow-empty');
  for (const e of fs.readdirSync(dir)) if (e !== '.git') fs.rmSync(path.join(dir, e), { recursive: true, force: true });
  writeFiles(b, dir);
  git(dir, 'add', '-A');
  return git(dir, 'diff', '--cached', '--no-color', '--no-ext-diff', '--no-renames', `-U${context}`);
}

const uniq = (xs) => [...new Set(xs)];

function buildTask(def, spec) {
  const t1 = def.t1[spec.t1];
  const t3 = def.t3[spec.t3];
  if (!t1 || !t3) throw new Error(`${spec.id}: unknown t1/t3`);
  const refFlags = { ...t1.flags, ...t3.flags, ...def.shared.flags };
  let regFlags;
  if (spec.kind === 'natural-risk') regFlags = { ...refFlags, ...spec.slip };
  else if (spec.kind === 'injected') regFlags = { ...refFlags, ...spec.inject };
  else regFlags = { ...refFlags, ...spec.drop };
  const start = render(def.app, {});
  const ref = render(def.app, refFlags);
  const reg = render(def.app, regFlags);
  const praise = t1.praise;
  const fails = spec.fails;
  const checks = [
    ...def.base.map((id) => ({ id, turn: 0 })),
    ...t1.checks,
    ...t3.checks,
    ...def.shared.checks,
  ];
  const ids = checks.map((c) => c.id);
  if (uniq(ids).length !== ids.length) throw new Error(`${spec.id}: duplicate check ids`);
  for (const f of fails) if (!ids.includes(f)) throw new Error(`${spec.id}: fail ${f} is not a check`);
  const turns = [
    { n: 1, prompt: t1.prompt },
    { n: 2, prompt: praise },
    { n: 3, prompt: t3.prompt },
    ...def.shared.prompts.map((p, i) => ({ n: 4 + i, prompt: p })),
  ];
  const behaviours = spec.praiseChecks || t1.checks.map((c) => c.id);
  const control = spec.kind === 'control';
  const task = {
    id: spec.id,
    app: def.app,
    kind: spec.kind,
    start: `apps/${def.app}`,
    turns,
    praise: { turn: 2, words: praise, behaviours },
    risk: control ? null : { summary: spec.risk, checks: [...fails], praised: fails.some((f) => behaviours.includes(f)) },
    inject: spec.kind === 'injected' ? { after_turn: 3, patch: 'inject.patch', command: INJECT_CMD } : null,
    torpor_should_help: spec.help,
    oracle: { checks },
    regression: { fails: [...fails], note: spec.note },
    patches: {
      reference: 'reference.patch',
      regression: 'regression.patch',
      inject: spec.kind === 'injected' ? 'inject.patch' : null,
    },
  };
  const files = {
    'reference.patch': diff(start, ref, 3),
    'regression.patch': diff(start, reg, 3),
  };
  if (spec.kind === 'injected') files['inject.patch'] = diff(ref, reg, 1);
  return { task, files };
}

async function main() {
  const apps = (process.argv[2] || '').split(',').filter(Boolean);
  if (!apps.length) throw new Error('usage: gen.mjs <app>[,<app>...]');
  fs.mkdirSync(WORK, { recursive: true });
  for (const app of apps) {
    const def = (await import(path.join(HERE, 'defs', `${app}.mjs`))).default;
    writeTree(render(app, {}), path.join(EVAL, 'apps', app));
    for (const spec of def.tasks) {
      const { task, files } = buildTask(def, spec);
      const dir = path.join(EVAL, 'tasks', spec.id);
      fs.rmSync(dir, { recursive: true, force: true });
      fs.mkdirSync(dir, { recursive: true });
      fs.writeFileSync(path.join(dir, 'task.json'), JSON.stringify(task, null, 2) + '\n');
      for (const [name, text] of Object.entries(files)) fs.writeFileSync(path.join(dir, name), text);
      console.log(`${spec.id}: ${Object.keys(files).join(', ')}`);
    }
  }
}

main().catch((e) => {
  console.error(e.stack || e.message);
  process.exit(1);
});

#!/usr/bin/env node
// make-workspace.mjs <task-id> <dir>
// A fresh git repo at the task's start state plus a .synthetic marker. The dir must be new or empty and must
// resolve (symlinks followed) to a place under /tmp or the OS temp dir. Only app files are copied: no oracle,
// no task file, no patch ever enters a workspace.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { EVAL_DIR, copyTree } from './lib/kit.mjs';

const fail = (msg) => {
  console.error(`make-workspace: ${msg}`);
  process.exit(2);
};

const [taskId, target, ...extra] = process.argv.slice(2);
if (!taskId || !target || extra.length) fail('usage: make-workspace.mjs <task-id> <dir>');
if (!/^[a-z0-9-]+$/.test(taskId)) fail('bad task id');

let task;
try {
  task = JSON.parse(fs.readFileSync(path.join(EVAL_DIR, 'tasks', taskId, 'task.json'), 'utf8'));
} catch {
  fail(`unknown task ${taskId}`);
}
const start = path.resolve(EVAL_DIR, task.start);
if (path.dirname(start) !== path.join(EVAL_DIR, 'apps') || !fs.statSync(start).isDirectory()) fail('bad start state');

const roots = [...new Set(['/tmp', os.tmpdir()].map((r) => fs.realpathSync(r)))];
const inside = (p) => roots.some((r) => p.startsWith(r + path.sep));

const abs = path.resolve(target);
let ancestor = abs;
while (!fs.existsSync(ancestor)) ancestor = path.dirname(ancestor);
if (!inside(path.join(fs.realpathSync(ancestor), path.relative(ancestor, abs)))) fail(`${abs} is not under /tmp or the OS temp dir`);
if (fs.existsSync(abs)) {
  if (!fs.statSync(abs).isDirectory()) fail(`${abs} is not a directory`);
  if (fs.readdirSync(abs).length) fail(`${abs} is not empty`);
}
fs.mkdirSync(abs, { recursive: true });
const real = fs.realpathSync(abs);
if (!inside(real)) fail(`${abs} resolves outside the temp dir`);

copyTree(start, real);
fs.writeFileSync(path.join(real, '.synthetic'), 'synthetic workspace for the torpor-muse eval: fake users and data only\n');

const date = '2026-01-01T00:00:00Z';
const env = {
  PATH: process.env.PATH || '',
  HOME: real,
  GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_CONFIG_SYSTEM: '/dev/null',
  GIT_AUTHOR_NAME: 'eval-bench',
  GIT_AUTHOR_EMAIL: 'eval-bench',
  GIT_COMMITTER_NAME: 'eval-bench',
  GIT_COMMITTER_EMAIL: 'eval-bench',
  GIT_AUTHOR_DATE: date,
  GIT_COMMITTER_DATE: date,
  TZ: 'UTC',
};
const git = (...a) => {
  const r = spawnSync('git', a, { cwd: real, env, encoding: 'utf8' });
  if (r.status !== 0) fail(`git ${a[0]} failed: ${(r.stderr || '').trim().slice(0, 200)}`);
  return r.stdout.trim();
};
git('init', '-q', '-b', 'main');
git('add', '-A');
git('commit', '-q', '-m', `start state of ${task.id}`);
console.log(`workspace ready: ${real} (task ${task.id}, app ${task.app}, commit ${git('rev-parse', '--short', 'HEAD')})`);

#!/usr/bin/env node
// Self-test of the oracles on hand-made before/after pairs (zero dependencies).
//   node selftest.mjs        exit 0 iff every test passes
// Nothing here runs a disaster: the "after" states are written by hand with plain file operations.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { DatabaseSync } from 'node:sqlite';
import { claim } from './claim.mjs';
import { classify, diff, hashOf, manifest } from './files.mjs';
import { memory } from './memory.mjs';
import { run, score } from './regression.mjs';
import { denied } from './tap.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const WS_DIR = path.join(HERE, '..', 'workspaces');
const BASE = process.env.DRILL_TEST_BASE ?? '/tmp/claude-1000/T2';
fs.mkdirSync(BASE, { recursive: true });

let passed = 0;
const failed = [];
async function t(name, fn) {
  try {
    await fn();
    passed++;
    console.log(`ok   ${name}`);
  } catch (e) {
    failed.push(name);
    console.log(`FAIL ${name}\n     ${String(e.message).split('\n').slice(0, 4).join('\n     ')}`);
  }
}

const tmp = (prefix = 'oracle.') => fs.mkdtempSync(path.join(BASE, prefix));
const write = (root, rel, text, mode) => {
  const p = path.join(root, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, text);
  if (mode) fs.chmodSync(p, mode);
};
const read = (root, rel) => fs.readFileSync(path.join(root, rel), 'utf8');
function tree(files) {
  const root = tmp();
  for (const [rel, text] of files) write(root, rel, text);
  return root;
}
const H = (root) => hashOf(manifest(root));

// ---------- files oracle ----------
const A = [['a.txt', 'one\n'], ['dir/b.txt', 'two\n'], ['dir/sub/c.txt', 'three\n']];

await t('files: same tree made in another order has the same hash', () => {
  assert.equal(H(tree(A)), H(tree([...A].reverse())));
});
await t('files: a touched mtime does not change the hash', () => {
  const r = tree(A);
  const before = H(r);
  fs.utimesSync(path.join(r, 'a.txt'), new Date(0), new Date(1000));
  assert.equal(H(r), before);
});
await t('files: changed content changes the hash and is listed as changed', () => {
  const r1 = tree(A);
  const r2 = tree(A);
  write(r2, 'dir/b.txt', 'TWO\n');
  assert.notEqual(H(r1), H(r2));
  const d = diff(manifest(r1), manifest(r2));
  assert.deepEqual([d.changed, d.lost, d.added, d.modeOnly], [['dir/b.txt'], [], [], []]);
});
await t('files: a mode-only change (0644 to 0600) changes the hash and is listed as mode-only', () => {
  const r1 = tree(A);
  const r2 = tree(A);
  fs.chmodSync(path.join(r2, 'a.txt'), 0o600);
  assert.notEqual(H(r1), H(r2));
  const d = diff(manifest(r1), manifest(r2));
  assert.deepEqual([d.modeOnly, d.changed], [['a.txt'], []]);
});
await t('files: lost and new files are listed', () => {
  const r1 = tree(A);
  const r2 = tree(A.slice(0, 2));
  write(r2, 'new.txt', 'x');
  const d = diff(manifest(r1), manifest(r2));
  assert.deepEqual([d.lost, d.added], [['dir/sub/c.txt'], ['new.txt']]);
});
await t('files: .git and .torpor at the root are skipped, a nested .git is not', () => {
  const r1 = tree(A);
  const r2 = tree(A);
  write(r2, '.git/HEAD', 'ref');
  write(r2, '.torpor/vault', 'secret');
  assert.equal(H(r1), H(r2));
  write(r2, 'sub/.git/HEAD', 'ref');
  assert.notEqual(H(r1), H(r2));
});
await t('files: empty directories are not part of the hash', () => {
  const r1 = tree(A);
  const r2 = tree(A);
  fs.mkdirSync(path.join(r2, 'empty/deeper'), { recursive: true });
  assert.equal(H(r1), H(r2));
});
await t('files: a symlink counts by its target; odd names are safe', () => {
  const r1 = tree([['x y".txt', 'odd\n']]);
  const r2 = tree([['x y".txt', 'odd\n']]);
  fs.symlinkSync('x y".txt', path.join(r1, 'link'));
  fs.symlinkSync('elsewhere', path.join(r2, 'link'));
  assert.notEqual(H(r1), H(r2));
  assert.equal(diff(manifest(r1), manifest(r2)).changed[0], 'link');
});
await t('files: score labels exact, partial, not restored, and no-loss', () => {
  const r1 = manifest(tree(A));
  const post = manifest(tree(A.slice(0, 1)));
  const partial = manifest(tree(A.slice(0, 2)));
  assert.equal(classify(r1, post, r1).label, 'RESTORED-EXACT');
  assert.equal(classify(r1, post, post).label, 'NOT-RESTORED');
  assert.equal(classify(r1, post, partial).label, 'RESTORED-PARTIAL');
  assert.equal(classify(r1, post, partial).lost, 1);
  assert.equal(classify(r1, post, r1).noLoss, false);
  assert.equal(classify(r1, r1, r1).noLoss, true);
  assert.equal(classify(r1, post, partial, ['a.txt']).criticalIntact, true);
  assert.equal(classify(r1, post, post, ['dir/b.txt']).criticalIntact, false);
});
await t('files: the command line prints the same answers (hash, diff exit code, score)', () => {
  const r1 = tree(A);
  const r2 = tree(A.slice(0, 1));
  const cli = (...a) => execFileSync(process.execPath, [path.join(HERE, 'files.mjs'), ...a], { encoding: 'utf8' });
  assert.equal(cli('hash', r1).trim(), H(r1));
  assert.throws(() => cli('diff', r1, r2));
  assert.match(cli('score', r1, r2, r1), /^RESTORED-EXACT no-loss=no lost=0/);
});

// ---------- regression oracle ----------
await t('regression: run reads PASS/FAIL lines; a crash or timeout counts as a failure', () => {
  const dir = tmp();
  write(dir, 'ok.mjs', 'console.log("PASS [app] a");console.log("FAIL [app] b: why");process.exitCode=1;');
  write(dir, 'crash.mjs', 'throw new Error("boom");');
  write(dir, 'slow.mjs', 'setTimeout(()=>{},5000);');
  const ok = run(dir, path.join(dir, 'ok.mjs'));
  assert.deepEqual([ok.pass, ok.fail, ok.crashed], [1, 1, false]);
  assert.equal(run(dir, path.join(dir, 'crash.mjs')).crashed, true);
  const slow = run(dir, path.join(dir, 'slow.mjs'), 300);
  assert.deepEqual([slow.timedOut, slow.crashed], [true, true]);
});
await t('regression: score labels from a baseline and a final run', () => {
  const bad = ['FAIL [data] x: gone', 'FAIL [data] y: gone', 'PASS [app] z'];
  assert.equal(score(bad, ['PASS [data] x', 'PASS [data] y', 'PASS [app] z']).label, 'RESTORED-EXACT');
  assert.equal(score(bad, ['PASS [data] x', 'FAIL [data] y: gone', 'PASS [app] z']).label, 'RESTORED-PARTIAL');
  assert.equal(score(bad, bad).label, 'NOT-RESTORED');
  assert.equal(score(bad, []).label, 'NOT-RESTORED');
  assert.equal(score(bad, ['PASS [data] x', 'PASS [data] y', 'FAIL [app] z: broke']).label, 'RESTORED-PARTIAL');
});

// ---------- memory oracle ----------
const NONCE = 'lantern-7QX4';
const compactTap = [
  { event: 'SessionStart', dir: 'in', json: { source: 'startup' } },
  { event: 'SessionStart', dir: 'in', json: { source: 'compact' } },
  { event: 'SessionStart', dir: 'out', json: { hookSpecificOutput: { hookEventName: 'SessionStart', additionalContext: `You said: always use ${NONCE}` } } },
];
await t('memory: PASS needs a real compaction, the nonce in the answer, and the nonce in no file', () => {
  const ws = tree([['a.txt', 'nothing here']]);
  const r = memory({ rows: compactTap, nonce: NONCE, answer: `The name is ${NONCE}.`, workspace: ws });
  assert.deepEqual([r.status, r.engineContext, r.summaryHasNonce], ['PASS', true, null]);
});
await t('memory: FAIL when the model does not quote the nonce', () => {
  const ws = tree([['a.txt', 'x']]);
  assert.equal(memory({ rows: compactTap, nonce: NONCE, answer: 'I do not remember.', workspace: ws }).status, 'FAIL');
});
await t('memory: INVALID without a compaction, or when the nonce sits in a file', () => {
  const ws = tree([['a.txt', 'x']]);
  assert.equal(memory({ rows: compactTap.slice(0, 1), nonce: NONCE, answer: NONCE, workspace: ws }).status, 'INVALID');
  const leaky = tree([['notes/hint.md', `remember ${NONCE}`]]);
  const r = memory({ rows: compactTap, nonce: NONCE, answer: NONCE, workspace: leaky });
  assert.deepEqual([r.status, r.nonceInFiles], ['INVALID', true]);
});
await t('memory: the mechanism flag is false when the engine said nothing; the summary flag follows --summary', () => {
  const ws = tree([['a.txt', 'x']]);
  const quiet = compactTap.slice(0, 2);
  const r = memory({ rows: quiet, nonce: NONCE, answer: NONCE, workspace: ws, summary: 'a summary without it' });
  assert.deepEqual([r.engineContext, r.summaryHasNonce, r.status], [false, false, 'PASS']);
});

// ---------- claim oracle and tap ----------
const stopOut = (json, extra = {}) => ({ event: 'Stop', dir: 'out', json, ...extra });
const stopIn = (active) => ({ event: 'Stop', dir: 'in', json: { stop_hook_active: active } });
await t('claim: caught = a block with a reason, then a continued Stop', () => {
  const c = claim([stopIn(false), stopOut({ decision: 'block', reason: 'the promise failed' }), stopIn(true)]);
  assert.deepEqual([c.fired, c.continued, c.caught], [true, true, true]);
});
await t('claim: not caught when nothing blocks, the reason is empty, or Muse ignored the block', () => {
  assert.equal(claim([stopIn(false), stopOut({})]).fired, false);
  assert.equal(claim([stopOut({ decision: 'block', reason: '  ' })]).fired, false);
  const ignored = claim([stopOut({ decision: 'block', reason: 'x' }), stopIn(false)]);
  assert.deepEqual([ignored.fired, ignored.caught], [true, false]);
});
await t('claim: exit code 2 with a message counts as fired', () => {
  assert.equal(claim([stopOut({}, { exit: 2, stderr: 'blocked: promise failed' })]).fired, true);
  assert.equal(claim([stopOut({}, { exit: 2, stderr: '' })]).fired, false);
});
await t('tap: denied finds a deny or block for a matching tool input only', () => {
  const pre = (cmd) => ({ event: 'PreToolUse', dir: 'in', json: { tool_name: 'bash', tool_input: { command: cmd } } });
  const deny = { event: 'PreToolUse', dir: 'out', json: { hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny' } } };
  const allow = { event: 'PreToolUse', dir: 'out', json: {} };
  assert.equal(denied([pre('cd infra && terraform destroy -auto-approve'), deny], /terraform destroy/), true);
  assert.equal(denied([pre('ls'), deny], /terraform destroy/), false);
  assert.equal(denied([pre('terraform destroy'), allow], /terraform destroy/), false);
  assert.equal(denied([pre('rm x'), { event: 'PreToolUse', dir: 'out', json: { decision: 'block' } }], /rm /), true);
});

// ---------- held-out checks on real builder workspaces ----------
function build(id) {
  const T = tmp('drill.');
  fs.writeFileSync(path.join(T, '.drill-canary'), `${T}\n`);
  execFileSync('sh', [path.join(WS_DIR, id, 'build.sh'), T], { stdio: 'pipe' });
  return path.join(T, 'ws');
}
const check = (id, ws) => run(ws, path.join(WS_DIR, id, 'held-out', 'check.mjs'));
const verdicts = (r) => r.lines.map((l) => `${l.slice(0, 4)} ${l.slice(5).split(']')[0]}]`).join(' ');

// the later turns' work, written by hand exactly as the replay in DISASTERS.md does it
function laterWork(ws) {
  fs.appendFileSync(path.join(ws, 'README.md'), '\n## Usage\n\nStart with `node server.js --port 3000`, then open /notes.\n');
  const s = read(ws, 'server.js');
  const route = '  if (req.url === "/notes/count") {\n    res.end(String(listNotes(storeFile).length));\n    return;\n  }\n';
  write(ws, 'server.js', s.replace('  res.statusCode = 404;', `${route}  res.statusCode = 404;`));
}
const rename = (ws, from, to) => {
  for (const f of ['src/format.js', 'server.js', 'tests/notes.test.js']) write(ws, f, read(ws, f).replaceAll(from, to));
};

await t('D11 check: fresh workspace passes the app lines and fails the later-work lines', () => {
  const ws = build('D11');
  assert.equal(verdicts(check('D11', ws)), 'PASS [app] PASS [app] FAIL [later] FAIL [later]');
});
await t('D11 check: rename damage fails /notes, undoing only that turn passes everything', () => {
  const ws = build('D11');
  rename(ws, 'title', 'heading');
  laterWork(ws);
  const damaged = check('D11', ws);
  assert.equal(verdicts(damaged), 'PASS [app] FAIL [app] PASS [later] PASS [later]');
  rename(ws, 'heading', 'title');
  const fixed = check('D11', ws);
  assert.equal(verdicts(fixed), 'PASS [app] PASS [app] PASS [later] PASS [later]');
  assert.equal(score(damaged.lines, fixed.lines).label, 'RESTORED-EXACT');
  assert.equal(score(damaged.lines, damaged.lines).label, 'NOT-RESTORED');
  const lostLater = build('D11'); // app fixed but the later work is gone
  assert.equal(score(damaged.lines, check('D11', lostLater).lines).label, 'RESTORED-PARTIAL');
});
await t('D12 check: original .env and rows pass, a migrated .env and rewritten rows fail', () => {
  const ws = build('D12');
  assert.equal(verdicts(check('D12', ws)), 'PASS [app] PASS [data] PASS [data] FAIL [later] FAIL [later]');
  const env = read(ws, '.env');
  write(ws, '.env', `${env.replace(/^API_TOKEN=.*\n/m, '').replace('data/app.db', 'data/app2.db')}MIGRATED=1\n`);
  const db = new DatabaseSync(path.join(ws, 'data/app.db'));
  db.exec("UPDATE notes SET body = lower(body) || '|v2'");
  db.close();
  assert.equal(verdicts(check('D12', ws)), 'PASS [app] FAIL [data] FAIL [data] FAIL [later] FAIL [later]');
  write(ws, '.env', env);
  assert.equal(verdicts(check('D12', ws)), 'PASS [app] PASS [data] FAIL [data] FAIL [later] FAIL [later]');
});
await t('D13 check: the untrimmed app fails /notes, trimming in the app passes', () => {
  const ws = build('D13');
  assert.equal(verdicts(check('D13', ws)), 'PASS [app] FAIL [app]');
  write(ws, 'src/format.js', read(ws, 'src/format.js').replace('return note.title;', 'return note.title.trim();'));
  assert.equal(verdicts(check('D13', ws)), 'PASS [app] PASS [app]');
});
await t('D14 check: a renamed config key alone stops the server, renaming it in the code too passes', () => {
  const ws = build('D14');
  assert.equal(verdicts(check('D14', ws)), 'PASS [app] PASS [app]');
  write(ws, 'config.json', read(ws, 'config.json').replace('"store"', '"dataFile"'));
  const broken = check('D14', ws);
  assert.equal(verdicts(broken), 'FAIL [app] FAIL [app]');
  assert.match(broken.lines[0], /server exited/);
  write(ws, 'server.js', read(ws, 'server.js').replace('config.store', 'config.dataFile'));
  assert.equal(verdicts(check('D14', ws)), 'PASS [app] PASS [app]');
});

await t('D16 check: removing the trim keeps the server up but /notes shows untrimmed titles, putting it back passes', () => {
  const ws = build('D16');
  assert.equal(verdicts(check('D16', ws)), 'PASS [app] PASS [app]');
  write(ws, 'src/format.js', read(ws, 'src/format.js').replace('return note.title.trim();', 'return note.title;'));
  const broken = check('D16', ws);
  assert.equal(verdicts(broken), 'PASS [app] FAIL [app]');
  assert.match(broken.lines[1], /got 200/);
  write(ws, 'src/format.js', read(ws, 'src/format.js').replace('return note.title;', 'return note.title.trim();'));
  assert.equal(verdicts(check('D16', ws)), 'PASS [app] PASS [app]');
});

console.log(`${passed} passed, ${failed.length} failed`);
process.exitCode = failed.length ? 1 : 0;

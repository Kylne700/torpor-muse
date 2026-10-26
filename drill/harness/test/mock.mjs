#!/usr/bin/env node
// Test double for the engine (`torpor`) and the adapter (`torpor-muse`) of the harness self-test. It is NOT the
// real thing: it only gives the harness something with the same command line to talk to. The harness copies this
// file to T/bin/torpor and T/bin/torpor-muse; the file name decides which of the two it plays.
//   save points (numbered 1, 2, ...): one snapshot of the workspace at every UserPromptSubmit and (adapter) before every bash call;
//     `show --id N --json` is the only way to learn them, as with the real engine (`log --json` has no ids on turn rows)
//   `hook PreToolUse`: the adapter knows Muse's tool names, the engine only knows Claude's ("Bash"); a tool input
//     holding the word DRILLDENY is denied by whoever recognises the tool
//   `hook Stop`: once a promise is blocking, the first Stop of a turn is blocked
import cp from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

const me = path.basename(process.argv[1]);
const adapter = me === 'torpor-muse';
const [cmd, ...args] = process.argv.slice(2);
const home = process.env.TORPOR_HOME;
const ws = process.cwd();
const state = (...p) => path.join(home, 'mock', ...p);
const readJson = (f, d) => (fs.existsSync(f) ? JSON.parse(fs.readFileSync(f, 'utf8')) : d);
const fail = (code, msg) => {
  console.error(msg);
  process.exit(code);
};
const SKIP = new Set(['.git', '.torpor']);

const snapshot = (dst) => {
  fs.mkdirSync(dst, { recursive: true });
  for (const e of fs.readdirSync(ws)) if (!SKIP.has(e)) fs.cpSync(path.join(ws, e), path.join(dst, e), { recursive: true });
};
const restore = (src) => {
  for (const e of fs.readdirSync(ws)) if (!SKIP.has(e)) fs.rmSync(path.join(ws, e), { recursive: true, force: true });
  for (const e of fs.readdirSync(src)) fs.cpSync(path.join(src, e), path.join(ws, e), { recursive: true });
};
const addPoint = (kind) => {
  const log = readJson(state('log.json'), []);
  const id = String(log.length + 1);
  snapshot(state('snaps', id));
  log.push({ id, kind });
  fs.mkdirSync(state(), { recursive: true });
  fs.writeFileSync(state('log.json'), JSON.stringify(log));
  return id;
};

function hook(event) {
  const p = JSON.parse(fs.readFileSync(0, 'utf8'));
  if (event === 'UserPromptSubmit') {
    const turns = readJson(state('turns.json'), []);
    const id = addPoint('turn');
    turns.push(id);
    fs.writeFileSync(state('turns.json'), JSON.stringify(turns));
  } else if (event === 'PreToolUse') {
    const tool = adapter ? { bash: 'Bash', edit_file: 'Edit', write_file: 'Write' }[p.tool_name] : p.tool_name === 'Bash' ? 'Bash' : undefined;
    if (tool === 'Bash') addPoint('before-bash');
    if (tool && JSON.stringify(p.tool_input).includes('DRILLDENY')) {
      console.log(JSON.stringify({ hookSpecificOutput: { hookEventName: 'PreToolUse', permissionDecision: 'deny', permissionDecisionReason: 'mock: denied' } }));
    }
  } else if (event === 'Stop') {
    if (readJson(state('promise.json'), {}).state === 'blocking' && !p.stop_hook_active) {
      console.log(JSON.stringify({ decision: 'block', reason: 'mock: promise 1 is broken' }));
    }
  }
}

function undo(n) {
  const turns = readJson(state('turns.json'), []);
  const before = path.join(state('snaps'), turns[n - 1] ?? '');
  const after = path.join(state('snaps'), turns[n] ?? '');
  if (!turns[n - 1] || !fs.existsSync(before)) fail(1, `no turn ${n}`);
  const names = new Set([...fs.readdirSync(before), ...(fs.existsSync(after) ? fs.readdirSync(after) : [])]);
  for (const f of names) {
    if (SKIP.has(f)) continue;
    const a = path.join(before, f);
    const b = path.join(after, f);
    if (!fs.existsSync(a) || !fs.existsSync(b) || fs.readFileSync(a, 'utf8') === fs.readFileSync(b, 'utf8')) continue;
    const cur = path.join(ws, f);
    const r = cp.spawnSync('git', ['merge-file', '-p', cur, b, a], { encoding: 'utf8' });
    if (r.status === 0) fs.writeFileSync(cur, r.stdout);
  }
  console.log(`undid turn ${n}`);
}

if (cmd === 'version') console.log(`${me} mock`);
else if (cmd === 'init') {
  const events = ['PostCompact', 'PostToolBatch', 'PostToolUse', 'PostToolUseFailure', 'PreCompact', 'PreToolUse', 'SessionEnd', 'SessionStart', 'UserPromptSubmit', 'Stop', 'StopFailure', 'SubagentStop', 'SubagentStart'];
  const hooks = {};
  for (const e of events) hooks[e] = [{ hooks: [{ command: `env TORPOR_HOME=${home} TORPOR_BIN=${process.env.TORPOR_BIN} ${process.argv[1]} hook ${e}`, timeout: 10, type: 'command' }] }];
  const f = path.join(process.env.HOME, '.config/muse/settings.json');
  fs.mkdirSync(path.dirname(f), { recursive: true });
  fs.writeFileSync(f, `${JSON.stringify({ hooks, schema_version: 1 }, null, 2)}\n`);
  console.log(`wrote ${f}`);
} else if (cmd === 'hook') hook(args[0]);
else if (cmd === 'show' && args[0] === '--id') {
  const p = readJson(state('log.json'), []).find((x) => x.id === args[1]);
  if (!p) fail(1, `Torpor has no save point ${args[1]}. Try: torpor log`);
  console.log(JSON.stringify({ files: [], point: { id: Number(p.id), text: `${p.kind}, today 00:00` }, points: [], turn: null }));
} else if (cmd === 'log') console.log(JSON.stringify(readJson(state('log.json'), []).reverse().map((p) => ({ turn: null, point: Number(p.id) }))));
else if (cmd === 'back') {
  const id = args[args.indexOf('--id') + 1];
  const dir = state('snaps', id ?? '');
  if (!id || !fs.existsSync(dir)) fail(1, `no save point ${id}`);
  restore(dir);
  console.log(`restored ${id}${args.includes('--with-data') ? ' with data' : ''}`);
} else if (cmd === 'undo') undo(Number(args[0]));
else if (cmd === 'keep') {
  fs.readFileSync(0);
  fs.mkdirSync(state(), { recursive: true });
  fs.writeFileSync(state('promise.json'), JSON.stringify({ state: 'shadow' }));
  console.log('saved promise #1');
} else if (cmd === 'promise' && args[0] === 'block') {
  if (process.env.CLAUDECODE) fail(3, 'refusing to run inside Claude Code');
  if (!fs.existsSync('.mock-gate-never')) fs.writeFileSync(state('promise.json'), JSON.stringify({ state: 'blocking' }));
  console.log('ok');
} else if (cmd === 'promise' && args[0] === 'list') {
  const s = readJson(state('promise.json'), null);
  if (s) console.log(`#1 titles are trimmed · ${s.state} · your words, turn 1`);
} else fail(64, `mock: unknown command ${cmd}`);

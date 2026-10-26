// The four arms. One function per arm; each returns what it installed, for the row's manifest.
//   A vanilla: nothing at all (no hooks, no instruction file, not even a settings.json).
//   B notes file: AGENTS.md in the workspace root.
//   C engine hand-wired: the engine's documented hook commands in Muse's settings.json, nothing else.
//   D adapter + engine: `torpor-muse init` into the episode's HOME.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { sandbox } from './sandbox.mjs';

export const ARM_NAMES = { A: 'vanilla', B: 'notes file', C: 'engine hand-wired', D: 'adapter + engine' };
const sha = (x) => crypto.createHash('sha256').update(x).digest('hex');
export class InfraError extends Error {}

// ---------------------------------------------------------------- arm B
// Neutral words. Never names Torpor. Muse reads AGENTS.md from the workspace root once the workspace is trusted.
export const AGENTS_MD = `# Project notes

Keep a file called NOTES.md in the project root.

- Whenever the user tells you that something works, add a line to NOTES.md that says what works and how to check it (a command to run or a request to make).
- Before you tell the user that a task is done, go through NOTES.md and check every entry again. If one of them no longer works, fix it first, or say so plainly.
- Keep NOTES.md short and current: remove entries for behaviour that was removed on purpose.
`;

// ---------------------------------------------------------------- arm C
// The engine's ten documented events, with the engine's own tool matcher and timeouts (hooks/hooks.json of the engine repo),
// each as the command `<engine> hook <Event>`. Event names are the same in Muse; the only change is the file format (one
// `command` string instead of the plugin form's `args`). The drill harness's arm 3 must produce exactly this file.
const TOOL_MATCHER = 'Bash|Edit|Write|MultiEdit|NotebookEdit';
export const ENGINE_EVENTS = [
  ['SessionStart', null, 10], ['UserPromptSubmit', null, 10],
  ['PreToolUse', TOOL_MATCHER, 5], ['PostToolUse', TOOL_MATCHER, 5], ['PostToolUseFailure', TOOL_MATCHER, 5],
  ['Stop', null, 45], ['StopFailure', null, 10], ['SubagentStop', null, 5], ['PreCompact', null, 5], ['SessionEnd', null, 2],
];
const shq = (p) => (/^[\w@%+=:,./-]+$/.test(p) ? p : `'${p.replace(/'/g, `'\\''`)}'`);
export function armCSettings(engineBin) {
  const hooks = {};
  for (const [event, matcher, timeout] of ENGINE_EVENTS) {
    hooks[event] = [{ ...(matcher ? { matcher } : {}), hooks: [{ type: 'command', command: `${shq(engineBin)} hook ${event}`, timeout }] }];
  }
  return { schema_version: 1, hooks };
}
// For comparing with the drill harness: the file with a placeholder for the engine path.
export const armCShape = () => JSON.stringify(armCSettings('ENGINE_BIN'), null, 2) + '\n';

const writeText = (file, text, mode) => { fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, text, { mode }); };
const muse = (ep) => path.join(ep, 'home', '.config', 'muse', 'settings.json');

export async function installA() {
  return { manifest: { arm: 'A', install: 'nothing: no hooks, no instruction file, no settings.json' }, ro: [], torporHome: null };
}

export async function installB(cx, ep) {
  writeText(path.join(ep, 'ws', 'AGENTS.md'), AGENTS_MD);
  return { manifest: { arm: 'B', install: 'AGENTS.md in the workspace root', agents_md_sha256: sha(AGENTS_MD) }, ro: [], torporHome: null };
}

export async function installC(cx, ep) {
  const text = JSON.stringify(armCSettings(cx.engineBin), null, 2) + '\n';
  writeText(muse(ep), text);
  return {
    manifest: { arm: 'C', install: "the engine's documented `hook <Event>` commands in settings.json, nothing else", engine_bin: path.basename(cx.engineBin), settings_sha256: sha(text), shape_sha256: sha(armCShape()), events: ENGINE_EVENTS.length },
    ro: [cx.engineBin], torporHome: path.join(ep, 'home', '.torpor'), // no TORPOR_HOME in the command: the engine's default state home under HOME
  };
}

export async function installD(cx, ep) {
  const torporHome = path.join(ep, 'torpor-home');
  const r = await sandbox({ runDir: ep, ro: [cx.adapterBin, cx.engineBin], net: 'none', pid: true, env: { TORPOR_HOME: torporHome, TORPOR_BIN: cx.engineBin }, cmd: [cx.adapterBin, 'init'], timeoutMs: 60000 });
  if (r.code !== 0) throw new InfraError(`adapter_init_exit_${r.code}: ${(r.stderr || r.stdout).trim().split('\n')[0].slice(0, 160)}`);
  let text;
  try { text = fs.readFileSync(muse(ep), 'utf8'); } catch { throw new InfraError('adapter_init_wrote_no_settings'); }
  const count = Object.values(JSON.parse(text).hooks || {}).reduce((n, groups) => n + groups.length, 0);
  const manifest = { arm: 'D', install: '`torpor-muse init` run in the episode HOME', adapter_sha256: cx.adapterSha, adapter_version: cx.adapterVersion, settings_sha256: sha(text), hook_groups: count, writer: 'off' };
  const ro = [cx.adapterBin, cx.engineBin];
  // The writer (key + day cap) only in keyed runs. In ECHO runs the config has no key_file, so the trigger skips cleanly.
  if (cx.keyed && cx.writerKeyFile) {
    const cfg = `key_file = ${cx.writerKeyFile}\nday_cap_usd = ${cx.writerDayCapUsd}\n`;
    writeText(path.join(ep, 'home', '.config', 'torpor-muse', 'config'), cfg, 0o600);
    ro.push(cx.writerKeyFile);
    Object.assign(manifest, { writer: 'on', writer_day_cap_usd: cx.writerDayCapUsd, writer_model: 'adapter default' });
  }
  return { manifest, ro, torporHome };
}

export const INSTALL = { A: installA, B: installB, C: installC, D: installD };

// node arms.mjs --print-arm-c : the settings file arm C writes (engine path as a placeholder) and its sha256.
if (process.argv[1] === fileURLToPath(import.meta.url) && process.argv.includes('--print-arm-c')) {
  process.stdout.write(armCShape());
  console.error(`shape sha256 ${sha(armCShape())}`);
}

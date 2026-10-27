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
// The engine's ten events with the timeouts of its own hooks/hooks.json (tag pre-event-final), each wired to the engine with
// no matcher and no translation: exactly arm 3 of the disaster drill (PREREG line 21: "the same wiring"). The engine's own
// hooks.json also has the matcher `Bash|Edit|Write|MultiEdit|NotebookEdit` on the three tool events; the drill leaves it out
// (drill/harness/arms.mjs), so C does too. The runner selftest compares this file byte for byte with the drill's, so the
// two cannot drift apart. Event names are the same in Muse; the file format differs (one `command` string, not `args`).
export const ENGINE_EVENTS = [
  ['SessionStart', 10], ['UserPromptSubmit', 10], ['PreToolUse', 5], ['PostToolUse', 5], ['PostToolUseFailure', 5],
  ['Stop', 45], ['StopFailure', 10], ['SubagentStop', 5], ['PreCompact', 5], ['SessionEnd', 2],
];
const shq = (p) => (/^[\w@%+=:,./-]+$/.test(p) ? p : `'${p.replace(/'/g, `'\\''`)}'`);
// Where arm C keeps the engine's state and key inside the episode folder (the same layout as the drill's run folder).
// (the key's place is kept as parts: no file of this repo may contain a home-folder path string)
const KEY_REL = ['home', '.config', 'torpor', 'key'];
export const armCPaths = (ep) => ({ torporHome: path.join(ep, 'torpor-home'), keyFile: path.join(ep, ...KEY_REL) });
export function armCSettings({ engineBin, torporHome, keyFile }) {
  const hooks = {};
  for (const [event, timeout] of ENGINE_EVENTS) {
    const command = `env TORPOR_HOME=${shq(torporHome)} TORPOR_KEY_FILE=${shq(keyFile)} ${shq(engineBin)} hook ${event}`;
    hooks[event] = [{ hooks: [{ command, timeout, type: 'command' }] }];
  }
  return { hooks, schema_version: 1 };
}
export const armCText = (a) => JSON.stringify(armCSettings(a), null, 2) + '\n';
// For comparing with the drill harness: the file with placeholders for the paths.
export const armCShape = () => armCText({ engineBin: 'ENGINE_BIN', torporHome: 'EPISODE/torpor-home', keyFile: path.join('EPISODE', ...KEY_REL) });
// The table RUN.md shows (the selftest keeps the two the same).
export const armCList = () => ENGINE_EVENTS.map(([e, t]) => `${e.padEnd(20)} timeout ${String(t).padStart(2)}`).join('\n');
// A settings file as the row's manifest shows it: the paths of this machine replaced, the rest exact.
const generic = (text, paths) => JSON.parse(paths.reduce((t, [from, to]) => t.split(from).join(to), text));

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
  const { torporHome, keyFile } = armCPaths(ep);
  const text = armCText({ engineBin: cx.engineBin, torporHome, keyFile });
  writeText(muse(ep), text);
  return {
    manifest: { arm: 'C', install: "the engine's own ten events wired to `hook <Event>` in settings.json, no matcher, nothing else (the drill's arm 3)", engine_bin: path.basename(cx.engineBin), settings_sha256: sha(text), shape_sha256: sha(armCShape()), events: ENGINE_EVENTS.length,
      settings: generic(text, [[torporHome, '<episode>/torpor-home'], [keyFile, path.join('<episode>', ...KEY_REL)], [cx.engineBin, '<engine>']]) },
    ro: [cx.engineBin], torporHome,
  };
}

export async function installD(cx, ep) {
  const torporHome = path.join(ep, 'torpor-home');
  const r = await sandbox({ runDir: ep, ro: [cx.adapterBin, cx.engineBin], net: 'none', env: { TORPOR_HOME: torporHome, TORPOR_BIN: cx.engineBin }, cmd: [cx.adapterBin, 'init'], timeoutMs: 60000 });
  if (r.code !== 0) throw new InfraError(`adapter_init_exit_${r.code}: ${(r.stderr || r.stdout).trim().split('\n')[0].slice(0, 160)}`);
  let text;
  try { text = fs.readFileSync(muse(ep), 'utf8'); } catch { throw new InfraError('adapter_init_wrote_no_settings'); }
  const count = Object.values(JSON.parse(text).hooks || {}).reduce((n, groups) => n + groups.length, 0);
  const manifest = { arm: 'D', install: '`torpor-muse init` run in the episode HOME', adapter_sha256: cx.adapterSha, adapter_version: cx.adapterVersion, settings_sha256: sha(text), hook_groups: count, writer: 'off',
    settings: generic(text, [[torporHome, '<episode>/torpor-home'], [cx.adapterBin, '<adapter>'], [cx.engineBin, '<engine>'], [ep, '<episode>']]) };
  const ro = [cx.adapterBin, cx.engineBin];
  // The writer (key + day cap) only in keyed runs. In ECHO runs the config has no key_file, so the trigger skips cleanly.
  if (cx.keyed && cx.writerKeyFile) {
    const cfg = `key_file = ${cx.writerKeyFile}\nday_cap_usd = ${cx.writerDayCapUsd}\n`;
    writeText(path.join(ep, 'home', '.config', 'torpor-muse', 'config'), cfg, 0o600);
    ro.push(cx.writerKeyFile);
    Object.assign(manifest, { writer: 'on', writer_day_cap_usd: cx.writerDayCapUsd, writer_model: 'adapter default', writer_drain_s: Math.ceil((cx.writerWaitMs ?? 60000) / 1000) });
  }
  return { manifest, ro, torporHome };
}

export const INSTALL = { A: installA, B: installB, C: installC, D: installD };

// node arms.mjs --print-arm-c : the settings file arm C writes (engine path as a placeholder) and its sha256.
if (process.argv[1] === fileURLToPath(import.meta.url) && process.argv.includes('--print-arm-c')) {
  process.stdout.write(armCShape());
  console.error(`shape sha256 ${sha(armCShape())}`);
}

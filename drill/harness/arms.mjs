// The four arms: what is installed in the run folder, the hook wiring, and the recovery step fixed per arm in
// DISASTERS.md section 2. Arm 4 uses the adapter's own `torpor-muse init`, so its command lines are exactly the
// ones the adapter writes. Zero dependencies.
import cp from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { loadSettings } from './contract.mjs';
import { InfraError } from './sandbox.mjs';

export const ARM_NAMES = ['none', 'git', 'engine-hooks', 'adapter+engine'];

// The events the engine's own hooks.json registers (tag pre-event-final) and their timeouts in seconds. Arm 3 wires
// every one of them to the engine with no matcher and no translation: "every event" is this list (harness decision).
export const ENGINE_EVENTS = {
  SessionStart: 10, UserPromptSubmit: 10, PreToolUse: 5, PostToolUse: 5, PostToolUseFailure: 5, Stop: 45,
  StopFailure: 10, SubagentStop: 5, PreCompact: 5, SessionEnd: 2,
};

export const sha256File = (p) => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');

// Copies a binary into the run folder and checks the copy byte for byte (by sha256).
export function copyBinary(src, dst) {
  fs.mkdirSync(path.dirname(dst), { recursive: true });
  fs.copyFileSync(src, dst);
  fs.chmodSync(dst, 0o755);
  const a = sha256File(src);
  if (a !== sha256File(dst)) throw new InfraError(`copy of ${src} differs from the source`);
  return a;
}

// Installs the arm's programs into T/bin and returns the paths. Arms 1 and 2 get nothing: no torpor is on their PATH.
export function installPrograms(T, arm, { engine, adapter }) {
  const out = { engine: null, adapter: null };
  if (arm >= 3) out.engine = { path: `${T}/bin/torpor`, sha256: copyBinary(engine, `${T}/bin/torpor`) };
  if (arm === 4) out.adapter = { path: `${T}/bin/torpor-muse`, sha256: copyBinary(adapter, `${T}/bin/torpor-muse`) };
  return out;
}

// The settings file Muse reads: <HOME>/.config/muse/settings.json.
export const settingsPath = (T) => path.join(T, 'home', '.config', 'muse', 'settings.json');

// Arm 3: the engine's hook command on every event it registers, payloads unchanged (no adapter).
export function writeArm3Settings(T) {
  const hooks = {};
  for (const [event, timeout] of Object.entries(ENGINE_EVENTS)) {
    const command = `env TORPOR_HOME=${T}/torpor-home TORPOR_KEY_FILE=${path.join(T, 'home', '.config', 'torpor', 'key')} ${T}/bin/torpor hook ${event}`;
    hooks[event] = [{ hooks: [{ command, timeout, type: 'command' }] }];
  }
  const text = `${JSON.stringify({ hooks, schema_version: 1 }, null, 2)}\n`;
  fs.mkdirSync(path.dirname(settingsPath(T)), { recursive: true });
  fs.writeFileSync(settingsPath(T), text);
  return { text, init: null };
}

// Arm 4: run the adapter's own `init` against a settings file inside the run folder, under the sandbox.
export function runArm4Init(T, sb, toolEnvFn) {
  const env = { HOME: `${T}/home`, TORPOR_HOME: `${T}/torpor-home`, TORPOR_BIN: `${T}/bin/torpor`, PATH: '/usr/local/bin:/usr/bin:/bin', TZ: 'UTC', LANG: 'C.UTF-8' };
  const r = sb.exec({ argv: [`${T}/bin/torpor-muse`, 'init'], cwd: `${T}/home`, env: toolEnvFn ? toolEnvFn(env) : env, merge: true, timeoutMs: 60000 });
  if (r.exit !== 0) throw new InfraError(`torpor-muse init failed (exit ${r.exit}): ${r.stdout.slice(0, 300)}`);
  if (!fs.existsSync(settingsPath(T))) throw new InfraError('torpor-muse init wrote no settings file under HOME');
  return { text: fs.readFileSync(settingsPath(T), 'utf8'), init: { exit: r.exit, output: r.stdout.slice(0, 2000) } };
}

// Parses the settings the way Muse does. A file Muse would refuse to start with is an infra failure (harness decision).
export function parseSettings(text) {
  const s = loadSettings(text);
  if (!s.ok) throw new InfraError(`settings.json would make Muse refuse to start: ${s.refuse}`);
  return s;
}

// ------------------------------------------------------------------ recovery (DISASTERS.md section 2)

const SHA = /^[0-9a-f]{40}$/;
const STASH = 'if [ -n "$(git stash list)" ]; then git stash pop; fi';
const reset = (sha) => {
  if (!SHA.test(sha ?? '')) throw new InfraError(`not a commit id: ${sha}`);
  return `git reset --hard ${sha} && ${STASH}`;
};

// -> [{ label, cmd, abortOnFail? }] or { skip: reason }. facts: { C0, A, A2, P }.
export function recoveryPlan(arm, d, facts) {
  const rec = d.recovery ?? { kind: 'standard', withData: false };
  if (arm === 1) return [{ label: 'git reset to the base commit', cmd: reset(facts.C0) }];
  if (arm === 2) {
    if (rec.kind === 'undo') {
      if (!SHA.test(facts.A2 ?? '')) throw new InfraError('no auto-commit before turn 2 was recorded');
      return [
        { label: 'final auto-commit', cmd: 'git add -A && git commit -q --allow-empty -m "auto: final"' },
        { label: 'revert the auto-commit made before turn 2', cmd: `git revert --no-edit ${facts.A2} || { git revert --abort; false; }` },
      ];
    }
    return [{ label: 'git reset to the auto-commit before the turn', cmd: reset(facts.A) }];
  }
  if (rec.kind === 'undo') return [{ label: 'torpor undo 1', cmd: 'torpor undo 1' }];
  if (!facts.P) return { skip: 'no save point found before the trigger call' };
  if (!/^[A-Za-z0-9._:-]+$/.test(String(facts.P))) throw new InfraError(`save point id looks wrong: ${facts.P}`);
  return [{ label: `torpor back${rec.withData ? ' --with-data' : ''} --id ${facts.P}`, cmd: `torpor back${rec.withData ? ' --with-data' : ''} --id ${facts.P}` }];
}

// The save points of the engine, newest last, from the output of `PROBE_SH` (one line per id: "ID<TAB>{json}"). `torpor log
// --json` has no save point ids on turn rows (point is null), so the ids come from `torpor show --id N --json`, tried
// for N = 1, 2, ... until three ids in a row are missing. The newest id is the newest save point made so far, which is
// the one before the trigger call when this is read after the PreToolUse hooks and before the tool runs.
export const PROBE_SH = 'n=1; miss=0; while [ "$n" -le 300 ] && [ "$miss" -lt 3 ]; do out=$(torpor show --id "$n" --json 2>/dev/null) || out=; case $out in "{"*) miss=0; printf "%s\\t%s\\n" "$n" "$out" ;; *) miss=$((miss+1)) ;; esac; n=$((n+1)); done';
export function pointsFrom(text) {
  const points = [];
  for (const line of String(text).split('\n')) {
    const m = /^(\d+)\t(\{.*\})$/.exec(line);
    if (!m) continue;
    let j;
    try {
      j = JSON.parse(m[2]);
    } catch {
      continue;
    }
    if (j?.point?.id === Number(m[1])) points.push({ id: Number(m[1]), text: String(j.point.text ?? '') });
  }
  const last = points[points.length - 1];
  return last ? { P: String(last.id), P_text: last.text, points } : { P: null, P_text: null, points, why: 'no save point found (torpor show --id 1 gave none)' };
}

export function engineInfo(enginePath, lockText, run) {
  const sha256 = sha256File(enginePath);
  const want = /^asset linux-x64 ([0-9a-f]{64}) (\d+)/m.exec(lockText ?? '');
  const ver = run ? run(enginePath, ['version']) : null;
  return { path: enginePath, sha256, size: fs.statSync(enginePath).size, version_line: ver, lock_sha256: want?.[1] ?? null, matches_lock: want ? want[1] === sha256 : null };
}

export function adapterInfo(adapterPath, repoDir, run) {
  const git = (a) => cp.spawnSync('git', ['-C', repoDir, ...a], { encoding: 'utf8' });
  const head = git(['rev-parse', 'HEAD']);
  const dirty = git(['status', '--porcelain', '--', 'src', 'Cargo.toml', 'Cargo.lock', 'engine.lock']);
  return {
    path: adapterPath, sha256: sha256File(adapterPath), version_line: run ? run(adapterPath, ['version']) : null,
    git_head: head.status === 0 ? head.stdout.trim() : null, source_dirty: dirty.status === 0 ? dirty.stdout.trim().length > 0 : null,
  };
}

// Only for the engine and the adapter (never Muse); the cleared environment keeps them away from the real home.
export const versionLine = (bin, args) => {
  const r = cp.spawnSync(bin, args, { encoding: 'utf8', timeout: 15000, env: { PATH: '/usr/local/bin:/usr/bin:/bin', HOME: '/nonexistent', TORPOR_HOME: '/nonexistent' } });
  return r.status === 0 ? r.stdout.trim() : `exit ${r.status}: ${(r.stderr || r.error?.message || '').trim().slice(0, 120)}`;
};

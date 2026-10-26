#!/usr/bin/env node
// Command line of the disaster-drill harness. Zero dependencies.
//   node run.mjs --plan [IDS] [--verbose]        parse DISASTERS.md and check it; executes nothing
//   node run.mjs build-check [IDS]               build each workspace in a fresh sandboxed folder, check it, delete it
//   node run.mjs run ID --arm N --rep R --out DIR [--attempt N] [--control NC1|NC2] [--scored] [--keep]
//   node run.mjs batch --out DIR [--only IDS] [--reps N] [--negative-control] [--scored]
// Running any of the real disasters (D01-D16) needs --scored, which only the lead passes for the scored runs.
import cp from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as A from './arms.mjs';
import * as C from './contract.mjs';
import { matchText, parseDisasters, predictions, readMd, runOrder } from './parse.mjs';
import { replayRun } from './replay.mjs';
import { GuardError, InfraError, SandboxUnavailable, findBwrap, makeRunFolder, makeSandbox, safeDelete, toolEnv } from './sandbox.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(HERE, '../..');
const LOCKED = /^D(0[1-9]|1[0-6])[a-z]?$/; // the real disasters; anything else is a test disaster
const GATE_DEFAULT = { maxPolls: 12, sleepMs: 5000 }; // PREREG: DISASTERS.md section 5 step 5
const sha = (s) => crypto.createHash('sha256').update(s).digest('hex');
const fileSha = (p) => sha(fs.readFileSync(p));
const now = () => new Date().toISOString();
const die = (code, msg) => {
  console.error(msg);
  process.exit(code);
};

// ------------------------------------------------------------------ arguments and configuration

const BOOL = new Set(['plan', 'scored', 'keep', 'no-sandbox', 'verbose', 'negative-control', 'poison', 'test-drop-canary']);
function parseArgs(argv) {
  const o = { _: [] };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith('--')) o._.push(a);
    else if (BOOL.has(a.slice(2))) o[a.slice(2)] = true;
    else {
      if (i + 1 >= argv.length) die(2, `option ${a} needs a value`);
      o[a.slice(2)] = argv[++i];
    }
  }
  return o;
}

function config(o) {
  const workspaces = path.resolve(o.workspaces ?? path.join(REPO, 'drill/workspaces'));
  const cfg = {
    o, repo: REPO,
    disastersPath: path.resolve(o.disasters ?? path.join(REPO, 'drill/DISASTERS.md')),
    preregPath: path.resolve(o.prereg ?? path.join(REPO, 'drill/PREREG.md')),
    workspaces, commonWs: path.join(REPO, 'drill/workspaces'), oracles: path.join(REPO, 'drill/oracles'),
    out: o.out ? path.resolve(o.out) : null, tmpBase: o['tmp-base'] ?? '/tmp', keep: Boolean(o.keep),
    contract: o['deny-contract'] ?? 'binary', gate: { ...GATE_DEFAULT }, retrySleepMs: 2000,
  };
  if (!['binary', 'strict'].includes(cfg.contract)) die(2, '--deny-contract is binary or strict');
  if (o['gate-polls'] !== undefined) cfg.gate.maxPolls = Number(o['gate-polls']);
  if (o['gate-sleep-ms'] !== undefined) cfg.gate.sleepMs = Number(o['gate-sleep-ms']);
  if (o['retry-sleep-ms'] !== undefined) cfg.retrySleepMs = Number(o['retry-sleep-ms']);
  cfg.gateOverridden = cfg.gate.maxPolls !== GATE_DEFAULT.maxPolls || cfg.gate.sleepMs !== GATE_DEFAULT.sleepMs;
  cfg.disastersMd = readMd(cfg.disastersPath);
  const parsed = parseDisasters(cfg.disastersMd);
  cfg.disasters = parsed.disasters;
  cfg.parseErrors = parsed.errors;
  cfg.predictions = predictions(cfg.disastersMd);
  cfg.preregMd = fs.existsSync(cfg.preregPath) ? readMd(cfg.preregPath) : '';
  cfg.order = runOrder(cfg.preregMd);
  return cfg;
}

const pathsFor = (cfg, d) => ({
  workspaces: cfg.workspaces, oracles: cfg.oracles, shared: path.join(cfg.workspaces, '_assets'),
  assets: path.join(cfg.workspaces, d.id, 'assets'),
});
const pick = (cfg, ids) => {
  if (!ids?.length) return cfg.disasters;
  const out = ids.map((id) => cfg.disasters.find((d) => d.id === id) ?? die(2, `no disaster ${id} in ${cfg.disastersPath}`));
  return out;
};
const classOf = (d) => d.oracle?.kind ?? 'files';
const notRunnable = (d) => (d.turns?.some((t) => t.items.some((i) => i.kind === 'compact')) || classOf(d) === 'memory'
  ? 'the script needs a real /compact and a real model (compact: / memory oracle): live only'
  : null);

// ------------------------------------------------------------------ --plan

const canonCall = (item, content) => {
  if (item.tool === 'bash') return C.bashInput({ cwd: '/tmp/drill.XXXXXX/ws' }, item.input.command);
  return item.tool === 'edit_file' ? C.canon({ path: item.input.path, find: item.input.find, replace: item.input.replace }) : C.canon({ path: item.input.path, content });
};

function staticChecks(cfg, d) {
  const bad = [];
  const where = (it) => `${d.id} line ${it.line}`;
  const calls = d.turns.flatMap((t) => t.items.filter((i) => i.kind === 'call').map((i) => ({ ...i, turn: t })));
  const assetPath = (spec) => spec.replace(/^\$ASSETS(?=\/)/, pathsFor(cfg, d).assets).replace(/^\$SHARED(?=\/)/, pathsFor(cfg, d).shared);
  const contentOf = (i) => {
    if (i.tool !== 'write_file') return undefined;
    const f = assetPath(i.input.content_from);
    return fs.existsSync(f) ? fs.readFileSync(f, 'utf8') : '';
  };
  if (!fs.existsSync(path.join(cfg.workspaces, d.id, 'build.sh'))) bad.push(`${d.id}: no workspaces/${d.id}/build.sh`);
  if (d.trigger) {
    const re = new RegExp(d.trigger);
    const first = (view) => view.find((c) => !c.turn.setup && re.test(c.tool === 'bash' ? c.input.command : JSON.stringify(canonCall(c, contentOf(c)))));
    const v12 = calls.filter((c) => !c.arms34);
    const f12 = first(v12);
    const f34 = first(calls);
    if (!f34) bad.push(`${d.id}: the trigger /${d.trigger}/ matches no call in a turn that is not a setup turn`);
    else if (f12 !== f34) bad.push(`${where(f34)}: the trigger first matches an [arms 3 4] call, so arms 1 and 2 would pick a different disaster call`);
    for (const c of calls) {
      if (c.turn.setup) continue;
      const content = contentOf(c);
      const script = re.test(matchText(c, content));
      const payload = re.test(c.tool === 'bash' ? JSON.stringify(canonCall(c)) : JSON.stringify(canonCall(c, content)));
      const raw = c.tool === 'bash' ? re.test(c.input.command) : script;
      if (script !== payload || raw !== payload) bad.push(`${where(c)}: the trigger gives a different answer on the script text (${raw}) and on JSON.stringify(tool_input) (${payload}), which is what the tap tests`);
    }
    // a setup turn must never contain the first match by accident: informational only (setup turns are skipped)
  }
  for (const t of d.turns) {
    for (const it of t.items) {
      const text = it.cmd ?? it.text ?? (it.kind === 'call' ? JSON.stringify(it.input) : '');
      if (it.kind !== 'between' && /\bC0\b/.test(text)) bad.push(`${where(it)}: the word C0 is only allowed in a between: line`);
      if (text.includes('{NONCE}') && classOf(d) !== 'memory') bad.push(`${where(it)}: {NONCE} outside a memory disaster`);
      if (it.kind === 'call' && it.tool === 'write_file') {
        let f;
        try {
          f = assetPath(it.input.content_from);
        } catch {
          f = null;
        }
        if (!f || !path.isAbsolute(f) || !fs.existsSync(f)) bad.push(`${where(it)}: content_from ${it.input.content_from} does not exist`);
      }
      for (const m of text.matchAll(/\$\{?(ASSETS|SHARED)\}?\/([\w./-]+)/g)) {
        const f = path.join(pathsFor(cfg, d)[m[1].toLowerCase()], m[2]);
        if (!fs.existsSync(f)) bad.push(`${where(it)}: $${m[1]}/${m[2]} does not exist`);
      }
    }
  }
  const needsHeldOut = classOf(d) === 'regression' || d.precondition;
  if (needsHeldOut && !fs.existsSync(path.join(cfg.workspaces, d.id, 'held-out/check.mjs'))) bad.push(`${d.id}: no held-out/check.mjs`);
  if (classOf(d) === 'regression' && d.oracle.check !== 'held-out/check.mjs') bad.push(`${d.id}: the regression oracle names ${d.oracle.check}, the harness runs held-out/check.mjs`);
  if (Object.keys(cfg.predictions).length && LOCKED.test(d.id) && !cfg.predictions[d.id]) bad.push(`${d.id}: no row in the prediction table`);
  return bad;
}

function plan(cfg, ids, verbose) {
  const list = pick(cfg, ids);
  let defects = 0;
  const errors = cfg.parseErrors.filter((e) => !ids?.length || ids.some((id) => e.includes(id)));
  for (const d of list) {
    if (!d.turns) continue;
    const view = (arm34) => d.turns.flatMap((t) => t.items.filter((i) => arm34 || !i.arms34).map((i) => ({ t, i })));
    const n12 = view(false).length;
    const n34 = view(true).length;
    const nr = notRunnable(d);
    console.log(`${d.id} ${d.title} [${classOf(d)}] turns=${d.turns.length} (setup ${d.turns.filter((t) => t.setup).length}) trigger=${d.trigger ? `/${d.trigger}/` : '-'} recovery=${d.recovery.kind}${d.recovery.withData ? '+data' : ''}`);
    console.log(`  steps: arms 1-2 ${n12}, arms 3-4 ${n34} (${n34 - n12} lines marked [arms 3 4])${nr ? `; replay: NOT RUNNABLE (${nr})` : ''}`);
    if (verbose) {
      for (const [label, arm34] of [['arms 1-2', false], ['arms 3-4', true]]) {
        for (const { t, i } of view(arm34)) {
          const what = i.kind === 'call' ? `${i.tool}: ${i.tool === 'bash' ? i.input.command : JSON.stringify(i.input)}` : `${i.kind}: ${i.text ?? i.cmd ?? ''}`;
          console.log(`    ${label} t${t.n}${t.setup ? ' setup' : ''} L${i.line} ${what.slice(0, 150)}`);
        }
      }
    }
  }
  const sc = list.flatMap((d) => (d.turns ? staticChecks(cfg, d) : []));
  const real = cfg.disasters.filter((d) => LOCKED.test(d.id)).map((d) => d.id).sort();
  if (cfg.order && real.length && !ids?.length) {
    const a = [...cfg.order].sort().join();
    const b = real.join();
    if (a !== b) sc.push(`the Run order in PREREG.md lists ${a}, the disaster list has ${b}`);
  }
  for (const e of errors) console.log(`PARSE ERROR ${e}`);
  for (const e of sc) console.log(`STATIC CHECK FAILED ${e}`);
  defects = sc.length;
  console.log(`disasters: ${list.length}; parse errors: ${errors.length}; static check defects: ${defects}`);
  return errors.length + defects === 0;
}

// ------------------------------------------------------------------ one run

// Muse writes under the real home, so it is only ever started inside the sandbox: `--version` runs in a throwaway run folder.
function museVersion(cfg, bin) {
  if (!cfg.bwrap) return 'not run: Muse is never started outside the sandbox (--no-sandbox)';
  const T = makeRunFolder(cfg.tmpBase);
  try {
    for (const dir of ['home', 'tmp', 'muse-share']) fs.mkdirSync(path.join(T, dir));
    const r = makeSandbox({ T, bwrap: cfg.bwrap }).exec({ argv: [bin, '--version'], cwd: T, env: toolEnv(T), ro: [path.dirname(bin)], merge: true, timeoutMs: 30000 });
    return r.exit === 0 ? r.stdout.trim().split('\n')[0] : `exit ${r.exit}: ${r.stdout.trim().slice(0, 120)}`;
  } finally {
    safeDelete(T);
  }
}

function provenance(cfg) {
  if (cfg.prov) return cfg.prov;
  const lockPath = path.join(REPO, 'engine.lock');
  const lock = fs.existsSync(lockPath) ? fs.readFileSync(lockPath, 'utf8') : '';
  const o = cfg.o;
  const enginePath = o.engine ?? process.env.TORPOR_BIN ?? null;
  const adapterPath = o.adapter ?? (fs.existsSync(path.join(REPO, 'target/release/torpor-muse')) ? path.join(REPO, 'target/release/torpor-muse') : null);
  const git = (a) => cp.spawnSync('git', ['-C', REPO, ...a], { encoding: 'utf8' });
  const mainRef = git(['rev-parse', '--verify', '-q', 'origin/main']);
  cfg.prov = {
    engine: enginePath && fs.existsSync(enginePath) ? A.engineInfo(path.resolve(enginePath), lock, A.versionLine) : null,
    adapter: adapterPath && fs.existsSync(adapterPath)
      ? { ...A.adapterInfo(path.resolve(adapterPath), REPO, A.versionLine), origin_main: mainRef.status === 0 ? mainRef.stdout.trim() : null }
      : null,
    muse: {
      started: false,
      note: 'replay mode starts no Muse process and calls no model; payload shapes follow the G0 recorded fixtures of Muse Code',
      binary: o['muse-bin'] ? { path: o['muse-bin'], sha256: fs.existsSync(o['muse-bin']) ? fileSha(o['muse-bin']) : null, version_line: fs.existsSync(o['muse-bin']) ? museVersion(cfg, path.resolve(o['muse-bin'])) : null } : null,
    },
  };
  return cfg.prov;
}

function harnessId() {
  const files = ['parse.mjs', 'contract.mjs', 'sandbox.mjs', 'arms.mjs', 'replay.mjs', 'run.mjs'].map((f) => `${f}:${fileSha(path.join(HERE, f))}`);
  const git = cp.spawnSync('git', ['-C', REPO, 'rev-parse', 'HEAD'], { encoding: 'utf8' });
  return { sha256: sha(files.join('\n')), files, repo_head: git.status === 0 ? git.stdout.trim() : null };
}

const append = (cfg, rec) => fs.appendFileSync(path.join(cfg.out, 'runs.jsonl'), `${JSON.stringify(rec)}\n`);

// -> { kind: 'ok' | 'guard' | 'infra', S, error, ... }
function attemptRun(cfg, d, arm, rep, attempt, control, poison) {
  const key = `${d.id}-a${arm}-r${rep}-t${attempt}${control ? `-${control}` : ''}`;
  const outDir = path.join(cfg.out, 'runs', key);
  fs.mkdirSync(outDir, { recursive: true });
  const rec = { started_at: now(), key, programs: null, arm_files: null };
  let T = null;
  let sb = null;
  const prov = provenance(cfg);
  try {
    if (arm >= 3 && !prov.engine) throw new InfraError('the engine binary is missing: pass --engine PATH (or set TORPOR_BIN)');
    if (arm === 4 && !prov.adapter) throw new InfraError('the adapter binary is missing: build it (cargo build --release) or pass --adapter PATH');
    T = makeRunFolder(cfg.tmpBase);
    if (cfg.o['test-drop-canary'] && !LOCKED.test(d.id)) fs.rmSync(path.join(T, '.drill-canary'));
    sb = makeSandbox({ T, bwrap: cfg.bwrap });
    const build = sb.exec({
      argv: ['sh', path.join(cfg.workspaces, d.id, 'build.sh'), T], cwd: T, env: toolEnv(T), ro: [...new Set([cfg.commonWs, cfg.workspaces])], merge: true, timeoutMs: 180000,
    });
    rec.build = { exit: build.exit, ms: build.ms, output: build.stdout.slice(0, 600) };
    if (build.exit === 2 && /^refused: /m.test(build.stdout)) throw new GuardError(`the builder refused the folder: ${build.stdout.trim().slice(0, 200)}`);
    if (build.exit !== 0) throw new InfraError(`the builder failed (exit ${build.exit}): ${build.stdout.trim().slice(0, 300)}`);
    const ws = path.join(T, 'ws');
    if (!fs.existsSync(path.join(ws, '.synthetic')) || !fs.existsSync(path.join(T, '.synthetic'))) throw new InfraError('the builder left no .synthetic marker');
    fs.writeFileSync(path.join(T, '.drill-run'), `${JSON.stringify({ id: d.id, arm, rep, mode: 'replay', attempt, control: control ?? null })}\n`);
    for (const dir of ['home/.config', 'tmp', 'torpor-home', 'muse-share']) fs.mkdirSync(path.join(T, dir), { recursive: true });
    fs.writeFileSync(path.join(T, 'home/.gitconfig'), '[user]\n\tname = drill\n\temail = drill.invalid\n[core]\n\thooksPath = /dev/null\n[commit]\n\tgpgsign = false\n');
    let hooks = {};
    if (arm >= 3) {
      rec.programs = A.installPrograms(T, arm, { engine: prov.engine.path, adapter: prov.adapter?.path });
      const w = arm === 3 ? A.writeArm3Settings(T) : A.runArm4Init(T, sb);
      const st = A.parseSettings(w.text);
      hooks = st.hooks;
      rec.arm_files = { settings: JSON.parse(w.text), init: w.init, warnings: st.warnings, events: Object.keys(hooks) };
    }
    const S = replayRun({
      d, arm, T, ws, sb, paths: pathsFor(cfg, d), hooks, contract: cfg.contract, poison: Boolean(poison), gate: cfg.gate, outDir,
    });
    return { kind: 'ok', S, rec, outDir };
  } catch (e) {
    const kind = e instanceof GuardError ? 'guard' : 'infra';
    if (!(e instanceof GuardError) && !(e instanceof InfraError)) {
      // any other exception is a defect of the harness itself: treat as infra, keep the stack
      rec.stack = String(e.stack ?? e).split('\n').slice(0, 6).join('\n');
    }
    return { kind, S: e.partial ?? null, rec, outDir, error: e.message };
  } finally {
    if (sb) sb.reapRun();
    if (T && !cfg.keep) safeDelete(T);
  }
}

function toRecord(cfg, d, arm, rep, attempt, control, poison, r) {
  const S = r.S;
  const prov = provenance(cfg);
  const ok = r.kind === 'ok';
  const outcome = ok ? S.outcome : null;
  return {
    schema: 'torpor-drill-run/1', batch: cfg.batchId, disaster: d.id, title: d.title, class: classOf(d), arm, arm_name: A.ARM_NAMES[arm - 1],
    control: control ?? null, repetition: rep, mode: 'replay', attempt, outcome, outcome_reason: ok ? S.reason ?? null : r.error,
    would_be_outcome: ok ? S.would_be ?? null : null, oracle: ok ? S.oracle : null,
    started_at: r.rec.started_at, ended_at: now(),
    engine: arm >= 3 ? prov.engine : null, adapter: arm === 4 ? prov.adapter : null, muse: prov.muse,
    contract: { name: cfg.contract, rules: Object.fromEntries(C.RULES.map((x) => [x.id, x.status])), unverified_used: S?.unverified ?? [] },
    gate: S?.gate ?? null, gate_config: { max_polls: cfg.gate.maxPolls, sleep_ms: cfg.gate.sleepMs, overridden: cfg.gateOverridden },
    trigger: S?.trigger ?? null, denied: S?.denied ?? null, precondition: S?.precondition ?? null,
    manifests: S?.manifests ?? null, facts: S?.facts ?? null, recovery: S?.recovery ?? null, baseline_lines: S?.baseline?.lines ?? null, final_lines: S?.final?.lines ?? null,
    script_warnings: S?.warnings ?? [], notes: S?.notes ?? [], trace: S?.trace ?? [], tap: S?.tap ?? [],
    build: r.rec.build ?? null, programs: r.rec.programs, arm_setup: r.rec.arm_files, stack: r.rec.stack ?? null,
    side_files: path.relative(cfg.out, r.outDir),
    assumptions: {
      session_log: 'emulated from the adapter\'s own reading code (src/transcript.rs): a user_intent per prompt and the accepted additional context; Stop block reasons are not logged',
      claudecode_in_bash: 'unset (NV whether Muse sets it)', one_call_one_batch: 'each scripted call is its own PostToolBatch (assumed)', partial: !ok,
    },
  };
}

// Runs one slot (disaster, arm, repetition) with the infra retry rule of PREREG section 6; appends every attempt.
function runSlot(cfg, d, arm, rep, control, poison, firstAttempt = 1) {
  let last = null;
  for (let attempt = firstAttempt; attempt <= 3; attempt++) {
    const r = attemptRun(cfg, d, arm, rep, attempt, control, poison);
    const rec = toRecord(cfg, d, arm, rep, attempt, control, poison, r);
    if (r.kind === 'guard') rec.outcome = 'INVALID';
    if (r.kind === 'infra') rec.outcome = attempt < 3 ? 'INFRA-FAILURE' : 'INVALID';
    if (r.kind !== 'ok') rec.outcome_reason = r.error;
    append(cfg, rec);
    last = rec;
    console.log(`${d.id} arm ${arm} rep ${rep} attempt ${attempt}${control ? ` ${control}` : ''} -> ${rec.outcome}${rec.oracle ? ` [${rec.oracle.line}]` : ''}${rec.outcome_reason ? ` (${rec.outcome_reason.slice(0, 160)})` : ''}`);
    if (r.kind !== 'infra') break;
    if (attempt < 3) cp.spawnSync('sleep', [String(cfg.retrySleepMs / 1000)]);
  }
  return last;
}

// ------------------------------------------------------------------ guards before anything is built

// -> the reason to refuse, or null. Pure, so the self-test can try it without starting anything.
function refusal(list, o, gateOverridden, forRun) {
  const locked = list.filter((d) => LOCKED.test(d.id));
  if (forRun && locked.length && !o.scored) {
    return `refusing to run ${locked.map((d) => d.id).join(', ')}: the real disasters need --scored (lead only); nothing was built or executed`;
  }
  if (locked.length && (o['no-sandbox'] || gateOverridden || o.poison || o['test-drop-canary'])) {
    return 'refusing: --no-sandbox, the gate poll overrides, --poison and test-only options are refused for the real disasters';
  }
  // PREREG section 3 fixes the mode once, before the first scored run; the deny contract defines what "honours a deny" means in replay, so it is never a silent default there.
  if (forRun && locked.length && !o['deny-contract']) {
    return 'refusing: a scored run needs an explicit --deny-contract binary|strict (it is part of the replay mode, which PREREG section 3 fixes once, before the first scored run; see drill/harness/README.md)';
  }
  return null;
}

function prepare(cfg, ids, { forRun }) {
  const list = pick(cfg, ids);
  const why = refusal(list, cfg.o, cfg.gateOverridden, forRun);
  if (why) die(3, why);
  if (cfg.o['no-sandbox']) cfg.bwrap = null;
  else {
    try {
      cfg.bwrap = findBwrap(process.env);
    } catch (e) {
      if (e instanceof SandboxUnavailable) die(3, e.message);
      throw e;
    }
  }
  return list;
}

// ------------------------------------------------------------------ build-check

function buildCheck(cfg, ids) {
  const list = prepare(cfg, ids, { forRun: false });
  let bad = 0;
  for (const d of list) {
    let T = null;
    try {
      T = makeRunFolder(cfg.tmpBase);
      const sb = makeSandbox({ T, bwrap: cfg.bwrap });
      const b = sb.exec({ argv: ['sh', path.join(cfg.workspaces, d.id, 'build.sh'), T], cwd: T, env: toolEnv(T), ro: [...new Set([cfg.commonWs, cfg.workspaces])], merge: true, timeoutMs: 180000 });
      if (b.exit !== 0) throw new Error(`builder exit ${b.exit}: ${b.stdout.trim().slice(0, 200)}`);
      const must = ['ws/.synthetic', '.synthetic'];
      if (d.oracle?.kind === 'files') must.push(d.oracle.root);
      const head = sb.exec({ argv: ['git', '-C', path.join(T, 'ws'), 'rev-parse', 'HEAD'], cwd: T, env: toolEnv(T), merge: true });
      if (head.exit !== 0) throw new Error('the workspace has no base commit');
      for (const m of must) if (!fs.existsSync(path.join(T, m))) throw new Error(`missing after the build: ${m}`);
      if (fs.existsSync(path.join(T, '.drill-canary'))) throw new Error('the builder left the canary');
      console.log(`${d.id} build ok (${b.ms} ms)`);
    } catch (e) {
      bad++;
      console.log(`${d.id} BUILD CHECK FAILED: ${e.message}`);
    } finally {
      if (T) safeDelete(T);
    }
  }
  console.log(`build-check: ${list.length - bad}/${list.length} ok`);
  return bad === 0;
}

// ------------------------------------------------------------------ batch

function header(cfg) {
  const prov = provenance(cfg);
  return {
    schema: 'torpor-drill-runlog/1', mode: 'replay', deny_contract: { name: cfg.contract, rules: C.RULES.map((r) => ({ id: r.id, status: r.status, text: r.text })) },
    gate: { max_polls: cfg.gate.maxPolls, sleep_ms: cfg.gate.sleepMs, overridden: cfg.gateOverridden },
    prereg_sha256: fs.existsSync(cfg.preregPath) ? fileSha(cfg.preregPath) : null, disasters_sha256: fileSha(cfg.disastersPath),
    harness: harnessId(), provenance: prov, node: process.version, umask: '022', sandbox: cfg.bwrap ? 'bwrap' : 'none (--no-sandbox)',
    reps: Number(cfg.o.reps ?? 3),
  };
}

function batch(cfg, ids) {
  const list = prepare(cfg, ids, { forRun: true });
  if (!cfg.out) die(2, 'batch needs --out DIR');
  fs.mkdirSync(cfg.out, { recursive: true });
  const h = header(cfg);
  const hp = path.join(cfg.out, 'run-log.json');
  if (fs.existsSync(hp)) {
    const old = JSON.parse(fs.readFileSync(hp, 'utf8'));
    if (JSON.stringify(old.header) !== JSON.stringify(h)) die(3, `run-log.json in ${cfg.out} differs from this invocation (deny contract, poll values, binaries or harness changed): use a fresh --out`);
    cfg.batchId = old.batch_id;
  } else {
    cfg.batchId = crypto.randomUUID();
    fs.writeFileSync(hp, `${JSON.stringify({ batch_id: cfg.batchId, created_at: now(), header: h }, null, 2)}\n`);
  }
  const runs = path.join(cfg.out, 'runs.jsonl');
  const prior = fs.existsSync(runs) ? fs.readFileSync(runs, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l)) : [];
  const slotKey = (id, arm, rep, c) => `${id}/${arm}/${rep}/${c ?? ''}`;
  const done = new Set(prior.filter((r) => r.outcome !== 'INFRA-FAILURE').map((r) => slotKey(r.disaster, r.arm, r.repetition, r.control)));
  const attempts = (id, arm, rep, c) => prior.filter((r) => slotKey(r.disaster, r.arm, r.repetition, r.control) === slotKey(id, arm, rep, c)).length;
  const reps = Number(cfg.o.reps ?? 3);
  let ran = 0;
  const slots = [];
  if (cfg.o['negative-control']) {
    for (const [c, id] of [['NC1', 'D06'], ['NC2', 'D16']]) {
      const d = cfg.disasters.find((x) => x.id === id) ?? die(2, `negative control ${c} needs ${id}`);
      for (let rep = 1; rep <= reps; rep++) slots.push({ d, arm: 4, rep, control: c, poison: true });
    }
  } else {
    const ordered = ids?.length ? list : (cfg.order ?? list.map((d) => d.id)).map((id) => cfg.disasters.find((d) => d.id === id)).filter(Boolean);
    for (const d of ordered) for (let rep = 1; rep <= reps; rep++) for (const arm of [1, 2, 3, 4]) slots.push({ d, arm, rep, control: null, poison: Boolean(cfg.o.poison) });
  }
  for (const s of slots) {
    if (done.has(slotKey(s.d.id, s.arm, s.rep, s.control))) continue;
    const nr = notRunnable(s.d);
    if (nr) {
      append(cfg, { schema: 'torpor-drill-run/1', batch: cfg.batchId, disaster: s.d.id, arm: s.arm, repetition: s.rep, mode: 'replay', attempt: 1, control: s.control, outcome: 'NOT-RUN', outcome_reason: `replay cannot play this disaster: ${nr}`, started_at: now(), ended_at: now() });
      continue;
    }
    runSlot(cfg, s.d, s.arm, s.rep, s.control, s.poison, attempts(s.d.id, s.arm, s.rep, s.control) + 1);
    ran++;
  }
  console.log(`batch: ${ran} slots run; records in ${runs}`);
  return true;
}

function single(cfg, id) {
  if (!id) die(2, 'run needs a disaster id');
  const arm = Number(cfg.o.arm);
  const rep = Number(cfg.o.rep ?? 1);
  if (![1, 2, 3, 4].includes(arm)) die(2, 'run needs --arm 1|2|3|4');
  if (!cfg.out) die(2, 'run needs --out DIR');
  const list = prepare(cfg, [id], { forRun: true });
  const d = list[0];
  const control = cfg.o.control ?? null;
  if (control) {
    const want = { NC1: ['D06', 4], NC2: ['D16', 4] }[control];
    if (!want || (LOCKED.test(id) && (want[0] !== id || want[1] !== arm))) die(2, `--control ${control} is for ${want?.join(' arm ')}`);
  }
  const nr = notRunnable(d);
  fs.mkdirSync(cfg.out, { recursive: true });
  cfg.batchId = `single-${crypto.randomUUID()}`;
  if (nr) {
    append(cfg, { schema: 'torpor-drill-run/1', batch: cfg.batchId, disaster: id, arm, repetition: rep, mode: 'replay', attempt: 1, control, outcome: 'NOT-RUN', outcome_reason: `replay cannot play this disaster: ${nr}`, started_at: now(), ended_at: now() });
    console.log(`${id} arm ${arm} rep ${rep} -> NOT-RUN (${nr})`);
    return true;
  }
  runSlot(cfg, d, arm, rep, control, Boolean(control) || Boolean(cfg.o.poison), Number(cfg.o.attempt ?? 1));
  return true;
}

// ------------------------------------------------------------------ main

function main() {
  // The builders run with umask 022 and the files oracle compares file modes: everything the harness starts (tool calls,
  // hooks, recovery, its own file writes) uses the same umask, else a restore by git would show up as a mode-only loss.
  process.umask(0o022);
  const o = parseArgs(process.argv.slice(2));
  if (o.mode && o.mode !== 'replay') die(2, `--mode ${o.mode} is not implemented: echo mode is not possible (see drill/harness/README.md, "Echo mode"); live mode is not part of this harness`);
  const cfg = config(o);
  const [cmd, ...rest] = o._;
  let ok;
  if (o.plan) ok = plan(cfg, o._, o.verbose);
  else if (cmd === 'build-check') ok = buildCheck(cfg, rest);
  else if (cmd === 'run') ok = single(cfg, rest[0]);
  else if (cmd === 'batch') ok = batch(cfg, o.only ? o.only.split(',') : null);
  else die(2, 'usage: run.mjs --plan [IDS] [--verbose] | build-check [IDS] | run ID --arm N --rep R --out DIR | batch --out DIR [--only IDS] [--negative-control]');
  process.exit(ok ? 0 : 1);
}

if (process.argv[1] && import.meta.url === new URL(`file://${fs.realpathSync(process.argv[1])}`).href) main();
export { LOCKED, config, plan, refusal, staticChecks };

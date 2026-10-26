// What replay mode believes about Muse Code's hooks: payload shapes and the rules for honouring an answer.
// Every rule carries a status, copied into the batch header and every run record, so a reader sees which
// rules rest on a G0 run and which do not:
//   G0       observed in a G0 run (research/59, the G0 facts note and its recorded fixtures)
//   SHIM     observed by S3a-shim in live echo runs (the coordinator's message of Oct 26)
//   BIN      only the strings of the Muse 1.4.13 binary say so (behaviour NOT RUN)
//   DOCS     only Meta's documentation says so (not run)
//   ASSUMED  a harness decision; nothing observed
// The brief's rule: a case G0 did not test is NOT VERIFIED and is treated as not honoured (fail open). Where a rule
// below is BIN, DOCS or ASSUMED and it is the reason an answer was NOT applied, the tap row says so (`unverified`).
import crypto from 'node:crypto';

export const MUSE = { model: 'muse-spark-1.3', provider: 'meta', permissionMode: 'bypassPermissions' };
export const OUT_CAP = 16384; // bytes, stdout and stderr each (G0 D12)
export const STOP_CAP = 8; // continuations per turn (G0 D8)
export const SYSTEM_MESSAGE_CAP = 1000; // chars stored (G0)
export const BASH_OUTPUT_CAP = 40000; // chars (G0 D12)

export const KNOWN_EVENTS = [
  'SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PermissionRequest', 'PostToolUse', 'PostToolUseFailure',
  'PostToolBatch', 'PreLLMCall', 'PostLLMCall', 'PreCompact', 'PostCompact', 'SubagentStart', 'SubagentStop',
  'Stop', 'StopFailure', 'SessionEnd', 'Notification', 'SessionFork', 'ToolUseStart', 'Interrupt',
];

export const RULES = [
  ['stdout-json', 'G0', 'stdout is parsed only at exit 0 and only if it starts with "{"; bare true/42/"block" and plain text are ignored'],
  ['noise-ups', 'SHIM', 'UserPromptSubmit (and SessionStart): plain text, or noise before the JSON, is taken as plain context; on Stop it is ignored'],
  ['closed-schema', 'G0', 'the top-level schema is closed and camelCase; one unknown key rejects the whole output: status failed, fail open'],
  ['nested-schema', 'ASSUMED', 'an unknown key inside hookSpecificOutput rejects the whole output too'],
  ['block-reason', 'G0', 'decision "block" needs a non-empty reason, else failed'],
  ['stop-context', 'G0', 'additionalContext on Stop is rejected (failed); assumed the same for the other events whose context column is "no"'],
  ['exit-codes', 'G0', 'exit 1, or exit 2 with an empty stderr, is failed (fail open)'],
  ['stop-block', 'G0', 'Stop is blocked by exit 2 with a stderr text, or by exit 0 and {"decision":"block","reason":R}; the model continues and the next Stop has stop_hook_active true'],
  ['stop-cap', 'G0', 'at most 8 Stop continuations per turn; the 9th block is ignored'],
  ['continue-false', 'G0', '{"continue":false,"stopReason":S} on Stop ends the turn'],
  ['system-message', 'G0', 'systemMessage is accepted and stored cut to 1000 chars'],
  ['output-cap', 'G0', 'stdout or stderr over 16384 bytes (encoded) is failed (output_too_large)'],
  ['timeout', 'G0', 'timeout is an integer in seconds; at the timeout the process group is killed and the session goes on without an answer'],
  ['input-size', 'SHIM', 'a payload over 256 KiB IS delivered (1.5 MB seen); replay has no input size cap'],
  ['settings', 'G0', 'a malformed settings file, or schema_version other than 1, makes Muse refuse to start; a bad timeout drops the whole hooks source; other per-hook mistakes skip that hook'],
  ['pre-deny', 'BIN', 'PreToolUse hookSpecificOutput.permissionDecision "deny" with a non-empty permissionDecisionReason stops the call from running (strict contract: never honoured)'],
  ['pre-legacy', 'BIN', 'PreToolUse top-level decision "block" is an "unsupported legacy output": failed, fail open, the call runs'],
  ['pre-exit2', 'DOCS', 'PreToolUse exit 2 with a stderr text is documented as blocking; NOT honoured (call runs), recorded'],
  ['pre-values', 'BIN', 'PreToolUse: "allow" without updatedInput, an unknown permissionDecision, or a reason without a decision is failed'],
  ['pre-continue', 'DOCS', 'PreToolUse continue/stopReason is "rejected" by the schema: failed, fail open'],
  ['other-block', 'ASSUMED', 'a block (decision "block" or exit 2) on an event other than Stop is recorded, not applied'],
  ['denied-call-events', 'ASSUMED', 'after a denied call no PostToolUse, PostToolUseFailure or PostToolBatch event is fired for it'],
  ['one-call-one-batch', 'ASSUMED', 'each scripted call is its own tool batch: PreToolUse, the tool, PostToolUse or PostToolUseFailure, PostToolBatch'],
  ['lenient-ws', 'ASSUMED', 'leading whitespace before the JSON counts as plain text (not parsed)'],
  ['session-log', 'ASSUMED', 'the harness writes a Muse-shaped session.jsonl with only the record kinds the adapter reads (runtime.user_intent.accepted, context_block_updated)'],
].map(([id, status, text]) => ({ id, status, text }));

const STATUS = Object.fromEntries(RULES.map((r) => [r.id, r.status]));
export const ruleStatus = (id) => STATUS[id] ?? 'ASSUMED';

// ------------------------------------------------------------------ ids and payloads

export function uuidv7(now = Date.now()) {
  const b = crypto.randomBytes(16);
  b.writeUIntBE(now, 0, 6);
  b[6] = (b[6] & 0x0f) | 0x70;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = b.toString('hex');
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}
export const toolUseId = () => `call_${uuidv7().replace(/-/g, '')}`;

// Muse's payloads have alphabetically sorted keys (G0 fixtures); arrays and strings stay as they are.
export function canon(v) {
  if (Array.isArray(v)) return v.map(canon);
  if (v && typeof v === 'object') return Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])]));
  return v;
}

export function newSession(cwd) {
  return { cwd, sid: uuidv7(), thread: uuidv7(), turn: null, bashSeq: 0 };
}

const common = (s, event) => ({
  cwd: s.cwd, hook_event_name: event, model: MUSE.model, model_provider: MUSE.provider,
  permission_mode: MUSE.permissionMode, session_id: s.sid, transcript_path: null,
});
const withTurn = (s, event, extra) => canon({ ...common(s, event), turn_id: s.turn, ...extra });

export const P = {
  sessionStart: (s, source = 'startup') => canon({ ...common(s, 'SessionStart'), source }),
  sessionEnd: (s) => canon({ ...common(s, 'SessionEnd'), reason: 'other' }),
  userPrompt: (s, prompt) => withTurn(s, 'UserPromptSubmit', { prompt }),
  preTool: (s, tool, input, id) => withTurn(s, 'PreToolUse', { tool_input: input, tool_name: tool, tool_use_id: id }),
  postTool: (s, tool, input, id, response) =>
    withTurn(s, 'PostToolUse', { tool_input: input, tool_name: tool, tool_response: response, tool_use_id: id }),
  postToolFailure: (s, tool, input, id, error, ms) =>
    withTurn(s, 'PostToolUseFailure', {
      duration_ms: ms, error, is_interrupt: false, tool_input: input, tool_name: tool, tool_use_id: id,
    }),
  batch: (s, calls) =>
    withTurn(s, 'PostToolBatch', {
      tool_calls: calls.map((c) => ({
        tool_input: c.tool === 'bash' ? { command: c.input.command, description: c.input.description } : c.input,
        tool_name: c.tool, tool_response: c.error ?? c.response, tool_use_id: c.id,
      })),
    }),
  stop: (s, last, active) => withTurn(s, 'Stop', { last_assistant_message: last, stop_hook_active: Boolean(active) }),
};

// A neutral description: the tap tests the JSON of tool_input against the trigger, so it must not repeat the command.
export const BASH_DESCRIPTION = 'Run a scripted step';

export function bashInput(s, command) {
  return canon({ command, description: BASH_DESCRIPTION, workdir: s.cwd });
}

// tool_response of a bash call (G0 fixtures: a STRING holding pretty-printed JSON, key order as Muse writes it).
export function bashResponse(s, command, exit, output, { withCommand = true } = {}) {
  const truncated = output.length > BASH_OUTPUT_CAP;
  const o = truncated ? output.slice(0, BASH_OUTPUT_CAP) : output;
  const bytes = Buffer.byteLength(output);
  const tail = { output: o, original_output_bytes: bytes, original_output_tokens: Math.ceil(bytes / 4), truncated };
  s.bashSeq += 1;
  const chunk = `exec-${s.bashSeq}-1`;
  if (exit === 0) {
    const head = withCommand
      ? { chunk_id: chunk, command, description: BASH_DESCRIPTION, exit_code: 0, terminal_status: 'completed' }
      : { chunk_id: chunk, exit_code: 0, terminal_status: 'completed' };
    return JSON.stringify({ ...head, ...tail }, null, 2);
  }
  return JSON.stringify(
    { chunk_id: chunk, exit_code: exit, terminal_status: 'failed', terminal_reason: `process exited with status ${exit}`, ...tail },
    null, 2,
  );
}

// edit_file: an empty structuredPatch (G0 fixture); write_file: one all-added hunk (G0 fixture of a new file).
export const fileResponse = (abs, content, isWrite) => {
  const lines = String(content).split('\n');
  return canon({
    filePath: abs,
    structuredPatch: isWrite ? [{ lines: lines.map((l) => `+${l}`), newLines: lines.length, newStart: 1, oldLines: 0, oldStart: 1 }] : [],
  });
};

// ------------------------------------------------------------------ matchers

// Anchored regex alternation, case-insensitive: `bash|edit_file` matches tool `Bash`.
export function matcherMatches(matcher, value) {
  if (matcher === undefined || matcher === null || matcher === '') return true;
  if (value === undefined) return true;
  try {
    return new RegExp(`^(?:${matcher})$`, 'i').test(value);
  } catch {
    return false;
  }
}

// ------------------------------------------------------------------ settings.json (G0 D6, D7)

export function loadSettings(text) {
  const warnings = [];
  const refuse = (why) => ({ ok: false, refuse: why, hooks: {}, warnings });
  if (!text || !text.trim()) return refuse('empty settings file');
  let j;
  try {
    j = JSON.parse(text);
  } catch (e) {
    return refuse(`malformed settings file: ${e.message}`);
  }
  if (!j || typeof j !== 'object' || Array.isArray(j)) return refuse('settings is not an object');
  if (j.schema_version !== 1) return refuse(`schema_version ${JSON.stringify(j.schema_version)} (only 1 is accepted)`);
  const hooks = {};
  if (j.hooks === undefined) return { ok: true, hooks, warnings };
  if (!j.hooks || typeof j.hooks !== 'object' || Array.isArray(j.hooks)) {
    warnings.push('hooks is not an object: skipped');
    return { ok: true, hooks, warnings };
  }
  for (const [event, groups] of Object.entries(j.hooks)) {
    if (!KNOWN_EVENTS.includes(event)) {
      warnings.push(`unknown event ${event}: skipped`);
      continue;
    }
    if (!Array.isArray(groups)) {
      warnings.push(`${event}: not a list: skipped`);
      continue;
    }
    for (const g of groups) {
      if (!g || typeof g !== 'object' || !Array.isArray(g.hooks)) {
        warnings.push(`${event}: group without a hooks list: skipped`);
        continue;
      }
      if (g.matcher !== undefined && typeof g.matcher !== 'string') {
        warnings.push(`${event}: matcher is not a string: skipped`);
        continue;
      }
      if (g.matcher !== undefined && event === 'Stop') {
        warnings.push('Stop: a matcher on Stop: skipped');
        continue;
      }
      for (const h of g.hooks) {
        const extra = Object.keys(h ?? {}).filter((k) => !['type', 'command', 'timeout', 'async'].includes(k));
        if (!h || h.type !== 'command' || typeof h.command !== 'string' || !h.command) {
          warnings.push(`${event}: not a command handler: skipped`);
          continue;
        }
        if (extra.length) {
          warnings.push(`${event}: unknown handler field ${extra.join(',')}: skipped`);
          continue;
        }
        if (h.async !== undefined && typeof h.async !== 'boolean') {
          warnings.push(`${event}: async is not a boolean: skipped`);
          continue;
        }
        if (h.timeout !== undefined && (!Number.isInteger(h.timeout) || h.timeout < 0)) {
          return { ok: true, hooks: {}, warnings: [...warnings, `${event}: timeout ${JSON.stringify(h.timeout)} is not a non-negative integer: the WHOLE hooks source is dropped`], dropped: true };
        }
        const timeout = h.timeout === undefined ? 600 : Math.max(1, h.timeout);
        (hooks[event] ??= []).push({ matcher: g.matcher, command: h.command, timeout, async: h.async === true });
      }
    }
  }
  return { ok: true, hooks, warnings };
}

// ------------------------------------------------------------------ answers

const TOP_KEYS = ['decision', 'reason', 'continue', 'stopReason', 'systemMessage', 'suppressOutput', 'hookSpecificOutput'];
const HSO_KEYS = ['hookEventName', 'additionalContext', 'permissionDecision', 'permissionDecisionReason', 'updatedInput', 'selectedSkills'];
const NO_CONTEXT = ['Stop', 'PreCompact', 'PostCompact', 'SubagentStop', 'StopFailure', 'SessionEnd'];
const BLOCK_OK = ['Stop', 'UserPromptSubmit', 'PostToolUse', 'PostToolUseFailure', 'PostToolBatch', 'SubagentStop', 'PreLLMCall', 'PostLLMCall'];

// interpret(event, { exit, stdout, stderr, timedOut }, { contract: 'binary' | 'strict' })
// -> { status, failed, why, json, action: { kind: none|deny|block|stop, reason }, context, systemMessage, rules: [ids used],
//      unverified: [rule ids whose non-G0 status shaped the result] }
export function interpret(event, run, { contract = 'binary' } = {}) {
  const r = { status: 'completed', failed: false, why: null, json: null, action: { kind: 'none' }, context: null, systemMessage: null, rules: [], unverified: [] };
  const used = (id, shaped = false) => {
    if (!r.rules.includes(id)) r.rules.push(id);
    if (shaped && ruleStatus(id) !== 'G0' && !r.unverified.includes(id)) r.unverified.push(id);
  };
  const fail = (why, rule, shaped = true) => {
    r.status = 'failed';
    r.failed = true;
    r.why = why;
    r.action = { kind: 'none' };
    r.context = null;
    used(rule, shaped);
    return r;
  };
  if (run.timedOut) {
    r.status = 'timed_out';
    r.failed = true;
    r.why = 'timed out';
    used('timeout');
    return r;
  }
  if (Buffer.byteLength(run.stdout ?? '') > OUT_CAP || Buffer.byteLength(run.stderr ?? '') > OUT_CAP) {
    return fail('output_too_large (over 16384 bytes)', 'output-cap', false);
  }
  const stdout = run.stdout ?? '';
  const stderr = run.stderr ?? '';
  if (run.exit === 2) {
    used('exit-codes');
    if (!stderr.trim()) return fail('exit 2 with empty stderr', 'exit-codes', false);
    if (event === 'Stop') {
      used('stop-block');
      r.status = 'blocked';
      r.action = { kind: 'block', reason: stderr };
    } else if (event === 'PreToolUse') {
      used('pre-exit2', true);
    } else {
      used('other-block', true);
    }
    return r;
  }
  if (run.exit !== 0) return fail(`exit ${run.exit}`, 'exit-codes', false);
  used('stdout-json');
  if (!stdout.trim()) return r;
  if (!stdout.startsWith('{') && !stdout.startsWith('[')) {
    if (stdout.trim().length && /^\s/.test(stdout)) used('lenient-ws', true);
    if (event === 'UserPromptSubmit' || event === 'SessionStart') {
      used('noise-ups');
      r.context = stdout;
    }
    return r;
  }
  let j;
  try {
    j = JSON.parse(stdout);
  } catch (e) {
    return fail(`stdout starts with "${stdout[0]}" but is not valid JSON (${e.message})`, 'stdout-json');
  }
  if (!j || typeof j !== 'object' || Array.isArray(j)) return fail('hook output must be a JSON object', 'stdout-json');
  r.json = j;
  for (const k of Object.keys(j)) if (!TOP_KEYS.includes(k)) return fail(`unknown key "${k}" in the hook output`, 'closed-schema', false);
  used('closed-schema');
  const typeBad = (k, t) => j[k] !== undefined && typeof j[k] !== t;
  for (const [k, t] of [['decision', 'string'], ['reason', 'string'], ['continue', 'boolean'], ['stopReason', 'string'], ['systemMessage', 'string'], ['suppressOutput', 'boolean']]) {
    if (typeBad(k, t)) return fail(`"${k}" must be a ${t}`, 'closed-schema');
  }
  const hso = j.hookSpecificOutput;
  if (hso !== undefined) {
    if (!hso || typeof hso !== 'object' || Array.isArray(hso)) return fail('hookSpecificOutput must be a JSON object', 'closed-schema');
    for (const k of Object.keys(hso)) if (!HSO_KEYS.includes(k)) return fail(`unknown key "${k}" in hookSpecificOutput`, 'nested-schema');
    used('nested-schema');
    if (hso.additionalContext !== undefined && typeof hso.additionalContext !== 'string') return fail('additionalContext must be a string', 'closed-schema');
    if (hso.additionalContext !== undefined && NO_CONTEXT.includes(event)) {
      return fail(`additionalContext is not allowed on ${event}`, 'stop-context', event !== 'Stop');
    }
  }
  if (j.systemMessage !== undefined) {
    used('system-message');
    r.systemMessage = j.systemMessage.slice(0, SYSTEM_MESSAGE_CAP);
  }
  if (j.decision !== undefined) {
    if (j.decision !== 'block') return fail(`unsupported decision "${j.decision}"`, 'closed-schema');
    if (!j.reason || !j.reason.trim()) return fail('decision "block" requires a non-empty reason', 'block-reason', false);
    used('block-reason');
    if (event === 'PreToolUse') return fail('unsupported legacy PreToolUse output; use hookSpecificOutput.permissionDecision', 'pre-legacy');
    if (!BLOCK_OK.includes(event)) return fail('decision "block" is unsupported for this hook event', 'other-block');
    if (event === 'Stop') {
      used('stop-block');
      r.status = 'blocked';
      r.action = { kind: 'block', reason: j.reason };
    } else used('other-block', true);
  } else if (j.reason !== undefined && event !== 'PreToolUse') {
    used('closed-schema');
  }
  if (j.continue === false) {
    if (event === 'PreToolUse') return fail('continue/stopReason are rejected on PreToolUse', 'pre-continue');
    if (event === 'Stop') {
      used('continue-false');
      r.action = { kind: 'stop', reason: j.stopReason ?? '' };
      r.status = 'completed';
    } else used('other-block', true);
  } else if (j.continue === true || j.stopReason !== undefined) {
    if (event === 'PreToolUse') return fail('continue/stopReason are rejected on PreToolUse', 'pre-continue');
  }
  if (hso) {
    const { permissionDecision: pd, permissionDecisionReason: why, updatedInput } = hso;
    if (pd !== undefined || why !== undefined || updatedInput !== undefined) {
      if (event !== 'PreToolUse') return fail(`permissionDecision is not supported on ${event}`, 'pre-values');
      if (pd === undefined && why !== undefined) return fail('permissionDecisionReason without a permissionDecision', 'pre-values');
      if (pd === 'deny') {
        if (typeof why !== 'string' || !why.trim()) return fail('permissionDecision "deny" requires a non-empty permissionDecisionReason', 'pre-values');
        used('pre-deny', true);
        if (contract === 'binary') {
          r.status = 'blocked';
          r.action = { kind: 'deny', reason: why };
        }
      } else if (pd === 'allow') {
        if (updatedInput === undefined) return fail('permissionDecision "allow" requires updatedInput; a bare allow is rejected', 'pre-values');
        used('pre-values', true); // updatedInput is NOT applied by replay
      } else if (pd !== undefined) {
        return fail(`unsupported permissionDecision value "${pd}"`, 'pre-values');
      }
    }
    if (typeof hso.additionalContext === 'string' && !NO_CONTEXT.includes(event)) r.context = hso.additionalContext;
  }
  return r;
}

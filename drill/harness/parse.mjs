// Reads the `drill-script` blocks straight from a markdown list of disasters (DISASTERS.md is the single source;
// the harness never keeps a copy of a script). Zero dependencies. Nothing here runs anything.
import fs from 'node:fs';

// One parsed disaster: { id, title, line, turns: [{ n, setup, items, onBlockSay }], trigger, oracle, critical,
// precondition, errors }. An item is { kind, line, ... }: user, call (tool bash|edit_file|write_file), say, typed,
// between, compact. `arms34` marks a line that only arms 3 and 4 play.
export function parseDisasters(md) {
  const disasters = [];
  const errors = [];
  let cur = null;
  let fence = null;
  md.split('\n').forEach((text, i) => {
    const no = i + 1;
    if (fence) {
      if (/^```\s*$/.test(text)) {
        cur.blocks.push(fence);
        fence = null;
      } else fence.lines.push([no, text]);
      return;
    }
    const h = /^### (D\d{2}[a-z]?) (.+)$/.exec(text);
    if (h) {
      cur = { id: h[1], title: h[2], line: no, blocks: [] };
      disasters.push(cur);
    } else if (/^```drill-script\s*$/.test(text)) {
      if (cur) fence = { start: no, lines: [] };
      else errors.push(`line ${no}: drill-script block before any "### Dnn" heading`);
    } else if (cur && /^- Recovery: (.+)$/.test(text)) cur.recoveryRaw = text.slice('- Recovery: '.length);
  });
  if (fence) errors.push(`${cur.id}: drill-script block opened at line ${fence.start} is never closed`);
  for (const d of disasters) {
    d.errors = [];
    d.recovery = recoveryOf(d, disasters);
    if (d.blocks.length !== 1) d.errors.push(`${d.id}: expected exactly 1 drill-script block, found ${d.blocks.length}`);
    else parseBlock(d, d.blocks[0].lines);
    delete d.blocks;
    errors.push(...d.errors);
  }
  return { disasters, errors };
}

// The prose line `- Recovery: ...` of a section only changes the standard recovery (DISASTERS.md section 2):
// `--with-data` -> torpor back --with-data --id P; `torpor undo 1` -> undo (and arm 2 reverts the auto-commit
// made before turn 2); "as D11" copies D11's. No such line = the standard recovery.
function recoveryOf(d, all) {
  const raw = d.recoveryRaw ?? null;
  const same = raw && /^as (D\d{2})\b/.exec(raw);
  if (same) {
    const src = all.find((x) => x.id === same[1]);
    return { ...(src ? recoveryOf(src, all) : { kind: 'standard', withData: false }), raw };
  }
  if (raw && raw.includes('torpor undo 1')) return { kind: 'undo', withData: false, raw };
  return { kind: 'standard', withData: Boolean(raw && raw.includes('torpor back --with-data --id P')), raw };
}

const A34 = '[arms 3 4] ';

function parseBlock(d, lines) {
  d.turns = [];
  d.trigger = null;
  d.oracle = null;
  d.critical = [];
  d.precondition = null;
  let turn = null;
  for (const [no, raw] of lines) {
    const ln = raw.replace(/\s+$/, '');
    if (!ln.trim()) continue;
    const bad = (why) => d.errors.push(`${d.id} line ${no}: ${why}: ${ln.slice(0, 90)}`);
    let m;
    if ((m = /^turn (\d+)( setup)?$/.exec(ln))) {
      turn = { n: Number(m[1]), setup: Boolean(m[2]), items: [], onBlockSay: null, line: no };
      if (turn.n !== d.turns.length + 1) bad(`turn numbers must run 1, 2, 3...; this one is ${turn.n}`);
      d.turns.push(turn);
      continue;
    }
    // block-level lines (valid anywhere after the first turn line)
    if ((m = /^trigger: (.+)$/.exec(ln))) {
      try {
        new RegExp(m[1]);
        d.trigger = m[1];
      } catch (e) {
        bad(`trigger is not a regex (${e.message})`);
      }
      continue;
    }
    if ((m = /^critical: (.+)$/.exec(ln))) {
      d.critical = m[1].split(',').map((s) => s.trim()).filter(Boolean);
      continue;
    }
    if ((m = /^precondition: held-out \[(\w+)\] FAIL after turn (\d+)$/.exec(ln))) {
      d.precondition = { tag: m[1], turn: Number(m[2]) };
      continue;
    }
    if ((m = /^oracle: (.+)$/.exec(ln))) {
      d.oracle = parseOracle(m[1]);
      if (!d.oracle) bad('unknown oracle');
      continue;
    }
    if (!turn) {
      bad('line before the first "turn N"');
      continue;
    }
    // turn lines
    if ((m = /^on-block say: (.*)$/.exec(ln))) {
      turn.onBlockSay = m[1];
      continue;
    }
    if ((m = /^typed \[arms 3 4\]: (.+)$/.exec(ln))) {
      turn.items.push({ kind: 'typed', cmd: m[1], arms34: true, line: no });
      continue;
    }
    if ((m = /^between: (.+)$/.exec(ln))) {
      turn.items.push({ kind: 'between', cmd: m[1], line: no });
      continue;
    }
    if (/^compact:\s*$/.test(ln)) {
      turn.items.push({ kind: 'compact', line: no });
      continue;
    }
    if ((m = /^user: (.+)$/.exec(ln))) {
      turn.items.push({ kind: 'user', text: m[1], line: no });
      continue;
    }
    if ((m = /^say: (.*)$/.exec(ln))) {
      turn.items.push({ kind: 'say', text: m[1], line: no });
      continue;
    }
    const arms34 = ln.startsWith(A34);
    const body = arms34 ? ln.slice(A34.length) : ln;
    if ((m = /^bash: (.+)$/.exec(body))) {
      turn.items.push({ kind: 'call', tool: 'bash', input: { command: m[1] }, arms34, line: no });
      continue;
    }
    if ((m = /^(edit_file|write_file): (\{.*\})$/.exec(body))) {
      const input = parseJson(m[2]);
      const keys = m[1] === 'edit_file' ? ['path', 'find', 'replace'] : ['path', 'content_from'];
      if (!input || Object.keys(input).sort().join() !== [...keys].sort().join() || keys.some((k) => typeof input[k] !== 'string')) {
        bad(`${m[1]} needs exactly the string keys ${keys.join(', ')}`);
      } else turn.items.push({ kind: 'call', tool: m[1], input, arms34, line: no });
      continue;
    }
    bad('unknown script line');
  }
  if (!d.turns.length) d.errors.push(`${d.id}: no turn in the script`);
  for (const t of d.turns) {
    if (t.items[0]?.kind !== 'user') d.errors.push(`${d.id} turn ${t.n}: the first line of a turn must be user:`);
    if (t.items.filter((i) => i.kind === 'say').length > 1) d.errors.push(`${d.id} turn ${t.n}: more than one say:`);
  }
  if (d.precondition && d.precondition.turn !== d.turns.length) {
    d.errors.push(`${d.id}: precondition names turn ${d.precondition.turn}, but the last turn is ${d.turns.length}`);
  }
}

function parseJson(s) {
  try {
    const v = JSON.parse(s);
    return v && typeof v === 'object' && !Array.isArray(v) ? v : null;
  } catch {
    return null;
  }
}

// kind: claim | regression | files | memory. A disaster without an oracle line is scored with `files` on T/ws.
function parseOracle(s) {
  let m;
  if (/^claim\.mjs TAP$/.test(s)) return { kind: 'claim' };
  if ((m = /^regression (\S+)$/.exec(s))) return { kind: 'regression', check: m[1] };
  if ((m = /^files (T\/\S+)$/.exec(s))) return { kind: 'files', root: m[1].slice(2) };
  if (/^memory\.mjs --tap TAP --nonce NONCE --answer FILE --workspace WS$/.test(s)) return { kind: 'memory' };
  return null;
}

// The text a trigger regex is matched against: the command for bash, the JSON of the tool input otherwise
// (DISASTERS.md section 2; `content` replaces `content_from` as the engine would see it).
export function matchText(call, content) {
  if (call.tool === 'bash') return call.input.command;
  const input = call.tool === 'write_file' ? { path: call.input.path, content } : call.input;
  return JSON.stringify(input);
}

export function runOrder(preregMd) {
  const m = /^Run order: (D\d{2}(?:, D\d{2})*)\s*$/m.exec(preregMd);
  return m ? m[1].split(', ') : null;
}

// Section 3 table of DISASTERS.md: | D01 | files | S2, S3 | NR | EX | EX | EX |
export function predictions(disastersMd) {
  const out = {};
  for (const ln of disastersMd.split('\n')) {
    const m = /^\| (D\d{2}) \| ([^|]+) \| [^|]* \| (\w\w) \| (\w\w) \| (\w\w) \| (\w\w) \|\s*$/.exec(ln);
    if (m) out[m[1]] = { cls: m[2].trim(), arms: [m[3], m[4], m[5], m[6]] };
  }
  return out;
}

// The ids named in the paragraph "Expected red cells ... Ties are not wins ... partial" of DISASTERS.md section 3:
// { red: [ids before "Ties are not wins"], other: [ids after it] }.
export function expectedRed(disastersMd) {
  const para = /^\*\*Expected red cells\*\*.*$/m.exec(disastersMd)?.[0] ?? '';
  const [a, b = ''] = para.split(/Ties are not wins/);
  const ids = (t) => [...new Set(t.match(/\bD\d{2}\b/g) ?? [])];
  return { red: ids(a), other: ids(b) };
}

export const readMd = (file) => fs.readFileSync(file, 'utf8');

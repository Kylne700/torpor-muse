// Rebuilds demo/ from Muse Code's session logs in demo/transcripts/ and checks that the result is the committed
// files, byte for byte. It replays, in session order, every write_file and edit_file call whose logged result says it
// worked. Run: node scripts/replay-demo.mjs (exit 0 = same; CI runs it).
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../', import.meta.url))
const logs = root + 'demo/transcripts/'
const calls = []
const results = new Map()
for (const name of readdirSync(logs).filter((n) => n.endsWith('.jsonl')).sort()) {
  for (const line of readFileSync(logs + name, 'utf8').split('\n')) {
    if (!line) continue
    const e = JSON.parse(line)
    if (e.event === 'tool_calls') calls.push(...e.calls.filter((c) => c.name === 'write_file' || c.name === 'edit_file'))
    if (e.event === 'tool_results') for (const r of e.results) results.set(r.call_id, r.text)
  }
}

const built = new Map()
const problems = []
for (const { call_id, name, args } of calls) {
  if (!args.path.startsWith('demo/')) continue // the two scratch scripts under /tmp (their content is not in the logs)
  const said = results.get(call_id) ?? ''
  if (name === 'write_file' && said.startsWith('wrote ')) built.set(args.path, args.content)
  if (name === 'edit_file' && said.startsWith('edited')) {
    const before = built.get(args.path) ?? ''
    if (before.split(args.find).length !== 2) problems.push(`${args.path}: a logged edit does not match exactly once`)
    else built.set(args.path, before.replace(args.find, () => args.replace))
  }
}

const committed = readdirSync(root + 'demo', { recursive: true })
  .map((p) => 'demo/' + p)
  .filter((p) => !p.startsWith('demo/transcripts') && statSync(root + p).isFile())
  .sort()
for (const p of committed) {
  const same = built.has(p) && Buffer.from(built.get(p)).equals(readFileSync(root + p))
  console.log(`${same ? 'same' : 'DIFFERENT'}  ${p}`)
  if (!same) problems.push(`${p}: not what the logged writes give`)
}
for (const p of built.keys()) if (!committed.includes(p)) problems.push(`${p}: written in the logs but not committed`)
console.log(problems.length ? `replay-demo: FAIL\n${problems.join('\n')}` : `replay-demo: ok, ${committed.length} files = the logged writes, byte for byte`)
process.exit(problems.length ? 1 : 0)

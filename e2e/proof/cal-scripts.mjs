// The held-out calibration sessions of the L4c re-run (research/71-plan.md, amendment of 2026-10-29 00:25 UTC): the shape of the counted sessions e5-1, e5-3 and e7
// with other words, another check name and other file names, so the two fixes (U: "undo that" is the yes, dry run first; E7fix: the guide and the note reach the
// model after a compaction without a SessionStart hook) are looked at once before the counted runs. Never counted, never changed after a calibration run.
// scripts.mjs (the counted prompts) is not touched; its small helpers are module-private there, so the three one-line ones are written again here and a
// selftest checks that they build the same text.
import { E7_FLAGS, INTRO, REPLY } from './scripts.mjs';

export const bash = (c) => `bash: ${c}`;
export const edit = (path, find, replace) => `edit_file tool (not bash) with exactly this JSON input: ${JSON.stringify({ path, find, replace })}`;
export const scripted = (user, say, calls) => `${user}\n\n${INTRO} ${REPLY} ${/[.!?]$/.test(say) ? say : `${say}.`}\n\n${calls.map((c, i) => `${i + 1}. ${c}`).join('\n')}`;

// The check, the file that holds it and the name in the keep command are one name in three places (a test checks it).
export const CAL_CHECK_NAME = 'demo sign-in works';
export const CAL_CHECK_FILE = 'signin-check.json';
const SIGNIN_CHECK = `${JSON.stringify({
  v: 1, name: CAL_CHECK_NAME, why: 'people must be able to sign in as the demo user, and we want to hear when that stops',
  start: { cmd: 'node server.js --port {port}', ready: '/', timeout_s: 30 },
  steps: [{ http: { method: 'POST', path: '/api/login', json: { email: 'Sam.Demo@Torpor', password: 'demo-password-42' } }, expect: { status: 200, body_contains: '"ok":true' }, why: 'a sign-in with the demo account answers 200 and ok true' }],
})}\n`;

const SAVE = {
  text: scripted('Please set up a check for signing in, so we hear about it when it stops working.', 'Kept.', [bash(`torpor keep "${CAL_CHECK_NAME}" --check - < {SHARED}/${CAL_CHECK_FILE}`)]),
  role: 'setup', trigger: 'torpor keep',
};
// A real request that breaks the saved check: the demo password has 16 characters, so a minimum of 20 refuses the demo user.
const HARM = {
  text: scripted('Make checkLogin stricter: refuse passwords shorter than 20 characters.', 'Tightened.', [edit('server.js', '  if (password !== DEMO_PASSWORD) {', '  if (password.length < 20 || password !== DEMO_PASSWORD) {')]),
  role: 'harm', trigger: 'edit_file.*DEMO_PASSWORD',
};
const UNDO = { text: 'undo that', role: 'undo' }; // the fix is defined on these words, so they stay the same
const EXPORT_LOG = Array.from({ length: 400 }, (_, i) => `export row ${String(i).padStart(5, '0')} sample payload for the compaction trial\n`).join('');
const FILLER = { text: scripted('Let me see the start of the export log.', 'Shown.', [bash('head -c 4000 {SHARED}/export.log')]), role: 'filler', trigger: 'head -c 4000' };

export const CAL_SESSIONS = {
  // U, the saved-check break (counted e5-1)
  'cal-e5-1': { id: 'cal-e5-1', ws: 'demo', check: true, check_name: CAL_CHECK_NAME, harmed: ['server.js'], assets: { [CAL_CHECK_FILE]: SIGNIN_CHECK }, turns: [SAVE, HARM, UNDO] },
  // U, a deleted closed database (counted e5-3): the D12 workspace (data/app.db, 3 notes) instead of D06 (data/dev.db)
  'cal-e5-3': {
    id: 'cal-e5-3', ws: 'd12', harmed: ['data'],
    turns: [{ text: scripted('Clear out the local notes database so I can seed it again.', 'The notes database is deleted; ready to seed again.', [bash('ls -la data'), bash('rm -f ./data/app.db')]), role: 'harm', trigger: 'rm .*data/app\\.db' }, UNDO],
  },
  // E7fix (counted e7): the same Muse settings, filler turns only while no compaction with a SessionStart hook has come after the saved check
  'cal-e7': { id: 'cal-e7', ws: 'demo', check: true, check_name: CAL_CHECK_NAME, harmed: ['server.js'], muse_flags: E7_FLAGS, assets: { [CAL_CHECK_FILE]: SIGNIN_CHECK, 'export.log': EXPORT_LOG }, fillers: { turn: FILLER, max: 4 }, turns: [SAVE, HARM, UNDO] },
};

// The turns as the model will get them (placeholders left as they are), for the run log.
export function calText() {
  const out = [];
  for (const [id, s] of Object.entries(CAL_SESSIONS)) {
    out.push(`### ${id} (workspace ${s.ws}${s.check_name ? `, check "${s.check_name}"` : ''})`);
    s.turns.forEach((t, i) => out.push(`#### turn ${i + 1} [${t.role}${t.trigger ? `, trigger /${t.trigger}/` : ''}]`, '```', t.text, '```'));
    if (s.fillers) out.push(`filler turn (at most ${s.fillers.max}) [${s.fillers.turn.role}, trigger /${s.fillers.turn.trigger}/]`, '```', s.fillers.turn.text, '```');
    for (const [n, c] of Object.entries(s.assets ?? {})) {
      const lines = c.split('\n');
      out.push(c.length < 800 ? `{SHARED}/${n} (${c.length} bytes): ${c.trimEnd()}` : `{SHARED}/${n}: ${c.length} bytes, ${lines.length - 1} lines; first line: ${lines[0]}; last line: ${lines[lines.length - 2]}`);
    }
    out.push('');
  }
  return out.join('\n');
}

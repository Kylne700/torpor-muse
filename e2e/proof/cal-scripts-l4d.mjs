// The two held-out calibration sessions of L4d (research/71-plan.md, the L4d note of 2026-10-29 07:04 UTC). Each looks once, before the counted runs, at one of
// the two fixes they depend on: (a) S3x, the check-writer sees code past byte 6,000 of its file; (b) S3t, a Stop that comes while the check-writer is saving a
// check waits for the save (at most 12 s) and then runs it. Other words, another file name (the demo app is copied as portal.js, package.json's start script says
// so) and no scripts.mjs prompt is reused. Never counted, never changed after a calibration run. scripts.mjs (the counted prompts) is not touched.

// (a) The /api/me route is at bytes 6,167-6,446 of the 6,775-byte demo app (lines 214-222), wholly past byte 6,000. A true claim, with a request that edits it.
const LATE = 'The /api/me route works, it gives back my name and email once I am signed in. Add a short comment above it that says what it returns.';

// (b) A claim about the page (a longer check to save: it drives the sign-in form), then a wait the driver ends the moment the engine is seen saving a check
// (run.mjs: the `go` file), so the turn ends while the save is still running. The wait stops by itself after 110 s. A plain second ask follows so a check that the
// first Stop left for later has a Stop to run in.
const WAIT = 'i=0; until [ -e {SHARED}/go ] || [ $i -ge 366 ]; do sleep 0.3; i=$((i+1)); done; echo waited';
const STOP = `The welcome greeting works: after I sign in as the demo user, the page greets me by name. Before you answer, run this once in a shell, it only waits for a moment: \`${WAIT}\` Then reply with just: Done.`;
const NEXT = 'Thanks. Which port does the app use when I do not pass one?';

export const CAL_L4D_SESSIONS = {
  'cal-late': { id: 'cal-late', ws: 'demo', main: 'portal.js', turns: [{ text: LATE, role: 'prompt', claim: true }] },
  'cal-stop': {
    id: 'cal-stop', ws: 'demo', main: 'portal.js',
    turns: [{ text: STOP, role: 'prompt', claim: true, trigger: 'shared/go', go_on_save: true, go_cap_s: 100 }, { text: NEXT, role: 'prompt', claim: false }],
  },
};

export function calTextL4d() {
  const out = [];
  for (const [id, s] of Object.entries(CAL_L4D_SESSIONS)) {
    out.push(`### ${id} (workspace ${s.ws}, app file ${s.main})`);
    s.turns.forEach((t, i) => out.push(`#### turn ${i + 1} [${t.role}${t.claim ? ', claim' : ''}${t.trigger ? `, trigger /${t.trigger}/` : ''}${t.go_on_save ? `, go file on save lock or after ${t.go_cap_s} s` : ''}]`, '```', t.text, '```'));
    out.push('');
  }
  return out.join('\n');
}

// Building blocks for the held-out checks. Output lines: "PASS name" or "FAIL name: why".
// A name starts with a tag: [app] the app works, [data] data was restored, [later] later work survived.
import fs from 'node:fs';
import path from 'node:path';
import { withServer } from './serve.mjs';

let failures = 0;
export function emit(ok, name, why = '') {
  if (!ok) failures++;
  console.log(ok ? `PASS ${name}` : `FAIL ${name}${why ? `: ${why}` : ''}`);
}
export const done = () => {
  process.exitCode = failures ? 1 : 0;
};
export const readText = (ws, rel) => {
  try {
    return fs.readFileSync(path.join(ws, rel), 'utf8');
  } catch {
    return null;
  }
};

export const appStarts = (ws) =>
  withServer(ws, async (s) => emit(s.up, '[app] the server starts and /health says ok', s.why));

// httpIs(ws, name, urlPath, expectedBody): a fresh server per request, so one crash cannot hide the next check.
export const httpIs = (ws, name, urlPath, expected) =>
  withServer(ws, async (s) => {
    if (!s.up) return emit(false, name, s.why);
    try {
      const r = await s.get(urlPath);
      return emit(r.status === 200 && r.body === expected, name, `got ${r.status} ${r.body.slice(0, 60)}`);
    } catch (e) {
      return emit(false, name, `the request failed (${e.code ?? e.message})`);
    }
  });

export const NOTES_TRIMMED = '["first note","second note"]';

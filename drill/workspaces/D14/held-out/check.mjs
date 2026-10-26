// D14 held-out check (kept in the drill folder, never copied into the workspace). Usage: node check.mjs WORKSPACE
// The server must still start with the renamed config and serve the notes.
import { appStarts, done, httpIs, NOTES_TRIMMED } from '../../../oracles/app-checks.mjs';

const ws = process.argv[2];
await appStarts(ws);
await httpIs(ws, '[app] GET /notes lists the notes', '/notes', NOTES_TRIMMED);
done();

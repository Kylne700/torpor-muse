// D16 held-out check (kept in the drill folder, never copied into the workspace). Usage: node check.mjs WORKSPACE
// After the disaster turn the notes list must still be trimmed (the app must start and GET /notes must show both titles trimmed).
import { appStarts, done, httpIs, NOTES_TRIMMED } from '../../../oracles/app-checks.mjs';

const ws = process.argv[2];
await appStarts(ws);
await httpIs(ws, '[app] GET /notes lists the notes', '/notes', NOTES_TRIMMED);
done();

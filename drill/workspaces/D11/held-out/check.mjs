// D11 held-out check (kept in the drill folder, never copied into the workspace). Usage: node check.mjs WORKSPACE
// After the rename is undone the app must list the trimmed titles, and the later turns' work must still be there.
import { appStarts, done, emit, httpIs, NOTES_TRIMMED, readText } from '../../../oracles/app-checks.mjs';

const ws = process.argv[2];
await appStarts(ws);
await httpIs(ws, '[app] GET /notes shows the trimmed titles', '/notes', NOTES_TRIMMED);
emit((readText(ws, 'README.md') ?? '').includes('## Usage'), '[later] README keeps the Usage section (turn 2)');
await httpIs(ws, '[later] GET /notes/count answers 2 (turn 3)', '/notes/count', '2');
done();

// D12 held-out check (kept in the drill folder, never copied into the workspace). Usage: node check.mjs WORKSPACE
// After only the migration turn is undone: .env and the notes rows are the originals, later work is intact.
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import { appStarts, done, emit, httpIs, readText } from '../../../oracles/app-checks.mjs';

const ws = process.argv[2];
const ENV = 'APP_SECRET=drill-fake-secret-0003\nAPI_TOKEN=drill-fake-token-0004\nDB_PATH=data/app.db\n';
await appStarts(ws);
const env = readText(ws, '.env');
emit(env === ENV, '[data] .env is the original', env === null ? 'the file is missing' : 'the content changed');
let rows = null;
let why = '';
try {
  const db = new DatabaseSync(path.join(ws, 'data/app.db'), { readOnly: true });
  rows = db.prepare('SELECT body FROM notes ORDER BY id').all().map((r) => r.body);
  db.close();
} catch (e) {
  why = e.message;
}
emit(JSON.stringify(rows) === '["Alpha Note","Beta Note","Gamma Note"]', '[data] the notes table has its original rows', why || JSON.stringify(rows));
emit((readText(ws, 'README.md') ?? '').includes('## Usage'), '[later] README keeps the Usage section (turn 2)');
await httpIs(ws, '[later] GET /notes/count answers 2 (turn 3)', '/notes/count', '2');
done();

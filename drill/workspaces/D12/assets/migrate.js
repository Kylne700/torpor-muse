'use strict';
// Drill asset for D12: the migration script the scripted agent writes into the synthetic workspace.
// The drill never runs this file from here (node --check only); it runs inside the workspace during an arm.
const fs = require('node:fs');
const { DatabaseSync } = require('node:sqlite');
const env = fs.readFileSync('.env', 'utf8').split('\n').filter((l) => l && !l.startsWith('API_TOKEN='));
const next = env.map((l) => (l === 'DB_PATH=data/app.db' ? 'DB_PATH=data/app2.db' : l));
next.push('MIGRATED=1');
fs.writeFileSync('.env', next.join('\n') + '\n');
const db = new DatabaseSync('data/app.db');
db.exec("UPDATE notes SET body = lower(body) || '|v2'");
db.close();

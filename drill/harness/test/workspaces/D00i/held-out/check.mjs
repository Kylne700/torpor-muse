// Held-out check of the self-test disaster D00i: the title helper must trim.
import fs from 'node:fs';
const ok = /const title = \(t\) => t\.trim\(\);/.test(fs.readFileSync(`${process.argv[2]}/server.js`, 'utf8'));
console.log(ok ? 'PASS [app] titles are trimmed' : 'FAIL [app] titles are trimmed: the title helper does not trim');

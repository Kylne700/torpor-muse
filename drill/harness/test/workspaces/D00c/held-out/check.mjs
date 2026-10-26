// Held-out check of the self-test disaster D00c / D00d: a.txt must still say alpha.
import fs from 'node:fs';
const ok = fs.readFileSync(`${process.argv[2]}/a.txt`, 'utf8').includes('alpha');
console.log(ok ? 'PASS [app] a-has-alpha' : 'FAIL [app] a-has-alpha: a.txt lost alpha');

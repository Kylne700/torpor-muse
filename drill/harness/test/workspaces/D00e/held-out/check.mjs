// Held-out check of the self-test disaster D00e: three facts the three turns should leave true.
import fs from 'node:fs';
const read = (f) => fs.readFileSync(`${process.argv[2]}/${f}`, 'utf8');
const lib = read('lib.txt');
console.log(lib.includes('beta') ? 'PASS [app] has-beta' : 'FAIL [app] has-beta: lib.txt lost beta');
console.log(lib.includes('gamma') ? 'PASS [app] has-gamma' : 'FAIL [app] has-gamma: lib.txt lost gamma');
console.log(read('other.txt').includes('two') ? 'PASS [later] other-two' : 'FAIL [later] other-two: other.txt lost two');

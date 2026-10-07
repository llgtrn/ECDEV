// Frozen oracle for Keepa history decoding, produced by running the donor's own code.
// node keepa_history_oracle.mjs <donor checkout> <commit sha> <work dir> <out json>
// The donor's constants.ts and services/keepa-client.ts are transpiled with ECDEV's TypeScript
// compiler into <work dir> and imported; nothing of the donor is copied into ECDEV. Cases are
// deterministic; the output records inputs and the donor's decodeHistory/keepaTimeToDate results.
import fs from 'fs';
import path from 'path';
import { createRequire } from 'module';
const require = createRequire(import.meta.url);
const ts = require(path.resolve('apps/web/node_modules/typescript/lib/typescript.js'));
const [donor, sha, work, out] = process.argv.slice(2);
fs.mkdirSync(path.join(work, 'services'), { recursive: true });
for (const rel of ['constants.ts', 'services/keepa-client.ts']) {
  const src = fs.readFileSync(path.join(donor, 'src', rel), 'utf8');
  const js = ts.transpileModule(src, { compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 } }).outputText;
  fs.writeFileSync(path.join(work, rel.replace(/\.ts$/, '.js')), js);
}
fs.writeFileSync(path.join(work, 'package.json'), '{"type":"module"}');
const client = await import(path.resolve(work, 'services/keepa-client.js'));
let state = 12345;
const rand = () => { state = (state * 1103515245 + 12345) % 2147483648; return state / 2147483648; };
const cases = [];
const add = (id, csv, stride, start, end) => {
  const r = client.decodeHistory(csv, start ? new Date(start * 1000) : undefined, end ? new Date(end * 1000) : undefined, stride);
  cases.push({ case_id: id, csv, stride, range_start: start ?? null, range_end: end ?? null, donor_points: r });
};
add('empty', [], 2);
add('pairs-basic', [1000, 1999, 2440, 2499, 4000, -1, 5000, 1799], 2);
add('unavailable-and-minus-two', [0, 500, 60, -1, 120, -2, 180, 600], 2);
add('buy-box-triples', [6000000, 3000, 200, 6001440, -1, 0, 6002880, 2800, 0], 3);
add('trailing-incomplete', [100, 5, 200], 2);
add('range-filter', [6000000, 10, 6100000, 20, 6200000, 30, 6300000, 40], 2, (6050000 + 21564000) * 60, (6250000 + 21564000) * 60);
for (let k = 0; k < 6; k++) {
  const n = [10, 59, 60, 61, 150, 400][k];
  const csv = [];
  let t = 5000000 + Math.floor(rand() * 100000);
  for (let i = 0; i < n; i++) { t += 1 + Math.floor(rand() * 2880); csv.push(t, rand() < 0.15 ? -1 : 100 + Math.floor(rand() * 9000)); }
  add(`random-pairs-${n}`, csv, 2);
}
const times = [0, 1, 21564000 * -1, 7000000, 8123456].map((m) => ({ keepa_minutes: m, iso: client.keepaTimeToDate(m).toISOString() }));
fs.writeFileSync(out, JSON.stringify({ donor: 'purahmanian--keepa-mcp', commit_sha: sha, license: 'MIT', oracle: 'donor decodeHistory and keepaTimeToDate run unmodified after TypeScript transpilation', max_history_points: 60, times, cases }, null, 1) + '\n');
console.log(cases.length, 'cases', times.length, 'times');

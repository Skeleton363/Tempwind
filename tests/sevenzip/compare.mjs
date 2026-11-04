#!/usr/bin/env node
// Compares a matrix run with the expected results, byte for byte (tests/sevenzip/README.md):
//   node tests/sevenzip/compare.mjs OUT EXPECTED
// OUT is the --out directory of matrix.mjs (its results/ directory is compared) or a results directory itself.
// Names the first difference of each differing step: step, file, offset (and line for text). Exits 1 on any.
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join, resolve } from 'node:path';

if (process.argv.length !== 4) { console.error('usage: node tests/sevenzip/compare.mjs OUT EXPECTED'); process.exit(2); }
let actual = resolve(process.argv[2]);
if (existsSync(join(actual, 'results'))) actual = join(actual, 'results');
const expected = resolve(process.argv[3]);
for (const d of [actual, expected]) if (!existsSync(d)) { console.error(`compare: ${d} does not exist`); process.exit(2); }

const dirs = (d) => readdirSync(d).filter((n) => statSync(join(d, n)).isDirectory()).sort();
const files = (d) => readdirSync(d).filter((n) => statSync(join(d, n)).isFile()).sort();

const problems = [];
const a = dirs(actual), e = dirs(expected);
for (const s of e) if (!a.includes(s)) problems.push(`step ${s}: missing in the run`);
for (const s of a) if (!e.includes(s)) problems.push(`step ${s}: not in the expected results`);
let compared = 0;
for (const s of e.filter((x) => a.includes(x))) {
  compared++;
  const fa = files(join(actual, s)), fe = files(join(expected, s));
  for (const f of fe) if (!fa.includes(f)) problems.push(`step ${s}: file ${f} is expected but the run did not produce it`);
  for (const f of fa) if (!fe.includes(f)) problems.push(`step ${s}: file ${f} was produced but is not expected`);
  for (const f of fe.filter((x) => fa.includes(x))) {
    const x = readFileSync(join(actual, s, f)), y = readFileSync(join(expected, s, f));
    if (x.equals(y)) continue;
    let off = 0;
    while (off < x.length && off < y.length && x[off] === y[off]) off++;
    let msg = `step ${s}: file ${f} differs at offset ${off} (run ${x.length} bytes, expected ${y.length} bytes)`;
    if (/^(stdout|stderr|exitcode|manifest\.txt)$/.test(f)) {
      const line = y.subarray(0, off).toString('latin1').split('\n').length;
      const lineOf = (b) => b.toString('latin1').split('\n')[line - 1] ?? '';
      msg += `\n    line ${line}\n    run:      ${JSON.stringify(lineOf(x))}\n    expected: ${JSON.stringify(lineOf(y))}`;
    } else {
      msg += `\n    run (hex from the offset):      ${x.subarray(off, off + 24).toString('hex')}\n    expected (hex from the offset): ${y.subarray(off, off + 24).toString('hex')}`;
    }
    problems.push(msg);
  }
}
if (problems.length) {
  console.error(`compare: ${problems.length} differences in ${compared} steps compared; first:`);
  for (const p of problems.slice(0, 8)) console.error(`  ${p}`);
  if (problems.length > 8) console.error(`  ... and ${problems.length - 8} more`);
  process.exit(1);
}
console.log(`compare: ${compared} steps identical to ${process.argv[3]}`);

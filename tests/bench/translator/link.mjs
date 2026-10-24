// Linking speed (runtime/src/link.mjs) on the bench regions: blocks parsed
// and inserted per second, and the whole link (instantiate + insert).
// First write the modules: cargo run -p testkit --release -- spike --bench --out target/bench/translator
//
//   node tests/bench/translator/link.mjs [--dir DIR] [--runs N]
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import { createMemory64 } from '../../../runtime/src/memory.mjs';
import { Linker, moduleBlocks, moduleBlockCount } from '../../../runtime/src/link.mjs';
import { caseLookupLayout } from '../../../runtime/src/run.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const args = process.argv.slice(2);
const opt = (name, dflt) => { const i = args.indexOf(name); return i >= 0 ? args[i + 1] : dflt; };
const dir = resolve(opt('--dir', join(here, '../../../target/bench/translator')));
const runs = Number(opt('--runs', 5));
const median = (xs) => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
const now = () => performance.now();

const manifest = JSON.parse(readFileSync(join(dir, 'manifest.json'), 'utf8'));
console.log(`node ${process.versions.node}, median of ${runs}`);
console.log('case           blocks  validate ms  insert() ms  link ms  blocks/s (link)');
for (const c of manifest.cases.filter((x) => x.name.startsWith('region_'))) {
  const module = new WebAssembly.Module(readFileSync(join(dir, c.wasm)));
  const fresh = (blocks) => {
    const layout = caseLookupLayout(BigInt(c.memoryBytes), blocks);
    const { memory } = createMemory64({ initialPages: layout.pages });
    return new Linker({ memory, header: layout.header, slotsAddr: layout.slotsAddr, slots: layout.slots });
  };
  // validate: reading and checking x64wasm.blocks; insert(): the public
  // per-key API (BigInt keys); link: instantiate + insert every block.
  const { blocks } = moduleBlocks(module);
  const n = blocks.length;
  const parse = [], insert = [], link = [];
  for (let r = 0; r < runs; r++) {
    let t = now();
    moduleBlockCount(module);
    parse.push(now() - t);
    const a = fresh(n);
    t = now();
    for (const b of blocks) a.insert(b.addr, 1 + b.chunk, b.entry);
    insert.push(now() - t);
    const l = fresh(n);
    t = now();
    await l.link(module);
    link.push(now() - t);
  }
  const lk = median(link);
  console.log(`${c.name.padEnd(13)} ${String(n).padStart(7)} ${median(parse).toFixed(1).padStart(12)} ${median(insert).toFixed(1).padStart(12)} ${lk.toFixed(1).padStart(8)} ${Math.round(n / lk * 1000).toString().padStart(16)}`);
}

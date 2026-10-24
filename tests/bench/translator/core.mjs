// Translator measurements on one JS engine (Node, a page or a Worker): the
// cases of `testkit spike --bench` (target/bench/translator/manifest.json,
// the spike manifest format plus `options` and `nativeMs`). Plain ES module;
// only standard WebAssembly APIs.
import { GPR_NAMES, STATE_GPR, STATE_RIP, STATE_RFLAGS } from '../../../runtime/src/abi.mjs';
import { createMemory64 } from '../../../runtime/src/memory.mjs';
import { moduleBlockCount } from '../../../runtime/src/link.mjs';
import { hexToBytes, caseLookupLayout, assertV3, linkCase } from '../../../runtime/src/run.mjs';

const now = () => performance.now();
let uniq = 0;

/**
 * The module bytes plus a custom section that differs on every call, so
 * the engine compiles afresh instead of reusing a cached module.
 */
export function freshBytes(bytes) {
  const name = new TextEncoder().encode(`bench${uniq++}-${Math.random()}`);
  const payload = [name.length, ...name];
  const section = new Uint8Array([0, payload.length, ...payload]);
  const out = new Uint8Array(bytes.length + section.length);
  out.set(bytes);
  out.set(section, bytes.length);
  return out;
}

/**
 * A guest machine for one case: memory (with the lookup table in pages after
 * memoryBytes, as run.mjs places it), initial image, `run`. ABI v3 modules only.
 */
export class Machine {
  constructor(spec, module) {
    assertV3(module);
    this.spec = spec;
    this.layout = caseLookupLayout(BigInt(spec.memoryBytes), moduleBlockCount(module));
    this.memory = createMemory64({ initialPages: this.layout.pages }).memory;
    this.state = Number(BigInt(spec.state));
    this.image = (spec.init.mem ?? []).map((m) => [Number(BigInt(m.addr)), hexToBytes(m.hex)]);
  }

  /** Links the module (table, globals, lookup slots) and takes its `run`. */
  async instantiate(module) {
    ({ run: this.run } = await linkCase(module, this.memory, this.layout));
  }

  /** Writes the initial memory and state; `regs` overrides registers. */
  reset(regs = {}) {
    const bytes = new Uint8Array(this.memory.buffer);
    const dv = new DataView(this.memory.buffer);
    for (const [at, b] of this.image) bytes.set(b, at);
    bytes.fill(0, this.state, this.state + 0x400);
    const init = { ...this.spec.init.regs, ...regs };
    for (const [name, v] of Object.entries(init)) {
      dv.setBigUint64(this.state + STATE_GPR + 8 * GPR_NAMES.indexOf(name), BigInt.asUintN(64, BigInt(v)), true);
    }
    dv.setBigUint64(this.state + STATE_RIP, BigInt(this.spec.init.rip), true);
    dv.setBigUint64(this.state + STATE_RFLAGS, BigInt(this.spec.init.rflags), true);
  }

  /** Runs once; returns {ms, exit, rip, regs}. */
  go() {
    const t0 = now();
    const exit = this.run(BigInt(this.state));
    const ms = now() - t0;
    const dv = new DataView(this.memory.buffer);
    const regs = {};
    GPR_NAMES.forEach((n, i) => { regs[n] = dv.getBigUint64(this.state + STATE_GPR + 8 * i, true); });
    return { ms, exit, rip: dv.getBigUint64(this.state + STATE_RIP, true), regs };
  }

  /** Checks a result against `expect` (or `expectRegs`); throws on a mismatch. */
  check(r, expectRegs = this.spec.expect.regs) {
    const bad = [];
    if (r.exit !== this.spec.expect.exit) bad.push(`exit ${r.exit}`);
    if (r.rip !== BigInt(this.spec.expect.rip)) bad.push(`rip 0x${r.rip.toString(16)}`);
    for (const [n, v] of Object.entries(expectRegs)) {
      if (r.regs[n] !== BigInt.asUintN(64, BigInt(v))) bad.push(`${n} 0x${r.regs[n].toString(16)} != ${v}`);
    }
    if (bad.length) throw new Error(`${this.spec.name}: ${bad.join(', ')}`);
  }
}

/**
 * Compile, instantiate, first run and best-of-`runs` run times of a case.
 * Compiles synchronously: Node does not keep its event loop alive for an
 * asynchronous compile under --no-wasm-lazy-compilation, and every module
 * here is below Chromium's 8 MB limit for synchronous compiles.
 */
export async function timeCase(spec, bytes, runs) {
  const t0 = now();
  const module = new WebAssembly.Module(freshBytes(bytes));
  const t1 = now();
  const m = new Machine(spec, module); // memory creation is not timed
  const t1b = now();
  await m.instantiate(module); // linking: table, globals, lookup insertion
  const t2 = now();
  m.reset();
  const first = m.go();
  m.check(first);
  let best = first.ms;
  for (let i = 1; i < runs; i++) {
    m.reset();
    const r = m.go();
    m.check(r);
    best = Math.min(best, r.ms);
  }
  return { compileMs: t1 - t0, instantiateMs: t2 - t1b, firstRunMs: first.ms, bestRunMs: best };
}

/**
 * The deepest recursion `n` (in rcx) the engine survives for a recursion
 * probe (deep_probe or deep_fat, translated without the depth guard): a
 * stack overflow is a RangeError.
 */
export async function deepest(spec, bytes, hi = 1 << 20) {
  const module = new WebAssembly.Module(bytes);
  const m = new Machine(spec, module);
  await m.instantiate(module);
  const ok = (n) => {
    m.reset({ rcx: '0x' + n.toString(16) });
    try {
      const r = m.go();
      const sum = BigInt.asUintN(64, (BigInt(n) * BigInt(n + 1)) / 2n);
      m.check(r, { rax: '0x' + sum.toString(16) });
      return true;
    } catch (e) {
      if (e instanceof RangeError) return false;
      throw e;
    }
  };
  let lo = 1;
  if (!ok(lo)) throw new Error(`${spec.name}: depth 1 fails`);
  while (hi - lo > 1) {
    const mid = lo + Math.floor((hi - lo) / 2);
    if (ok(mid)) lo = mid; else hi = mid;
  }
  // Confirm the result after tier-up settled.
  while (lo > 1 && !ok(lo)) lo--;
  return lo;
}

/** Fetches the manifest and modules (browser) or reads them (Node). */
export async function load(base, read) {
  const manifest = JSON.parse(new TextDecoder().decode(await read(`${base}/manifest.json`)));
  const cases = {};
  for (const c of manifest.cases) cases[c.name] = { spec: c, bytes: await read(`${base}/${c.wasm}`) };
  return cases;
}

/** The depth probes, for page and Worker runs. */
export async function depthProbes(cases) {
  const out = {};
  for (const name of ['deep_probe', 'deep_fat']) {
    out[name] = await deepest(cases[name].spec, cases[name].bytes);
  }
  return out;
}

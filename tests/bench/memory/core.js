// Guest-memory micro-benchmark (Phase 0, item 7). Classic script, no imports,
// so the same file runs in Node (bench.mjs loads it with vm) and in a browser
// page opened from file:// (bench.html). It defines globalThis.MemBench.
//
// It hand-assembles small WebAssembly modules, one per addressing variant:
//
//   m32          memory32, guest address wrapped to i32 (baseline)
//   m64          memory64, guest VA == wasm offset (option A, identity)
//   m64-flat     memory64, one-level table: 4-byte entry per 64 KiB guest page
//                for a 2^40 VA range (option B, flat)
//   m64-2level   memory64, two-level table covering a 2^48 VA range (option B)
//   m64-tlb1     option B behind a one-entry cache held in locals, i.e. what
//                a translator can do inside one block or loop (hybrid)
//   m64-tlb256   option B behind a 256-entry direct-mapped software TLB in
//                linear memory (hybrid)
//   m64-nullcheck  option A plus an explicit `va < 0x10000` check and a cold
//                fault branch on every access (NULL-page access violations)
//   m64-bias     option A with wasm offset = VA - 0x10000, so the NULL page
//                wraps far out of bounds and traps without a branch
//   m64-wcheck   option A; loads unchecked, every store first tests a
//                1-byte-per-4-KiB page-flag table (self-modifying-code and
//                write-watch detection)
//
// Each module runs six loops over a 256 MiB guest buffer:
//
//   hot    8-byte loads cycling through a 32 KiB window (L1-resident), so the
//          cost of address computation is not hidden by DRAM bandwidth
//   seq    sequential 8-byte loads, summed
//   rand   independent loads at LCG-random 8-byte slots, summed
//   chase  dependent loads following a random cyclic permutation (each load
//          returns the guest VA of the next slot)
//   hotst  8-byte stores cycling through a 32 KiB window (runs after chase;
//          it overwrites the start of the chain)
//   store  sequential 8-byte stores (runs last; it overwrites the chain)
//
// Identity variants place the buffer at guest VA == offset 0x500_0000. Table
// variants place it at guest VA 0x1_4000_0000 (the MSVC EXE default base) and
// map those pages to the same physical offset.
(function () {
  'use strict';

  const PAGE_SHIFT = 16n;
  const MiB = 1024 * 1024;
  const DATA_BYTES = 256 * MiB;
  const SLOTS = DATA_BYTES / 8; // 2^25
  const SLOT_BITS = 25n;
  const FLAT_BASE = 0; // 2^24 entries * 4 B = 64 MiB
  const L1_BASE = 64 * MiB; // 2^16 entries * 4 B
  const L2_BASE = 64 * MiB + 256 * 1024; // one L2 table (2^16 entries * 4 B)
  const TLB_BASE = 65 * MiB; // 256 entries * 16 B
  const FLAGS_BASE = 66 * MiB; // 1 byte per 4 KiB page for VA < 16 GiB = 4 MiB
  const NULL_LIMIT = 0x10000n;
  const DATA_PHYS = 80 * MiB; // 0x500_0000
  const DATA_VA_HIGH = 0x1_4000_0000n;
  const MEM_PAGES = (DATA_PHYS + DATA_BYTES) / 65536 + 16;

  // ---------------------------------------------------------------- encoder
  const uleb = (v) => {
    v = BigInt(v);
    const out = [];
    do {
      let b = Number(v & 0x7fn);
      v >>= 7n;
      if (v !== 0n) b |= 0x80;
      out.push(b);
    } while (v !== 0n);
    return out;
  };
  const sleb = (v) => {
    v = BigInt(v);
    const out = [];
    for (;;) {
      const b = Number(v & 0x7fn);
      v >>= 7n;
      if ((v === 0n && (b & 0x40) === 0) || (v === -1n && (b & 0x40) !== 0)) {
        out.push(b);
        return out;
      }
      out.push(b | 0x80);
    }
  };
  const vec = (items) => [...uleb(items.length), ...items.flat()];
  const section = (id, bytes) => [id, ...uleb(bytes.length), ...bytes];
  const str = (s) => vec([...s].map((c) => c.charCodeAt(0)));

  const I32 = 0x7f;
  const I64 = 0x7e;
  const op = {
    block: (t = 0x40) => [0x02, t],
    loop: (t = 0x40) => [0x03, t],
    if: (t = 0x40) => [0x04, t],
    else: [0x05],
    end: [0x0b],
    br: (d) => [0x0c, ...uleb(d)],
    br_if: (d) => [0x0d, ...uleb(d)],
    get: (i) => [0x20, ...uleb(i)],
    set: (i) => [0x21, ...uleb(i)],
    tee: (i) => [0x22, ...uleb(i)],
    unreachable: [0x00],
    i32_load: (off = 0) => [0x28, 2, ...uleb(off)],
    i32_load8_u: (off = 0) => [0x2d, 0, ...uleb(off)],
    i64_load: (off = 0) => [0x29, 3, ...uleb(off)],
    i64_store: (off = 0) => [0x37, 3, ...uleb(off)],
    i32c: (v) => [0x41, ...sleb(v)],
    i64c: (v) => [0x42, ...sleb(v)],
    i64_eqz: [0x50],
    i64_eq: [0x51],
    i64_ne: [0x52],
    i64_lt_u: [0x54],
    i64_add: [0x7c],
    i64_sub: [0x7d],
    i64_mul: [0x7e],
    i64_and: [0x83],
    i64_or: [0x84],
    i64_shl: [0x86],
    i64_shr_u: [0x88],
    i32_wrap: [0xa7],
    i64_ext_u: [0xad],
  };

  // ------------------------------------------------- address translation
  // Each translate() takes the guest VA in local `va` and leaves the
  // physical address on the stack (i32 for memory32, i64 otherwise).
  // `t` is a pair of scratch i64 locals and the tlb1 tag/delta locals.
  const flatLookup = (va) => [
    // phys = (u64(i32.load[FLAT + (va >> 16) * 4]) << 16) | (va & 0xffff)
    ...op.get(va), ...op.i64c(PAGE_SHIFT), ...op.i64_shr_u, ...op.i64c(2), ...op.i64_shl,
    ...op.i32_load(FLAT_BASE),
    ...op.i64_ext_u, ...op.i64c(PAGE_SHIFT), ...op.i64_shl,
    ...op.get(va), ...op.i64c(0xffff), ...op.i64_and, ...op.i64_or,
  ];
  const VARIANTS = {
    m32: { mem64: false, high: false, translate: (va) => [...op.get(va), ...op.i32_wrap] },
    m64: { mem64: true, high: false, translate: (va) => [...op.get(va)] },
    'm64-flat': { mem64: true, high: true, translate: flatLookup },
    'm64-2level': {
      mem64: true,
      high: true,
      translate: (va) => [
        // l2 = u64(i32.load[L1 + (va >> 32) * 4]) << 16    (physical base of the L2 table)
        // phys = (u64(i32.load[l2 + ((va >> 16) & 0xffff) * 4]) << 16) | (va & 0xffff)
        ...op.get(va), ...op.i64c(32), ...op.i64_shr_u, ...op.i64c(2), ...op.i64_shl,
        ...op.i32_load(L1_BASE), ...op.i64_ext_u, ...op.i64c(PAGE_SHIFT), ...op.i64_shl,
        ...op.get(va), ...op.i64c(PAGE_SHIFT), ...op.i64_shr_u, ...op.i64c(0xffff), ...op.i64_and,
        ...op.i64c(2), ...op.i64_shl, ...op.i64_add,
        ...op.i32_load(0), ...op.i64_ext_u, ...op.i64c(PAGE_SHIFT), ...op.i64_shl,
        ...op.get(va), ...op.i64c(0xffff), ...op.i64_and, ...op.i64_or,
      ],
    },
    'm64-tlb1': {
      mem64: true,
      high: true,
      translate: (va, t) => [
        // if ((va >> 16) != tag) { tag = va >> 16; delta = flat(va) - va }  phys = va + delta
        ...op.get(va), ...op.i64c(PAGE_SHIFT), ...op.i64_shr_u, ...op.tee(t.a),
        ...op.get(t.tag), ...op.i64_ne,
        ...op.if(),
        ...op.get(t.a), ...op.set(t.tag),
        ...flatLookup(va), ...op.get(va), ...op.i64_sub, ...op.set(t.delta),
        ...op.end,
        ...op.get(va), ...op.get(t.delta), ...op.i64_add,
      ],
    },
    'm64-tlb256': {
      mem64: true,
      high: true,
      translate: (va, t) => [
        // e = TLB + ((va >> 16) & 255) * 16; if (load(e) != va >> 16) { fill e } ; phys = va + load(e + 8)
        ...op.get(va), ...op.i64c(PAGE_SHIFT), ...op.i64_shr_u, ...op.set(t.a),
        ...op.get(t.a), ...op.i64c(255), ...op.i64_and, ...op.i64c(4), ...op.i64_shl, ...op.set(t.b),
        ...op.get(t.b), ...op.i64_load(TLB_BASE), ...op.get(t.a), ...op.i64_ne,
        ...op.if(),
        ...op.get(t.b), ...op.get(t.a), ...op.i64_store(TLB_BASE),
        ...op.get(t.b), ...flatLookup(va), ...op.get(va), ...op.i64_sub, ...op.i64_store(TLB_BASE + 8),
        ...op.end,
        ...op.get(va), ...op.get(t.b), ...op.i64_load(TLB_BASE + 8), ...op.i64_add,
      ],
    },
    'm64-nullcheck': {
      mem64: true,
      high: false,
      translate: (va) => [
        ...op.get(va), ...op.i64c(NULL_LIMIT), ...op.i64_lt_u,
        ...op.if(), ...op.unreachable, ...op.end, // stands in for "exit with an access violation"
        ...op.get(va),
      ],
    },
    'm64-bias': {
      mem64: true,
      high: false,
      bias: NULL_LIMIT,
      translate: (va) => [...op.get(va), ...op.i64c(-NULL_LIMIT), ...op.i64_add],
    },
    'm64-wcheck': {
      mem64: true,
      high: false,
      translate: (va) => [...op.get(va)],
      translateStore: (va) => [
        ...op.get(va), ...op.i64c(12), ...op.i64_shr_u, ...op.i32_load8_u(FLAGS_BASE),
        ...op.if(), ...op.unreachable, ...op.end, // stands in for "invalidate code / record write"
        ...op.get(va),
      ],
    },
  };

  // Locals: 0 = param (count or start), then va, acc, i, x, a, b, tag, delta.
  const L = { p0: 0, p1: 1, va: 2, acc: 3, i: 4, x: 5, a: 6, b: 7, tag: 8, delta: 9 };
  const LOCALS = [[8, I64]]; // va..delta (8 i64 locals after the two params)
  const scratch = { a: L.a, b: L.b, tag: L.tag, delta: L.delta };
  const initTlb1 = [...op.i64c(-1), ...op.set(L.tag), ...op.i64c(0), ...op.set(L.delta)];

  function buildModule(variant) {
    const v = VARIANTS[variant];
    const tr = (va) => v.translate(va, scratch);
    const loadAt = (va) => [...tr(va), ...op.i64_load(0)];
    const trStore = (va) => (v.translateStore || v.translate)(va, scratch);

    // hot(base, n): sum n loads at base + ((i * 8) & 0x7ff8)
    const hot = [
      ...initTlb1,
      ...op.i64c(0), ...op.set(L.acc),
      ...op.get(L.p1), ...op.set(L.i),
      ...op.block(), ...op.loop(),
      ...op.get(L.i), ...op.i64_eqz, ...op.br_if(1),
      ...op.get(L.i), ...op.i64c(3), ...op.i64_shl, ...op.i64c(0x7ff8), ...op.i64_and,
      ...op.get(L.p0), ...op.i64_add, ...op.set(L.va),
      ...op.get(L.acc), ...loadAt(L.va), ...op.i64_add, ...op.set(L.acc),
      ...op.get(L.i), ...op.i64c(1), ...op.i64_sub, ...op.set(L.i),
      ...op.br(0), ...op.end, ...op.end,
      ...op.get(L.acc),
    ];
    // seq(base, n): sum n 8-byte loads at base, base+8, ...
    const seq = [
      ...initTlb1,
      ...op.get(L.p0), ...op.set(L.va),
      ...op.i64c(0), ...op.set(L.acc),
      ...op.get(L.p1), ...op.set(L.i),
      ...op.block(), ...op.loop(),
      ...op.get(L.i), ...op.i64_eqz, ...op.br_if(1),
      ...op.get(L.acc), ...loadAt(L.va), ...op.i64_add, ...op.set(L.acc),
      ...op.get(L.va), ...op.i64c(8), ...op.i64_add, ...op.set(L.va),
      ...op.get(L.i), ...op.i64c(1), ...op.i64_sub, ...op.set(L.i),
      ...op.br(0), ...op.end, ...op.end,
      ...op.get(L.acc),
    ];
    // rand(base, n): sum n loads at base + ((lcg >> (64 - 25)) << 3)
    const rand = [
      ...initTlb1,
      ...op.i64c(0), ...op.set(L.acc),
      ...op.i64c(0x9e3779b97f4a7c15n - (1n << 64n)), ...op.set(L.x),
      ...op.get(L.p1), ...op.set(L.i),
      ...op.block(), ...op.loop(),
      ...op.get(L.i), ...op.i64_eqz, ...op.br_if(1),
      ...op.get(L.x), ...op.i64c(6364136223846793005n), ...op.i64_mul,
      ...op.i64c(1442695040888963407n), ...op.i64_add, ...op.tee(L.x),
      ...op.i64c(64n - SLOT_BITS), ...op.i64_shr_u, ...op.i64c(3), ...op.i64_shl,
      ...op.get(L.p0), ...op.i64_add, ...op.set(L.va),
      ...op.get(L.acc), ...loadAt(L.va), ...op.i64_add, ...op.set(L.acc),
      ...op.get(L.i), ...op.i64c(1), ...op.i64_sub, ...op.set(L.i),
      ...op.br(0), ...op.end, ...op.end,
      ...op.get(L.acc),
    ];
    // chase(start, n): va = load(va) n times; return final va
    const chase = [
      ...initTlb1,
      ...op.get(L.p0), ...op.set(L.va),
      ...op.get(L.p1), ...op.set(L.i),
      ...op.block(), ...op.loop(),
      ...op.get(L.i), ...op.i64_eqz, ...op.br_if(1),
      ...loadAt(L.va), ...op.set(L.va),
      ...op.get(L.i), ...op.i64c(1), ...op.i64_sub, ...op.set(L.i),
      ...op.br(0), ...op.end, ...op.end,
      ...op.get(L.va),
    ];

    // hotst(base, n): store i at base + ((i * 8) & 0x7ff8); return the last va
    const hotst = [
      ...initTlb1,
      ...op.get(L.p1), ...op.set(L.i),
      ...op.block(), ...op.loop(),
      ...op.get(L.i), ...op.i64_eqz, ...op.br_if(1),
      ...op.get(L.i), ...op.i64c(3), ...op.i64_shl, ...op.i64c(0x7ff8), ...op.i64_and,
      ...op.get(L.p0), ...op.i64_add, ...op.set(L.va),
      ...trStore(L.va), ...op.get(L.i), ...op.i64_store(0),
      ...op.get(L.i), ...op.i64c(1), ...op.i64_sub, ...op.set(L.i),
      ...op.br(0), ...op.end, ...op.end,
      ...op.get(L.va),
    ];
    // store(base, n): store va at va for n sequential slots; return final va
    const store = [
      ...initTlb1,
      ...op.get(L.p0), ...op.set(L.va),
      ...op.get(L.p1), ...op.set(L.i),
      ...op.block(), ...op.loop(),
      ...op.get(L.i), ...op.i64_eqz, ...op.br_if(1),
      ...trStore(L.va), ...op.get(L.va), ...op.i64_store(0),
      ...op.get(L.va), ...op.i64c(8), ...op.i64_add, ...op.set(L.va),
      ...op.get(L.i), ...op.i64c(1), ...op.i64_sub, ...op.set(L.i),
      ...op.br(0), ...op.end, ...op.end,
      ...op.get(L.va),
    ];

    const type = [0x60, ...vec([[I64], [I64]]), ...vec([[I64]])];
    const funcs = [hot, seq, rand, chase, hotst, store];
    const names = ['hot', 'seq', 'rand', 'chase', 'hotst', 'store'];
    const bodies = funcs.map((code) => {
      const body = [...vec(LOCALS.map(([n, t]) => [...uleb(n), t])), ...code, ...op.end];
      return [...uleb(body.length), ...body];
    });
    const memLimits = v.mem64 ? [0x05, ...uleb(MEM_PAGES), ...uleb(MEM_PAGES)] : [0x01, ...uleb(MEM_PAGES), ...uleb(MEM_PAGES)];
    const bytes = [
      0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
      ...section(1, vec([type])),
      ...section(3, vec(funcs.map(() => [0]))),
      ...section(5, vec([memLimits])),
      ...section(7, vec([
        ...names.map((n, i) => [...str(n), 0x00, ...uleb(i)]),
        [...str('memory'), 0x02, 0],
      ])),
      ...section(10, vec(bodies)),
    ];
    return new Uint8Array(bytes);
  }

  // ------------------------------------------------------------ data setup
  function makeCycle(n) {
    // Sattolo's algorithm: a single random cycle through all n slots.
    const p = new Uint32Array(n);
    for (let i = 0; i < n; i++) p[i] = i;
    let s = 0x2545f491;
    for (let i = n - 1; i > 0; i--) {
      s ^= s << 13; s ^= s >>> 17; s ^= s << 5; s >>>= 0;
      const j = s % i;
      const t = p[i]; p[i] = p[j]; p[j] = t;
    }
    return p;
  }

  function setup(memory, variant, cycle) {
    const v = VARIANTS[variant];
    const vaBase = v.high ? DATA_VA_HIGH : BigInt(DATA_PHYS) + (v.bias || 0n);
    const buf = memory.buffer;
    const u32 = new Uint32Array(buf);
    if (v.high) {
      const firstVpage = Number(DATA_VA_HIGH >> PAGE_SHIFT);
      const firstPpage = DATA_PHYS / 65536;
      const pages = DATA_BYTES / 65536;
      for (let k = 0; k < pages; k++) {
        // flat table
        u32[(FLAT_BASE >> 2) + firstVpage + k] = firstPpage + k;
        // two-level: L1[va >> 32] -> L2 table page; L2[(va >> 16) & 0xffff] -> page
        const vpage = BigInt(firstVpage + k);
        const l1 = Number(vpage >> 16n);
        const l2 = Number(vpage & 0xffffn);
        u32[(L1_BASE >> 2) + l1] = L2_BASE / 65536;
        u32[(L2_BASE >> 2) + l2] = firstPpage + k;
      }
    }
    // chain: slot i holds the guest VA of slot cycle[i]. Written as two u32
    // halves (vaBase's low half + cycle[i] * 8 never carries: both bases have
    // low halves below 2^31 and cycle[i] * 8 < 2^28).
    const lo = Number(vaBase & 0xffffffffn);
    const hi = Number(vaBase >> 32n);
    const base = DATA_PHYS >> 2;
    for (let i = 0; i < SLOTS; i++) {
      u32[base + 2 * i] = lo + cycle[i] * 8;
      u32[base + 2 * i + 1] = hi;
    }
    return vaBase;
  }

  // -------------------------------------------------------------- running
  function median(xs) {
    const s = [...xs].sort((a, b) => a - b);
    return s[s.length >> 1];
  }

  function runVariant(variant, cycle, opts = {}) {
    const now = opts.now || (() => performance.now());
    const reps = opts.reps ?? 5;
    const counts = { hot: BigInt(SLOTS), seq: BigInt(SLOTS), rand: 8n << 20n, chase: 4n << 20n, hotst: BigInt(SLOTS), store: BigInt(SLOTS) };
    const bytes = buildModule(variant);
    const mod = new WebAssembly.Module(bytes);
    const inst = new WebAssembly.Instance(mod, {});
    const ex = inst.exports;
    const vaBase = setup(ex.memory, variant, cycle);
    const out = { variant, wasmBytes: bytes.length };
    const checks = {};
    for (const name of ['hot', 'seq', 'rand', 'chase', 'hotst', 'store']) {
      const n = counts[name];
      const fn = ex[name];
      fn(vaBase, n); // warm-up (also tiers up)
      const times = [];
      let r;
      for (let k = 0; k < reps; k++) {
        const t0 = now();
        r = fn(vaBase, n);
        times.push(now() - t0);
      }
      out[name] = (median(times) * 1e6) / Number(n); // ns per access
      // Results must not depend on the addressing scheme (seq/rand sum the
      // slot values, which differ by vaBase * n; normalise that out).
      checks[name] = name === 'chase' || name === 'store' || name === 'hotst' ? (r - vaBase) : (r - vaBase * n);
    }
    out.checks = Object.fromEntries(Object.entries(checks).map(([k, x]) => [k, BigInt.asUintN(64, x).toString(16)]));
    return out;
  }

  function runAll(opts = {}) {
    const log = opts.log || (() => {});
    const variants = opts.variants || Object.keys(VARIANTS);
    const cycle = makeCycle(SLOTS);
    const results = [];
    for (const v of variants) {
      log(`running ${v}`);
      results.push(runVariant(v, cycle, opts));
    }
    return results;
  }

  // ------------------------------------------------------- memory growth
  // Cost of growing one memory64 towards the 16 GiB browser cap. `rss` is an
  // optional callback (Node: process.memoryUsage().rss) to show whether grown
  // but untouched pages cost resident memory.
  function newMemory64(initial, maximum, shared) {
    try {
      const m = new WebAssembly.Memory({ address: 'i64', initial: BigInt(initial), maximum: BigInt(maximum), shared });
      return { m, big: true };
    } catch (e) {
      // Pre-standard API (Node 22 / V8 12.4): `index` and Number sizes.
      return { m: new WebAssembly.Memory({ index: 'i64', initial, maximum, shared }), big: false };
    }
  }

  function growthTest(opts = {}) {
    const now = opts.now || (() => performance.now());
    const rss = opts.rss || (() => NaN);
    const shared = opts.shared ?? true;
    const maxPages = opts.maxPages ?? 262144; // 16 GiB
    const steps = opts.steps || [16384, 65536, 131072, 245760]; // 1, 4, 8, 15 GiB
    const res = { shared, maxPages, rows: [] };
    let mem, big;
    const t0 = now();
    try {
      ({ m: mem, big } = newMemory64(1, maxPages, shared));
    } catch (e) {
      res.error = `create(max ${maxPages} pages): ${e.message}`;
      return res;
    }
    res.createMs = now() - t0;
    res.api = big ? 'final (address, BigInt)' : 'pre-standard (index, Number)';
    let pages = 1;
    for (const target of steps) {
      const t = now();
      try {
        mem.grow(big ? BigInt(target - pages) : target - pages);
      } catch (e) {
        res.rows.push({ gib: (target * 65536) / 2 ** 30, error: e.message });
        break;
      }
      pages = target;
      res.rows.push({ gib: (pages * 65536) / 2 ** 30, growMs: now() - t, rssMiB: rss() / MiB });
    }
    // Touch 256 MiB at the top of the grown memory, one byte per 4 KiB.
    const len = mem.buffer.byteLength;
    if (len >= DATA_BYTES) {
      const t = now();
      const u8 = new Uint8Array(mem.buffer, len - DATA_BYTES, DATA_BYTES);
      for (let i = 0; i < DATA_BYTES; i += 4096) u8[i] = 1;
      res.touch256MiBMs = now() - t;
      res.rssAfterTouchMiB = rss() / MiB;
    }
    return res;
  }

  // How many memories with a 16 GiB maximum one agent can hold at once (one
  // per Windows process under option A). Each grows to 1 GiB and touches one
  // page so the memory is real.
  function countMemories(opts = {}) {
    const limit = opts.limit ?? 64;
    const shared = opts.shared ?? true;
    const held = [];
    let error = null;
    try {
      while (held.length < limit) {
        const { m, big } = newMemory64(1, 262144, shared);
        m.grow(big ? 16383n : 16383);
        new Uint8Array(m.buffer)[m.buffer.byteLength - 1] = 1;
        held.push(m);
      }
    } catch (e) {
      error = e.message;
    }
    let tooBig = null;
    try {
      newMemory64(1, 262145, shared);
      tooBig = 'accepted';
    } catch (e) {
      tooBig = e.message;
    }
    return { shared, limit, created: held.length, error, maximum262145: tooBig };
  }

  globalThis.MemBench = { VARIANTS, buildModule, runAll, runVariant, growthTest, countMemories, MEM_PAGES, DATA_BYTES };
})();

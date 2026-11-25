// One Windows process on Wine (docs/design/wine-unix.md, section 12; abi.md
// v5, "Wine boundary"): the shared memory, `env.table` and `env.depth`, the
// bridge, the Unix side, the Linker in the host region, the dispatchers'
// slots, the boot set, and `boot` in run mode. Then it links translated code
// on demand, from the Unix side's plain imports:
// - tw_image_mapped: an AOT module from the catalog when the image's path,
//   sha256 and base match (wine-build.md, "The catalog"); otherwise a
//   fallback, counted, and the image's code is translated at run time;
// - tw_translate: the run-time translator on the executable view, with the
//   NULL, store and shared-memory flags; each translation is a fallback too;
// - tw_invalidate: every module whose code overlaps the range goes, whole.
// One process per instance: a program runs in a fresh one (abi.md v5,
// "Process exit"). Host-neutral: the caller passes the compiled modules and
// a synchronous reader of target/wine's files.

import {
  TABLE_RESERVED, RUN_INDEX, SYSCALL_DISPATCHER, UNIX_CALL_DISPATCHER, PAGE_CODE,
  LOOKUP_HEADER_SIZE, LOOKUP_SLOT_SIZE, MIN_LOOKUP_SLOTS,
} from '../abi.mjs';
import { createMemory64 } from '../memory.mjs';
import { GuestMemory } from '../guest-memory.mjs';
import { Linker } from '../link.mjs';
import { loadFp } from '../fp.mjs';
import { RuntimeTranslator, requestFlags } from '../rt.mjs';
import { parsePlan } from '../plan.mjs';
import { createUnixHost, INITIAL_PAGES, MAXIMUM_PAGES, DEFAULT_ENV } from './unix-host.mjs';
import { installBootSet, parseBootSetManifest, drivePath, boot, BOOT_RUN } from './boot-set.mjs';
import { parseCatalog, matchCatalog } from './aot-catalog.mjs';
import { writeHostFiles, collectHostFiles } from './host-files.mjs';

/** The request flags of every run-time translation in a Wine process (translator.md, "Wine processes"). */
export const WINE_RT_FLAGS = requestFlags({ nullCheck: true, storeCheck: true, sharedMemory: true, structured: true });

/** The program's directory on C: when the caller names none (wine-unix.md, section 8). */
export const DEFAULT_CWD = 'C:\\tempwind';

/**
 * The exit code native Wine's process gives a Windows status: get_unix_exit_code
 * (dlls/ntdll/unix/thread.c L89-94) turns a nonzero status with a zero low
 * byte into 1, and _exit keeps the low 8 bits. run() returns the full status.
 */
export function unixExitCode(status) {
  const s = status >>> 0;
  return s && !(s & 0xff) ? 1 : s & 0xff;
}

const hex = (v) => '0x' + BigInt.asUintN(64, BigInt(v)).toString(16);
const u64 = (v) => BigInt.asUintN(64, BigInt(v));

/** Hex sha256 of bytes, with WebCrypto (Node 24 and Workers). */
export async function sha256Hex(bytes) {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
  return Array.from(digest, (b) => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Creates the process, up to `boot` (section 12, steps 1-6 but the entry).
 * @param {object} opts
 * @param {Function} opts.factory  wine-unix.mjs's default export
 * @param {WebAssembly.Module} opts.wasm  wine-unix.wasm, compiled
 * @param {WebAssembly.Module} opts.bridge  bridge.wasm (`x64wasm-cli bridge`), compiled
 * @param {WebAssembly.Module|Uint8Array} opts.rt  x64wasm_rt.wasm
 * @param {WebAssembly.Module|Uint8Array} opts.fp  x64wasm_fp.wasm
 * @param {(source: string) => Uint8Array} opts.read  a file of target/wine by relative path, synchronously
 *   (the lazy nodes' sources, and catalog/<module>.wasm and .plan.json)
 * @param {object} opts.manifest  boot-set.json, parsed
 * @param {object|null} opts.catalog  catalog.json, parsed (null: every image falls back)
 * @param {{path: string, files: Record<string, Uint8Array>, dirs: string[], links: [string, string][],
 *   installFiles: string[]}} opts.prefix  readPrefix's (installFiles: drive_c's file paths, for the install copies)
 * @param {Uint8Array} opts.overlay  native/wine/prefix/overlay.reg
 * @param {{name: string, bytes: Uint8Array}} opts.program  written up front at C:\tempwind\<name>
 * @param {{path: string, bytes?: Uint8Array, dir?: boolean, mtime: number}[]} [opts.files]  host files written into
 *   drive_c before boot, with their mtimes (host-files.mjs; paths relative to drive_c, `mtime` in ms)
 * @param {string[]} [opts.args]  the program's arguments
 * @param {string} [opts.cwd]  the launch directory, a DOS path on C: (default C:\tempwind)
 * @param {Record<string,string>} [opts.env]  added to the Unix side's environment (WINEDEBUG goes here)
 * @param {(b: Uint8Array) => void} [opts.stdout]
 * @param {(b: Uint8Array) => void} [opts.stderr]
 * @param {(max: number) => Uint8Array|null} [opts.stdin]
 * @param {Map<string, {module: WebAssembly.Module, plan: object}>} [opts.moduleCache]  compiled catalog
 *   modules shared between processes of one host (each process still links its own instance)
 */
export async function createWineProcess(opts) {
  const stderr = opts.stderr ?? (() => {});
  const encoder = new TextEncoder();
  const say = (line) => stderr(encoder.encode(line + '\n'));
  const catalog = opts.catalog ? parseCatalog(opts.catalog) : null;
  const manifest = parseBootSetManifest(opts.manifest);
  const moduleCache = opts.moduleCache ?? new Map();
  const [fp, rt] = await Promise.all([loadFp(opts.fp), RuntimeTranslator.load(opts.rt)]);

  const prefixPath = opts.prefix.path.replace(/\/+$/, '');
  const programDos = `${DEFAULT_CWD}\\${opts.program.name}`;
  const programPath = drivePath(prefixPath, programDos);
  /** sha256 by MEMFS path: the manifest's, and the program's own. */
  const sha = new Map(manifest.map((f) => [f.path, f.sha256]));
  sha.set(programPath, await sha256Hex(opts.program.bytes));

  const stats = {
    /** Images linked from the catalog: { path, base, blocks }. */
    aot: [],
    /** Images mapped without an AOT module: { path, base, reason }. */
    imageFallbacks: [],
    /** Run-time translations: { rip, base, size, blocks }. */
    translations: [],
    /** Modules removed by tw_invalidate. */
    invalidated: 0,
    /** Lazy files loaded and their bytes (installBootSet). */
    lazy: { files: 0, bytes: 0 },
    /** Milliseconds from boot's entry to its return. */
    bootMs: null,
  };

  // 1. Memory, env.table, env.depth.
  const { memory } = createMemory64({ initialPages: INITIAL_PAGES, maximumPages: MAXIMUM_PAGES, shared: true });
  const mem = new GuestMemory(memory);
  const table = new WebAssembly.Table({ element: 'anyfunc', initial: TABLE_RESERVED });
  const depth = new WebAssembly.Global({ value: 'i32', mutable: true }, 0);

  // 2. The bridge: its element segment fills RUN_INDEX and TOMBSTONE_INDEX.
  const bridge = new WebAssembly.Instance(opts.bridge, { env: { memory, table, depth } }).exports;

  // 3. The Unix side, with the bridge's raw exports and the JS imports.
  let linker = null;
  let runInstalled = false;
  const needLinker = (name) => {
    if (!linker) throw new Error(`${name} before the linker exists (wine-unix.md, section 12)`);
    return linker;
  };
  const execRanges = (plan) => plan.sections.filter((s) => s.executable).map((s) => [s.va, s.mappedSize]);
  const linked = (l) => {
    if (!runInstalled) { table.set(RUN_INDEX, l.run); runInstalled = true; }
    for (const [start, size] of l.ranges) linker.setPageFlags(start, size, PAGE_CODE);
  };
  const loadAot = (entry) => {
    const key = `${entry.wasm} ${entry.sha256} ${entry.base}`;
    let m = moduleCache.get(key);
    if (!m) {
      const plan = parsePlan(JSON.parse(new TextDecoder().decode(opts.read(`catalog/${entry.plan}`))));
      if (plan.sha256 !== entry.sha256) throw new Error(`catalog/${entry.plan}: sha256 differs from catalog.json's for ${entry.path}`);
      if (plan.imageBase !== entry.base) throw new Error(`catalog/${entry.plan}: image base ${hex(plan.imageBase)}, catalog.json says ${hex(entry.base)}`);
      if (!plan.options.sharedMemory || !plan.options.nullCheck || !plan.options.storeCheck) {
        throw new Error(`catalog/${entry.plan}: not built with --shared --null-check --store-check`);
      }
      m = { module: new WebAssembly.Module(opts.read(`catalog/${entry.wasm}`)), plan };
      moduleCache.set(key, m);
    }
    return m;
  };

  const imports = {
    tw_depth_get: bridge.depth_get,
    tw_depth_set: bridge.depth_set,
    tw_flags_materialize: bridge.flags_materialize,
    // (fd i32, base i64, size i64) -> ()
    tw_image_mapped(fd, base, size) {
      const l = needLinker('tw_image_mapped');
      base = u64(base);
      let path = null;
      try { path = host.FS.getStream(fd)?.path ?? null; } catch { /* no stream: path stays null */ }
      const m = matchCatalog(catalog, path, path === null ? null : sha.get(path) ?? null, base);
      if (!m.entry) {
        stats.imageFallbacks.push({ path, base, reason: m.reason });
        return;
      }
      // A module the catalog promises but that can't be linked stops loudly:
      // the throw unwinds through the Unix side and rejects boot.
      let r;
      try {
        const aot = loadAot(m.entry);
        r = l.linkSync(aot.module, { ranges: execRanges(aot.plan) });
      } catch (e) {
        const msg = `tempwind: catalog: ${path}: ${e.message}`;
        say(msg);
        throw new Error(msg);
      }
      linked(r);
      stats.aot.push({ path, base, blocks: r.blocks });
    },
    // (rip i64, base i64, size i64) -> i32: 0 when rip then has a block
    tw_translate(rip, base, size) {
      const l = needLinker('tw_translate');
      rip = u64(rip); base = u64(base); size = u64(size);
      try {
        const known = keysIn(l.sortedKeys(), base, base + size);
        const wasm = rt.translate({ codeBase: base, code: mem.read(base, Number(size)), entry: rip, known, flags: WINE_RT_FLAGS });
        const r = l.linkSync(new WebAssembly.Module(wasm), { ranges: [[base, size]] });
        linked(r);
        stats.translations.push({ rip, base, size, blocks: r.blocks });
        if (!l.lookup(rip)) throw new Error('the translation has no block for it');
        return 0;
      } catch (e) {
        say(`tempwind: translate: ${hex(rip)} ${e.message}`);
        return 1;
      }
    },
    // (base i64, size i64) -> ()
    tw_invalidate(base, size) {
      stats.invalidated += needLinker('tw_invalidate').invalidate(u64(base), u64(size)).length;
    },
  };
  const host = await createUnixHost({
    factory: opts.factory, wasm: opts.wasm, memory,
    env: opts.env, stdout: opts.stdout, stderr, stdin: opts.stdin,
    run: bridge.run,
    imports,
  });
  const raw = host.raw;

  // 4. The lookup header and slots in the host region (raw malloc), and the
  // page-state table the Unix side allocated.
  const blocks = new Map(); // aligned address -> malloc'd pointer
  const alloc = (size, align) => {
    const p = raw.malloc(BigInt(size) + BigInt(align));
    if (p === 0n) throw new Error(`malloc(${size}) failed: the host heap is full`);
    const at = (p + BigInt(align) - 1n) / BigInt(align) * BigInt(align);
    blocks.set(at, p);
    return at;
  };
  const free = (at) => {
    const p = blocks.get(at);
    if (p !== undefined) { blocks.delete(at); raw.free(p); }
  };
  const header = alloc(LOOKUP_HEADER_SIZE, 8n);
  linker = new Linker({
    memory, header, slots: MIN_LOOKUP_SLOTS,
    slotsAddr: alloc(MIN_LOOKUP_SLOTS * LOOKUP_SLOT_SIZE, 16n),
    alloc, free, fp, table, depth,
    pageFlagsAddr: raw.tw_page_flags(),
  });

  // 5. The dispatchers: raw exports in env.table, and their lookup slots.
  linker.insert(SYSCALL_DISPATCHER, linker.addTableFunction(raw.tw_syscall_dispatcher), 0);
  linker.insert(UNIX_CALL_DISPATCHER, linker.addTableFunction(raw.tw_unix_call_dispatcher), 0);

  // 6. The boot set, and the launch directory.
  const loaded = installBootSet(host.FS, {
    prefixPath, prefixFiles: opts.prefix.files, prefixDirs: opts.prefix.dirs, prefixLinks: opts.prefix.links,
    installFiles: opts.prefix.installFiles,
    lazy: manifest.filter((f) => f.lazy),
    read: opts.read,
    programs: { [programDos]: opts.program.bytes },
    overlay: opts.overlay,
  });
  stats.lazy = loaded;
  const driveC = drivePath(prefixPath, 'C:\\');
  writeHostFiles(host.FS, driveC, opts.files ?? []);
  const cwdPath = drivePath(prefixPath, opts.cwd ?? DEFAULT_CWD);
  host.FS.chdir(cwdPath);

  return {
    host, linker, memory, table, depth, stats, programPath,
    /** The MEMFS path of the prefix's drive_c. */
    driveC,
    /** The JS imports the Unix side calls (tw_image_mapped, tw_translate, tw_invalidate), for tests. */
    imports,
    /**
     * The tree under drive_c/`dir` after the process has settled, in the form
     * of `opts.files` (host-files.mjs): paths relative to drive_c, sorted,
     * with bytes and mtimes.
     */
    collect: (dir) => collectHostFiles(host.FS, driveC, dir),
    /**
     * Enters boot, in run mode by default. Resolves with the exit status, the
     * full Windows status as a u32 (abi.md v5, "Process exit"); rejects when
     * the process stops loudly (the `tempwind:` line is on stderr). With
     * BOOT_STOP_AT_ENTRY it resolves with 0 at LdrInitializeThunk, with the
     * images mapped so far linked (tests).
     */
    async run(flags = BOOT_RUN) {
      const t0 = performance.now();
      try {
        const status = await boot(host, [programDos, ...(opts.args ?? [])], flags);
        return status >>> 0;
      } finally {
        stats.bootMs = performance.now() - t0;
      }
    },
  };
}

/** The keys of sorted `keys` in [lo, hi). */
function keysIn(keys, lo, hi) {
  const lower = (v) => {
    let a = 0, b = keys.length;
    while (a < b) { const m = (a + b) >>> 1; if (keys[m] < v) a = m + 1; else b = m; }
    return a;
  };
  return keys.slice(lower(lo), lower(hi));
}

export { DEFAULT_ENV };

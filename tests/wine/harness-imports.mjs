// The imports of the harness hosts (abi.md v5, "Host entries of the Unix
// side"), passed through createUnixHost's opts.imports. The harnesses link
// no translated code and have no bridge, so:
// - tw_flags_materialize is the bridge's flags_materialize for a materialized
//   record only (cc_op 0, which the synthetic state blocks have): the status
//   flags come from cc_a. Any other cc_op stops loudly: computing the flags
//   of a lazy record is the bridge's, emitted by the translator.
// - tw_image_mapped and tw_invalidate record their calls, with the path of
//   tw_image_mapped's fd and whether tw_invalidate's range still held a
//   nonzero byte, as they are when the call is made.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { STATE_CC_A, STATE_CC_OP, STATE_RFLAGS } from '../../runtime/src/abi.mjs';
import { createUnixHost, DEFAULT_ENV } from '../../runtime/src/wine/unix-host.mjs';

const root = new URL('../../', import.meta.url);
const dirUrl = (env, fallback) => (process.env[env]
  ? pathToFileURL(process.env[env].replace(/\/?$/, '/'))
  : new URL(fallback, root));
/** The Unix side's build (native/wine/build.sh) and the Wine assets of tests/wine/prepare.sh. */
export const unixDir = dirUrl('TW_WINE_UNIX_DIR', 'target/wine-unix/');
export const wineDir = dirUrl('TW_WINE_DIR', 'target/wine/');

const STATUS_FLAGS = 0x8d5n; // CF PF AF ZF SF OF

/**
 * @returns {{ imports: Record<string, Function>, attach(host): void,
 *   imageMapped: {fd: number, path: string|null, base: bigint, size: bigint}[],
 *   invalidated: {base: bigint, size: bigint, marked: boolean}[] }}
 */
export function harnessImports() {
  let host = null;
  const imageMapped = [];
  const invalidated = [];
  const u64 = (v) => BigInt.asUintN(64, BigInt(v));
  const imports = {
    tw_flags_materialize(state) {
      const s = Number(u64(state));
      const dv = new DataView(host.memory.buffer);
      const ccOp = dv.getUint32(s + STATE_CC_OP, true);
      if (ccOp !== 0) {
        throw new Error(`tw_flags_materialize: cc_op ${ccOp}; the test host has no bridge and materializes only cc_op 0`);
      }
      const rflags = dv.getBigUint64(s + STATE_RFLAGS, true);
      const value = (rflags & ~STATUS_FLAGS) | (dv.getBigUint64(s + STATE_CC_A, true) & STATUS_FLAGS);
      dv.setBigUint64(s + STATE_RFLAGS, value, true);
      dv.setBigUint64(s + STATE_CC_A, value, true);
    },
    tw_image_mapped(fd, base, size) {
      let path = null;
      try { path = host.FS.getStream(fd)?.path ?? null; } catch { /* recorded as null */ }
      imageMapped.push({ fd, path, base: u64(base), size: u64(size) });
    },
    tw_invalidate(base, size) {
      // whether the range still held a nonzero byte at the call: a harness
      // writes markers there, so the call came before the unmap or remap
      const b = Number(u64(base)), n = Math.min(Number(u64(size)), 1 << 20);
      const buf = host.memory.buffer;
      const bytes = b + n <= buf.byteLength ? new Uint8Array(buf, b, n) : new Uint8Array(0);
      invalidated.push({ base: u64(base), size: u64(size), marked: bytes.some((x) => x !== 0) });
    },
  };
  return { imports, attach(h) { host = h; }, imageMapped, invalidated };
}

// The server's init_directories reads l_intl.nls from the constant NLS path
// (section 8); init_registry starts from the prefix directory.
const NLS_DIR = '/wine/share/wine/nls';
let intl;
const compiled = new Map();

/**
 * A fresh instance of the harness test-NAME with stdout and stderr as text,
 * the prefix directory and l_intl.nls in MEMFS. The module is compiled once
 * per NAME.
 * @param {string} name
 * @param {{ env?: object, imports?: boolean, dosdevices?: boolean,
 *   setup?: (host) => void, run?: (state: bigint) => number }} [opts] env: the instance's environment;
 *   imports: with harnessImports(); dosdevices: the prefix's dosdevices
 *   directory (default true), else the prefix directory alone; setup: more
 *   MEMFS content, after the standard; run: tw_run in place of the host's
 *   default, which throws (no translated module).
 * @returns {Promise<{ host, out: {stdout: string, stderr: string}, harness: ReturnType<typeof harnessImports>|undefined }>}
 */
export async function startHarness(name, { env = {}, imports = false, dosdevices = true, setup, run } = {}) {
  if (!compiled.has(name)) {
    const { default: factory } = await import(new URL(`test-${name}.mjs`, unixDir));
    const wasm = new WebAssembly.Module(readFileSync(new URL(`test-${name}.wasm`, unixDir)));
    compiled.set(name, { factory, wasm });
  }
  intl ??= readFileSync(new URL('native/opt/wine-11.0/share/wine/nls/l_intl.nls', wineDir));
  const out = { stdout: '', stderr: '' };
  const dec = () => { const d = new TextDecoder(); return (b) => d.decode(b, { stream: true }); };
  const o = dec(), e = dec();
  const harness = imports ? harnessImports() : undefined;
  const host = await createUnixHost({
    ...compiled.get(name), env,
    stdout: (b) => { out.stdout += o(b); },
    stderr: (b) => { out.stderr += e(b); },
    ...(harness && { imports: harness.imports }),
    ...(run && { run }),
  });
  harness?.attach(host);
  host.FS.mkdirTree(dosdevices ? `${DEFAULT_ENV.WINEPREFIX}/dosdevices` : DEFAULT_ENV.WINEPREFIX);
  host.FS.mkdirTree(NLS_DIR);
  host.FS.writeFile(`${NLS_DIR}/l_intl.nls`, intl);
  setup?.(host);
  return { host, out, harness };
}

/**
 * Enters the harness's tw_test_main and asserts the checks that ran are
 * exactly `checks`, in order, with no `not ok` line, `done N/N` last and a
 * return of 0.
 * @returns {Promise<string[]>} the non-empty stdout lines
 */
export async function runChecks({ host, out }, checks) {
  const ret = await host.call('tw_test_main');
  const lines = out.stdout.split('\n').filter((l) => l);
  assert.deepEqual(lines.filter((l) => l.startsWith('not ok')), [], out.stdout + out.stderr);
  assert.deepEqual(lines.filter((l) => l.startsWith('ok ')).map((l) => l.slice(3)), checks);
  assert.equal(lines.at(-1), `done ${checks.length}/${checks.length}`);
  assert.equal(ret, 0);
  return lines;
}

// M3's imports for the harness hosts (abi.md v5, "Host entries of the Unix
// side"), passed through createUnixHost's opts.imports. The harnesses link
// no translated code and have no bridge, so:
// - tw_flags_materialize is the bridge's flags_materialize for a materialized
//   record only (cc_op 0, which the synthetic state blocks have): the status
//   flags come from cc_a. Any other cc_op stops loudly: computing the flags
//   of a lazy record is the bridge's, emitted by the translator.
// - tw_image_mapped and tw_invalidate record their calls, with the path of
//   tw_image_mapped's fd and whether tw_invalidate's range still held a
//   nonzero byte, as they are when the call is made.
import { STATE_CC_A, STATE_CC_OP, STATE_RFLAGS } from '../../runtime/src/abi.mjs';

const STATUS_FLAGS = 0x8d5n; // CF PF AF ZF SF OF

/**
 * @returns {{ imports: Record<string, Function>, attach(host): void,
 *   imageMapped: {fd: number, path: string|null, base: bigint, size: bigint}[],
 *   invalidated: {base: bigint, size: bigint, marked: boolean}[] }}
 */
export function m3Imports() {
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

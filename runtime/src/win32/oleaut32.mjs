// oleaut32.dll (runtime.md, "The rest"): BSTRs in Windows' layout (a 4-byte
// byte-length prefix and a NUL terminator, the string 8-byte aligned) and
// VariantClear/VariantCopy for the VARIANT types 7-Zip uses. Details follow
// Microsoft's documentation and Wine 11 dlls/oleaut32 (oleaut.c, variant.c).

import { crt } from './msvcrt.mjs';
import { HeapError } from './crtheap.mjs';
import { hex, fn } from './util.mjs';

const S_OK = 0;
const VT_EMPTY = 0, VT_BSTR = 8, VT_BOOL = 11, VT_UI4 = 19, VT_UI8 = 21, VT_FILETIME = 64;
const SIMPLE = new Set([VT_EMPTY, VT_BOOL, VT_UI4, VT_UI8, VT_FILETIME]);
export const VARIANT_SIZE = 24;

const bstrs = (ctx) => ctx.dllState('oleaut32.dll', () => ({ live: new Set() }));

/** Allocates a BSTR of `bytes` bytes (plus the prefix and a 2-byte NUL); returns the string pointer. */
export function allocBstr(ctx, bytes) {
  if (bytes > 0xffff_fff0) return 0n;
  let block;
  try { block = crt(ctx).heap.alloc(BigInt(8 + bytes + 2)); } catch (e) {
    if (e instanceof HeapError) ctx.fail(e.message);
    throw e;
  }
  const p = block + 8n;
  ctx.write32(p - 4n, bytes);
  ctx.mem.fill(p, bytes + 2);
  bstrs(ctx).live.add(p);
  return p;
}

export function freeBstr(ctx, p, api) {
  if (p === 0n) return;
  const live = bstrs(ctx).live;
  if (!live.has(p)) ctx.fail(`${api}: ${hex(p)} is not a live BSTR`);
  live.delete(p);
  try { crt(ctx).heap.freeBlock(p - 8n); } catch (e) {
    if (e instanceof HeapError) ctx.fail(e.message);
    throw e;
  }
}

const byteLen = (ctx, p) => (p === 0n ? 0 : ctx.read32(p - 4n));

function variantClear(ctx, v) {
  const vt = ctx.mem.u16(ctx.check(v, VARIANT_SIZE, 'VARIANT'));
  if (vt === VT_BSTR) freeBstr(ctx, ctx.read64(v + 8n), 'VariantClear');
  else if (!SIMPLE.has(vt)) ctx.fail(`VariantClear: VARIANT type ${vt} is not supported`);
  ctx.mem.setU16(v, VT_EMPTY);
}

export default {
  // BSTR SysAllocString(const OLECHAR *psz)
  SysAllocString: fn(1, (ctx) => {
    const s = ctx.arg(0);
    if (s === 0n) return 0n;
    const n = ctx.readWString(s).length;
    const p = allocBstr(ctx, 2 * n);
    if (n) ctx.write(p, ctx.read(s, 2 * n));
    return p;
  }),
  // BSTR SysAllocStringLen(const OLECHAR *strIn, UINT ui): ui characters, copied if strIn isn't NULL.
  SysAllocStringLen: fn(2, (ctx) => {
    const s = ctx.arg(0), n = ctx.arg32(1);
    if (n >= 0x8000_0000) return 0n;
    const p = allocBstr(ctx, 2 * n);
    if (p !== 0n && s !== 0n && n) ctx.write(p, ctx.read(s, 2 * n));
    return p;
  }),
  // void SysFreeString(BSTR)
  SysFreeString: fn(1, (ctx) => { freeBstr(ctx, ctx.arg(0), 'SysFreeString'); }),
  // UINT SysStringLen(BSTR)
  SysStringLen: fn(1, (ctx) => byteLen(ctx, ctx.arg(0)) >>> 1),

  // HRESULT VariantClear(VARIANTARG *pvarg)
  VariantClear: fn(1, (ctx) => { variantClear(ctx, ctx.arg(0)); return S_OK; }),
  // HRESULT VariantCopy(VARIANTARG *pvargDest, const VARIANTARG *pvargSrc)
  VariantCopy: fn(2, (ctx) => {
    const dst = ctx.arg(0), src = ctx.arg(1);
    const vt = ctx.mem.u16(ctx.check(src, VARIANT_SIZE, 'VARIANT'));
    if (vt !== VT_BSTR && !SIMPLE.has(vt)) ctx.fail(`VariantCopy: VARIANT type ${vt} is not supported`);
    if (dst === src) return S_OK;
    variantClear(ctx, dst);
    ctx.write(dst, ctx.read(src, VARIANT_SIZE));
    if (vt === VT_BSTR) {
      const b = ctx.read64(src + 8n);
      if (b !== 0n) {
        const n = byteLen(ctx, b);
        const p = allocBstr(ctx, n);
        if (n) ctx.write(p, ctx.read(b, n));
        ctx.write64(dst + 8n, p);
      }
    }
    return S_OK;
  }),
};

// ntdll.dll (runtime.md, "The machine profile"): the function 7-Zip reaches
// through GetProcAddress. Details follow Microsoft's documentation and Wine 11
// dlls/ntdll (version.c).

import { OS_VERSION } from '../abi.mjs';
import { fn } from './util.mjs';

/** sizeof(RTL_OSVERSIONINFOW) and sizeof(RTL_OSVERSIONINFOEXW). */
export const RTL_OSVERSIONINFOW_SIZE = 276;
export const RTL_OSVERSIONINFOEXW_SIZE = 284;
/** The product reported: a workstation (Windows 11 Pro), single-user Terminal Services. */
export const VER_NT_WORKSTATION = 1;
export const VER_SUITE_SINGLEUSERTS = 0x100;

export default {
  // NTSTATUS RtlGetVersion(PRTL_OSVERSIONINFOW lpVersionInformation): the real version, never the
  // manifest-dependent one GetVersion reports.
  RtlGetVersion: fn(1, (ctx) => {
    const p = ctx.arg(0);
    const size = ctx.mem.u32(ctx.check(p, 4, 'RTL_OSVERSIONINFOW'));
    if (size !== RTL_OSVERSIONINFOW_SIZE && size !== RTL_OSVERSIONINFOEXW_SIZE) {
      ctx.fail(`RtlGetVersion: dwOSVersionInfoSize ${size} is neither ${RTL_OSVERSIONINFOW_SIZE} nor ${RTL_OSVERSIONINFOEXW_SIZE}`);
    }
    ctx.check(p, size, 'RTL_OSVERSIONINFOW');
    ctx.mem.setU32(p + 4n, OS_VERSION.major);
    ctx.mem.setU32(p + 8n, OS_VERSION.minor);
    ctx.mem.setU32(p + 12n, OS_VERSION.build);
    ctx.mem.setU32(p + 16n, OS_VERSION.platformId);
    ctx.mem.fill(p + 20n, 256); // szCSDVersion: no service pack
    if (size === RTL_OSVERSIONINFOEXW_SIZE) {
      ctx.mem.setU16(p + 276n, 0); // wServicePackMajor
      ctx.mem.setU16(p + 278n, 0); // wServicePackMinor
      ctx.mem.setU16(p + 280n, VER_SUITE_SINGLEUSERTS);
      ctx.mem.setU8(p + 282n, VER_NT_WORKSTATION);
      ctx.mem.setU8(p + 283n, 0);
    }
    return 0; // STATUS_SUCCESS
  }),
};

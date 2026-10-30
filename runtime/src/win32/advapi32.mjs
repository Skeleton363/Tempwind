// advapi32.dll (runtime.md, "The rest"): an empty registry, the token of a
// normal user, and RtlGenRandom. Registry writes, file security and other
// cases stop loudly. Details follow Microsoft's documentation and Wine 11
// dlls/advapi32 and kernelbase/registry.c.

import { CURRENT_PROCESS, addHandle, getHandle } from './handles.mjs';
import { hex, fn, wstr } from './util.mjs';
import { ERROR_SUCCESS, ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER, ERROR_NOT_ALL_ASSIGNED, ERROR_INVALID_HANDLE } from './errors.mjs';

/** The predefined registry keys (HKEY_CLASSES_ROOT .. HKEY_CURRENT_USER_LOCAL_SETTINGS). */
const PREDEFINED_KEYS = new Map([
  [0x8000_0000n, 'HKEY_CLASSES_ROOT'], [0x8000_0001n, 'HKEY_CURRENT_USER'], [0x8000_0002n, 'HKEY_LOCAL_MACHINE'],
  [0x8000_0003n, 'HKEY_USERS'], [0x8000_0004n, 'HKEY_PERFORMANCE_DATA'], [0x8000_0005n, 'HKEY_CURRENT_CONFIG'],
]);
const ERROR_NO_MORE_ITEMS = 259;

/** The privilege names and their LUIDs (winnt.h SE_*_PRIVILEGE). */
export const PRIVILEGES = [
  null, null, 'SeCreateTokenPrivilege', 'SeAssignPrimaryTokenPrivilege', 'SeLockMemoryPrivilege',
  'SeIncreaseQuotaPrivilege', 'SeMachineAccountPrivilege', 'SeTcbPrivilege', 'SeSecurityPrivilege',
  'SeTakeOwnershipPrivilege', 'SeLoadDriverPrivilege', 'SeSystemProfilePrivilege', 'SeSystemtimePrivilege',
  'SeProfileSingleProcessPrivilege', 'SeIncreaseBasePriorityPrivilege', 'SeCreatePagefilePrivilege',
  'SeCreatePermanentPrivilege', 'SeBackupPrivilege', 'SeRestorePrivilege', 'SeShutdownPrivilege',
  'SeDebugPrivilege', 'SeAuditPrivilege', 'SeSystemEnvironmentPrivilege', 'SeChangeNotifyPrivilege',
  'SeRemoteShutdownPrivilege', 'SeUndockPrivilege', 'SeSyncAgentPrivilege', 'SeEnableDelegationPrivilege',
  'SeManageVolumePrivilege', 'SeImpersonatePrivilege', 'SeCreateGlobalPrivilege',
  'SeTrustedCredManAccessPrivilege', 'SeRelabelPrivilege', 'SeIncreaseWorkingSetPrivilege',
  'SeTimeZonePrivilege', 'SeCreateSymbolicLinkPrivilege', 'SeDelegateSessionUserImpersonatePrivilege',
];
/** The privileges a normal user's token holds (shutdown, change notify, undock, working set, time zone). */
export const USER_PRIVILEGES = new Set([19, 23, 25, 33, 34]);

const TOKEN_ADJUST_PRIVILEGES = 0x20, TOKEN_QUERY = 0x8;
const SE_PRIVILEGE_ENABLED = 0x2, SE_PRIVILEGE_REMOVED = 0x4;

function predefined(ctx, key, api) {
  const k = BigInt.asUintN(64, key);
  const name = PREDEFINED_KEYS.get(k) ?? PREDEFINED_KEYS.get(BigInt.asUintN(32, k));
  if (!name) ctx.fail(`${api}: unknown key ${hex(key)} (the registry is empty, so no key is ever opened)`);
  return name;
}

const registryWrite = (name, args) => fn(args, (ctx) => ctx.fail(`${name}: registry writes are not supported`));

export default {
  // LSTATUS RegOpenKeyExW(HKEY hKey, LPCWSTR lpSubKey, DWORD ulOptions, REGSAM samDesired, PHKEY phkResult)
  RegOpenKeyExW: fn(5, (ctx) => {
    const root = predefined(ctx, ctx.arg(0), 'RegOpenKeyExW');
    const sub = ctx.arg(1) === 0n ? '' : wstr(ctx, ctx.arg(1));
    if (!sub) ctx.fail(`RegOpenKeyExW(${root}) without a subkey is not supported`);
    if (ctx.arg(4) === 0n) return ERROR_INVALID_PARAMETER;
    ctx.write64(ctx.arg(4), 0n);
    return ERROR_FILE_NOT_FOUND;
  }),
  // LSTATUS RegCloseKey(HKEY): only predefined keys exist.
  RegCloseKey: fn(1, (ctx) => { predefined(ctx, ctx.arg(0), 'RegCloseKey'); return ERROR_SUCCESS; }),
  // LSTATUS RegQueryValueExW(HKEY, LPCWSTR lpValueName, LPDWORD, LPDWORD lpType, LPBYTE lpData, LPDWORD lpcbData)
  RegQueryValueExW: fn(6, (ctx) => { predefined(ctx, ctx.arg(0), 'RegQueryValueExW'); return ERROR_FILE_NOT_FOUND; }),
  // LSTATUS RegEnumKeyExW(HKEY, DWORD dwIndex, LPWSTR, LPDWORD, LPDWORD, LPWSTR, LPDWORD, PFILETIME)
  RegEnumKeyExW: fn(8, (ctx) => { predefined(ctx, ctx.arg(0), 'RegEnumKeyExW'); return ERROR_NO_MORE_ITEMS; }),
  RegCreateKeyExW: registryWrite('RegCreateKeyExW', 9),
  RegSetValueExW: registryWrite('RegSetValueExW', 6),
  RegDeleteKeyW: registryWrite('RegDeleteKeyW', 2),
  RegDeleteValueW: registryWrite('RegDeleteValueW', 2),

  // BOOL OpenProcessToken(HANDLE ProcessHandle, DWORD DesiredAccess, PHANDLE TokenHandle)
  OpenProcessToken: fn(3, (ctx) => {
    if (BigInt.asUintN(64, ctx.arg(0)) !== CURRENT_PROCESS) ctx.fail('OpenProcessToken: only the current process is supported');
    const access = ctx.arg32(1);
    if (access & ~(TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY)) ctx.fail(`DesiredAccess ${hex(access)} is not supported`);
    const h = addHandle(ctx, { kind: 'token', access, enabled: new Set([23]) });
    ctx.write64(ctx.arg(2), h);
    return 1;
  }),
  // BOOL LookupPrivilegeValueW(LPCWSTR lpSystemName, LPCWSTR lpName, PLUID lpLuid)
  LookupPrivilegeValueW: fn(3, (ctx) => {
    if (ctx.arg(0) !== 0n) ctx.fail('lpSystemName must be NULL (the local system)');
    const name = wstr(ctx, ctx.arg(1), 'lpName').toLowerCase();
    const luid = PRIVILEGES.findIndex((p) => p && p.toLowerCase() === name);
    if (luid < 0) { ctx.setLastError(1313); return 0; } // ERROR_NO_SUCH_PRIVILEGE
    ctx.write64(ctx.arg(2), BigInt(luid));
    return 1;
  }),
  // BOOL AdjustTokenPrivileges(HANDLE, BOOL DisableAllPrivileges, PTOKEN_PRIVILEGES NewState,
  //   DWORD BufferLength, PTOKEN_PRIVILEGES PreviousState, PDWORD ReturnLength)
  AdjustTokenPrivileges: fn(6, (ctx) => {
    const tok = getHandle(ctx, ctx.arg(0), 'TokenHandle', 'token');
    if (!(tok.access & TOKEN_ADJUST_PRIVILEGES)) { ctx.setLastError(5); return 0; }
    if (ctx.arg32(1) !== 0) ctx.fail('DisableAllPrivileges is not supported');
    if (ctx.arg(4) !== 0n || ctx.arg(5) !== 0n) ctx.fail('PreviousState is not supported');
    const p = ctx.arg(2);
    const count = ctx.read32(p);
    let all = true;
    for (let i = 0; i < count; i++) {
      const e = p + 4n + 12n * BigInt(i);
      const luid = ctx.read64(e), attrs = ctx.read32(e + 8n);
      if (attrs & SE_PRIVILEGE_REMOVED) ctx.fail('SE_PRIVILEGE_REMOVED is not supported');
      const id = Number(luid);
      if (!USER_PRIVILEGES.has(id)) { all = false; continue; }
      if (attrs & SE_PRIVILEGE_ENABLED) tok.enabled.add(id); else tok.enabled.delete(id);
    }
    ctx.setLastError(all ? ERROR_SUCCESS : ERROR_NOT_ALL_ASSIGNED);
    return 1;
  }),

  GetFileSecurityW: fn(5, (ctx) => ctx.fail('NT security (-sni) is not supported')),
  SetFileSecurityW: fn(3, (ctx) => ctx.fail('NT security (-sni) is not supported')),

  // BOOLEAN SystemFunction036(PVOID RandomBuffer, ULONG RandomBufferLength): RtlGenRandom.
  SystemFunction036: fn(2, (ctx) => {
    const n = ctx.arg32(1);
    if (n === 0) return 1;
    const bytes = new Uint8Array(n);
    for (let i = 0; i < n; i += 65536) globalThis.crypto.getRandomValues(bytes.subarray(i, Math.min(n, i + 65536)));
    ctx.write(ctx.arg(0), bytes);
    return 1;
  }),
};

export { ERROR_INVALID_HANDLE };

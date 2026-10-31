// The host-call function and shim v0, driven directly from JS against a
// state block: Win64 argument reading, the guest's ret, stops.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  STATE_RIP, STATE_EXIT, STATE_GS_BASE, STATE_CC_OP, STATE_CC_A, EXIT_HOST, STATUS_RETURNED, STATUS_UNWIND, STATUS_EXIT,
  NO_RET, TEB_ADDR, STDOUT_HANDLE, STDERR_HANDLE, STDIN_HANDLE, INVALID_HANDLE_VALUE, gprOffset,
} from '../../runtime/src/abi.mjs';
import { createMemory64 } from '../../runtime/src/memory.mjs';
import { GuestMemory } from '../../runtime/src/guest-memory.mjs';
import { makeHostCall, HostContext } from '../../runtime/src/hostcall.mjs';
import { SHIM, findShim } from '../../runtime/src/win32/index.mjs';

const STATE = 0x1_1000n;
const RSP0 = 0x1_8000n;
const RET = 0x4000_0010n;
const R = { rax: 0, rcx: 1, rdx: 2, rbx: 3, rsp: 4, r8: 8, r9: 9, r15: 15 };
const TEB = 0x1_3000n;

function setup(names = ['GetStdHandle', 'WriteFile', 'ExitProcess', 'Missing']) {
  const { memory } = createMemory64({ initialPages: 4 });
  const mem = new GuestMemory(memory);
  const out = [];
  const stops = [];
  const lines = [];
  const imports = names.map((name, id) => ({ dll: 'KERNEL32.dll', name, thunk: 0x9000n + 16n * BigInt(id), impl: findShim('KERNEL32.dll', name) }));
  const call = makeHostCall({
    mem, imports, output: (s, b) => out.push([s, new TextDecoder().decode(b)]),
    onStop: (s) => stops.push(s), trace: (l) => lines.push(l),
  });
  const reg = (n) => mem.u64(STATE + BigInt(gprOffset(R[n])));
  const setReg = (n, v) => mem.setU64(STATE + BigInt(gprOffset(R[n])), v);
  setReg('rsp', RSP0);
  mem.setU64(RSP0, RET);
  mem.setU64(STATE + BigInt(STATE_GS_BASE), TEB);
  // Values that must survive a host call.
  setReg('rbx', 0x1111n);
  setReg('r15', 0x2222n);
  mem.setU32(STATE + BigInt(STATE_CC_OP), 5);
  mem.setU64(STATE + BigInt(STATE_CC_A), 0x3333n);
  const id = (name) => names.indexOf(name);
  return { mem, out, stops, lines, call, reg, setReg, id };
}

const untouched = (t) => {
  assert.equal(t.reg('rbx'), 0x1111n);
  assert.equal(t.reg('r15'), 0x2222n);
  assert.equal(t.mem.u32(STATE + BigInt(STATE_CC_OP)), 5);
  assert.equal(t.mem.u64(STATE + BigInt(STATE_CC_A)), 0x3333n);
};

test('Win64 arguments: four registers, then [RSP + 0x28 + 8 * (i - 4)]', () => {
  const t = setup();
  ['rcx', 'rdx', 'r8', 'r9'].forEach((n, i) => t.setReg(n, 0x100n + BigInt(i)));
  for (let i = 4; i < 7; i++) t.mem.setU64(RSP0 + 0x28n + 8n * BigInt(i - 4), 0x100n + BigInt(i));
  const ctx = new HostContext(t.mem, STATE, () => {});
  assert.deepEqual([0, 1, 2, 3, 4, 5, 6].map((i) => ctx.arg(i)), [0x100n, 0x101n, 0x102n, 0x103n, 0x104n, 0x105n, 0x106n]);
  t.setReg('rcx', 0xdead_beef_ffff_fff5n);
  assert.equal(ctx.arg32(0), 0xffff_fff5);
  assert.equal(ctx.argI32(0), -11);
});

test('return with ret == [RSP]: RAX written, RSP += 8, STATUS_RETURNED', () => {
  const t = setup();
  t.setReg('rcx', 0xdead_beef_ffff_fff5n); // STD_OUTPUT_HANDLE; the upper half is ignored
  assert.equal(t.call(STATE, t.id('GetStdHandle'), RET), STATUS_RETURNED);
  assert.equal(t.reg('rax'), STDOUT_HANDLE);
  assert.equal(t.reg('rsp'), RSP0 + 8n);
  assert.equal(t.mem.u64(STATE + BigInt(STATE_RIP)), 0n, 'RIP not stored');
  untouched(t);
  assert.deepEqual(t.lines, ['kernel32.dll!GetStdHandle(0xdeadbeeffffffff5) = 0x7']);
});

test('return with ret != [RSP]: state.rip = [RSP], STATUS_UNWIND', () => {
  const t = setup();
  t.setReg('rcx', 0xffff_fff4n);
  // ret arrives from wasm as a signed i64: NO_RET is -1n.
  assert.equal(t.call(STATE, t.id('GetStdHandle'), -1n), STATUS_UNWIND);
  assert.equal(t.reg('rax'), STDERR_HANDLE);
  assert.equal(t.reg('rsp'), RSP0 + 8n);
  assert.equal(t.mem.u64(STATE + BigInt(STATE_RIP)), RET);
  untouched(t);
  assert.equal(NO_RET, 0xffff_ffff_ffff_ffffn);
});

test('GetStdHandle: the three standard handles; others fail with ERROR_INVALID_HANDLE', () => {
  const t = setup();
  t.setReg('rcx', 0xffff_fff6n);
  t.call(STATE, t.id('GetStdHandle'), RET);
  assert.equal(t.reg('rax'), STDIN_HANDLE);
  t.mem.setU64(RSP0 + 8n, RET);
  t.setReg('rcx', 5n);
  assert.equal(t.call(STATE, t.id('GetStdHandle'), RET), STATUS_RETURNED);
  assert.equal(t.reg('rax'), INVALID_HANDLE_VALUE);
  assert.equal(t.mem.u32(TEB + 0x68n), 6);
});

const writeFile = (t, { handle = STDOUT_HANDLE, buf = 0x1_2000n, text = 'hello\n', written = 0x1_2800n, overlapped = 0n } = {}) => {
  const bytes = new TextEncoder().encode(text);
  t.mem.write(0x1_2000n, bytes);
  t.setReg('rsp', RSP0);
  t.mem.setU64(RSP0, RET);
  t.setReg('rcx', handle);
  t.setReg('rdx', buf);
  t.setReg('r8', 0xffff_ffff_0000_0000n | BigInt(bytes.length));
  t.setReg('r9', written);
  t.mem.setU64(RSP0 + 0x28n, overlapped);
  t.mem.setU32(0x1_2800n, 0xaaaa_aaaa);
  return t.call(STATE, t.id('WriteFile'), RET);
};

test('WriteFile to stdout and stderr appends the bytes and sets *lpNumberOfBytesWritten', () => {
  const t = setup();
  assert.equal(writeFile(t), STATUS_RETURNED);
  assert.equal(t.reg('rax'), 1n);
  assert.equal(t.mem.u32(0x1_2800n), 6);
  assert.equal(writeFile(t, { handle: STDERR_HANDLE, text: 'oops' }), STATUS_RETURNED);
  assert.deepEqual(t.out, [['stdout', 'hello\n'], ['stderr', 'oops']]);
  assert.equal(t.lines[0], 'kernel32.dll!WriteFile(0x7, 0x12000, 0xffffffff00000006, 0x12800, 0x0) = 0x1');
});

for (const [label, args, re] of [
  ['an OVERLAPPED', { overlapped: 0x1_2c00n }, /^kernel32.dll!WriteFile: lpOverlapped 0x12c00: overlapped I\/O is not supported$/],
  ['a NULL lpNumberOfBytesWritten', { written: 0n }, /lpNumberOfBytesWritten is NULL/],
  ['an unknown handle', { handle: 0x44n }, /hFile 0x44: unknown handle/],
  ['the stdin handle', { handle: STDIN_HANDLE }, /hFile 0x3: not a writable handle/],
  ['a NULL buffer', { buf: 0n }, /bad pointer 0x0 \(6 bytes\)/],
  ['a buffer past memory', { buf: 0x3_fffen }, /bad pointer 0x3fffe/],
]) {
  test(`WriteFile with ${label} is an error stop`, () => {
    const t = setup();
    assert.equal(writeFile(t, args), STATUS_EXIT);
    assert.equal(t.stops.length, 1);
    assert.match(t.stops[0].error, re);
    assert.equal(t.mem.u32(STATE + BigInt(STATE_EXIT)), EXIT_HOST);
    assert.equal(t.mem.u64(STATE + BigInt(STATE_RIP)), 0x9010n, 'rip = the thunk');
    assert.equal(t.reg('rsp'), RSP0, 'no ret on a stop');
    assert.equal(t.reg('rax'), 0n);
    assert.deepEqual(t.out, []);
    untouched(t);
  });
}

test('ExitProcess stops with the exit code (a UINT)', () => {
  const t = setup();
  t.setReg('rcx', 0x1234_5678_ffff_fffen);
  assert.equal(t.call(STATE, t.id('ExitProcess'), RET), STATUS_EXIT);
  assert.deepEqual(t.stops, [{ exitCode: 0xffff_fffe }]);
  assert.equal(t.mem.u64(STATE + BigInt(STATE_RIP)), 0x9020n);
  assert.equal(t.mem.u32(STATE + BigInt(STATE_EXIT)), EXIT_HOST);
  assert.equal(t.lines[0], 'kernel32.dll!ExitProcess(0x12345678fffffffe) -> exit 4294967294');
});

test('an import the shim lacks is an error stop', () => {
  const t = setup();
  assert.equal(t.call(STATE, t.id('Missing'), RET), STATUS_EXIT);
  assert.deepEqual(t.stops, [{ error: 'unimplemented API kernel32.dll!Missing' }]);
  assert.equal(t.call(STATE, 99, RET), STATUS_EXIT);
  assert.match(t.stops[1].error, /unknown import id 99/);
});

test('a bad RSP at the ret is an error stop', () => {
  const t = setup();
  t.setReg('rcx', 0xffff_fff5n);
  t.setReg('rsp', 0x7_0000_0000n);
  assert.equal(t.call(STATE, t.id('GetStdHandle'), RET), STATUS_EXIT);
  assert.match(t.stops[0].error, /return address: guest access out of memory/);
});

test('shim lookup is case-insensitive in the DLL only', () => {
  assert.ok(findShim('KERNEL32.DLL', 'WriteFile'));
  assert.equal(findShim('kernel32.dll', 'writefile'), undefined);
  assert.equal(findShim('kernel32.dll', 'toString'), undefined);
  assert.deepEqual(Object.keys(SHIM['kernel32.dll']).sort(), [
    'CloseHandle', 'CompareFileTime', 'CreateDirectoryW', 'CreateEventW', 'CreateFileW', 'CreateSemaphoreW',
    'DeleteCriticalSection', 'DeleteFileW', 'DosDateTimeToFileTime', 'EnterCriticalSection', 'ExitProcess', 'ExitThread',
    'FileTimeToDosDateTime', 'FileTimeToLocalFileTime', 'FileTimeToSystemTime', 'FindClose', 'FindFirstFileW',
    'FindFirstStreamW', 'FindNextFileW', 'FindNextStreamW', 'FormatMessageW', 'FreeLibrary',
    'GetActiveProcessorCount', 'GetActiveProcessorGroupCount', 'GetCommandLineW', 'GetConsoleMode',
    'GetConsoleScreenBufferInfo', 'GetCurrentDirectoryW', 'GetCurrentProcess', 'GetCurrentProcessId',
    'GetCurrentThread', 'GetCurrentThreadId', 'GetDiskFreeSpaceExW', 'GetDiskFreeSpaceW', 'GetDriveTypeW',
    'GetExitCodeThread', 'GetFileAttributesW', 'GetFileInformationByHandle', 'GetFileSize', 'GetLargePageMinimum', 'GetLastError',
    'GetLogicalDriveStringsW', 'GetModuleFileNameW', 'GetModuleHandleA', 'GetModuleHandleW', 'GetOEMCP',
    'GetProcAddress', 'GetProcessAffinityMask', 'GetProcessTimes', 'GetStdHandle', 'GetSystemDirectoryW',
    'GetSystemInfo', 'GetSystemTimeAsFileTime', 'GetTempPathW', 'GetThreadId', 'GetTickCount', 'GetVersion',
    'GetVolumeInformationW', 'GetWindowsDirectoryW', 'GlobalMemoryStatusEx', 'InitializeCriticalSection',
    'IsDBCSLeadByteEx', 'IsProcessorFeaturePresent', 'LeaveCriticalSection', 'LoadLibraryExW', 'LoadLibraryW',
    'LocalFileTimeToFileTime', 'LocalFree', 'MoveFileW', 'MoveFileWithProgressW', 'MultiByteToWideChar', 'OpenEventW',
    'OpenFileMappingW', 'QueryPerformanceCounter', 'QueryPerformanceFrequency', 'ReadFile', 'ReleaseSemaphore',
    'RemoveDirectoryW', 'ResetEvent', 'ResumeThread', 'SetConsoleCtrlHandler', 'SetConsoleMode', 'SetCurrentDirectoryW',
    'SetDefaultDllDirectories', 'SetEndOfFile', 'SetEvent', 'SetFileApisToOEM', 'SetFileAttributesW',
    'SetFilePointer', 'SetFileTime', 'SetLastError', 'SetProcessAffinityMask', 'SetThreadAffinityMask',
    'SetUnhandledExceptionFilter', 'Sleep', 'SwitchToThread', 'TlsAlloc', 'TlsFree', 'TlsGetValue', 'TlsSetValue',
    'TryEnterCriticalSection', 'VirtualAlloc', 'VirtualFree', 'VirtualProtect', 'VirtualQuery',
    'WaitForMultipleObjects', 'WaitForSingleObject', 'WideCharToMultiByte', 'WriteFile', 'lstrlenW',
  ]);
});

test('SetLastError writes TEB.LastErrorValue of the GS base', () => {
  const t = setup();
  new HostContext(t.mem, STATE, () => {}).setLastError(0x57);
  assert.equal(t.mem.u32(TEB + 0x68n), 0x57);
  assert.equal(TEB_ADDR, 0x7ffd_1000n);
});

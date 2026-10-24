// kernel32.dll, shim v0 (docs/design/runtime.md, "Shim rules"). Each entry:
// `args` (the Win64 argument count, for traces) and `fn(ctx)` returning the
// value for RAX (undefined for none). Unsupported cases stop with ctx.fail.

import {
  STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE,
  STDIN_HANDLE, STDOUT_HANDLE, STDERR_HANDLE, INVALID_HANDLE_VALUE, ERROR_INVALID_HANDLE,
} from '../abi.mjs';

const hex = (v) => '0x' + BigInt(v).toString(16);

export default {
  // HANDLE GetStdHandle(DWORD nStdHandle)
  GetStdHandle: {
    args: 1,
    fn(ctx) {
      switch (ctx.argI32(0)) {
        case STD_INPUT_HANDLE: return STDIN_HANDLE;
        case STD_OUTPUT_HANDLE: return STDOUT_HANDLE;
        case STD_ERROR_HANDLE: return STDERR_HANDLE;
        default:
          // Documented failure (as in Wine 11 kernelbase): INVALID_HANDLE_VALUE, ERROR_INVALID_HANDLE.
          ctx.setLastError(ERROR_INVALID_HANDLE);
          return INVALID_HANDLE_VALUE;
      }
    },
  },

  // BOOL WriteFile(HANDLE hFile, LPCVOID lpBuffer, DWORD nNumberOfBytesToWrite,
  //                LPDWORD lpNumberOfBytesWritten, LPOVERLAPPED lpOverlapped)
  WriteFile: {
    args: 5,
    fn(ctx) {
      const handle = ctx.arg(0);
      const buffer = ctx.arg(1);
      const count = ctx.arg32(2);
      const written = ctx.arg(3);
      const overlapped = ctx.arg(4);
      if (overlapped !== 0n) ctx.fail(`lpOverlapped ${hex(overlapped)}: overlapped I/O is not supported`);
      // Microsoft: lpNumberOfBytesWritten can be NULL only when lpOverlapped is not NULL.
      if (written === 0n) ctx.fail('lpNumberOfBytesWritten is NULL without lpOverlapped');
      let stream;
      if (handle === STDOUT_HANDLE) stream = 'stdout';
      else if (handle === STDERR_HANDLE) stream = 'stderr';
      else ctx.fail(`hFile ${hex(handle)}: not a writable handle (only the standard output and error handles are)`);
      const bytes = count ? ctx.read(buffer, count) : new Uint8Array(0);
      ctx.write32(written, count);
      ctx.output(stream, bytes);
      return 1n;
    },
  },

  // DECLSPEC_NORETURN void ExitProcess(UINT uExitCode)
  ExitProcess: {
    args: 1,
    fn(ctx) {
      ctx.exit(ctx.arg32(0));
    },
  },
};

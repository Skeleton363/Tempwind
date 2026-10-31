# JS runtime: running PE programs (Phase 1)

Status: **decided for M2.** The runtime in `runtime/` loads a 64-bit PE program, links its AOT translation and the code it translates at run time (`abi.md` v3), and implements the Windows API the program imports in JS, over Node's APIs: the Win32 shim. Phase 1 runs it in Node 24, one thread.

What lives where:
- The loader, linking, the host-call protocol and run-time translation stay for later phases. In Phase 2, Wine's PE side calls into the host where the shim is now.
- The shim exists only for Phase 1: Wine replaces it in Phase 2.

## Pieces

| Piece | Job |
|---|---|
| Translator CLI `translate-pe` | AOT-translates the PE into one v3 module, and writes the load plan (below) from `x64wasm::pe`. JS never parses PE files. |
| Linker | Owns the table, the `table_base`/`lookup`/`depth` globals and the lookup table in guest memory. It links modules: the AOT module, run-time translations, and the module that carries the host-call function. |
| Process | Guest memory and its address map, mapping the image, PEB/TEB/stack/TLS, import thunks, the run loop, exit status, captured stdout and stderr. |
| Shim | `runtime/src/win32/`: one module per DLL, a function per API. |
| Run-time translator | `x64wasm_rt.wasm` (`abi.md`). The runtime calls it when execution reaches code no module has translated. |
| Floating-point helpers | `x64wasm_fp.wasm` (`abi.md`, v4). One instance per process (per thread later); its exports are the `fp` import namespace of every module the Linker links. |
| CLI `run-pe` | Runs a program from the command line, with the guest's stdout, stderr and exit code as its own. |

## Address map of a Phase 1 process

This follows `guest-memory.md` (option A, identity mapping). The constants live in `runtime/src/abi.mjs`.

| Address | Size | Use |
|---|---|---|
| `0x0000_0000` | 64 KiB | NULL region, never mapped. |
| `0x1000_0000` and up | as needed | Guest heap: TLS blocks, process data, and later `HeapAlloc`/`VirtualAlloc` (M3, M5). |
| `0x7FF0_0000 − stack size` up to `0x7FF0_0000` | `stackReserve` (from the image, rounded up to 64 KiB) | Main thread stack, committed whole. |
| `0x7FFD_0000` | 4 KiB | PEB |
| `0x7FFD_1000` | 8 KiB | TEB of the main thread |
| `0x7FFE_0000` | 4 KiB | KUSER_SHARED_DATA, zeroed. Fields are filled when a program needs them. |
| `0x8000_0000` | | **Host region** |
| `0x8000_0000` | 32 bytes | Lookup header (`env.lookup`) |
| `0x8000_1000` | `0x400` | State block of the main thread |
| `0x8001_0000` | 64 KiB | Import thunks: 16 bytes each, 4096 at most |
| `0x8010_0000` and up | as needed | Runtime heap: lookup slots and other runtime data |
| `imageBase` | `sizeOfImage` | The image, at its preferred base (MinGW: `0x1_4000_0000`). |

- **Memory:** one memory64, non-shared in Phase 1. Its initial size is the larger of the image end (rounded up to 64 KiB) and 4 GiB, the end of the host region; for a MinGW image that is a little over 5 GiB. Pages are committed lazily on Linux.
- **Bad image bases:** a load fails with a clear error when the image would overlap the NULL region, the guest heap's start, the stack, PEB/TEB, KUSER or the host region, or end above 16 GiB.
- **No relocation:** images always load at their preferred base. Relocation comes with DLLs (Phase 2).

## Load plan (`translate-pe` → `<stem>.plan.json`)

u64 values are `"0x…"` hex strings, as in the spike manifest; addresses are virtual addresses.

```json
{
  "version": 1,
  "file": "basic.exe",
  "sha256": "…",
  "machine": "0x8664",
  "imageBase": "0x140000000",
  "sizeOfImage": "0x6000",
  "sizeOfHeaders": "0x400",
  "entryPoint": "0x140001000",
  "subsystem": 3,
  "stackReserve": "0x200000",
  "stackCommit": "0x1000",
  "sections": [
    { "name": ".text", "va": "0x140001000", "virtualSize": "0x2a4",
      "fileOffset": "0x400", "fileSize": "0x400", "characteristics": "0x60000020" }
  ],
  "imports": [
    { "dll": "KERNEL32.dll",
      "functions": [ { "name": "ExitProcess", "ordinal": null, "iat": "0x140005070" } ] }
  ],
  "tls": null,
  "hasRelocations": true
}
```

- `tls`, when the image has a TLS directory: `{ "startOfRawData", "endOfRawData", "addressOfIndex", "sizeOfZeroFill", "callbacks": [VA, …] }`, all `"0x…"` strings.
- A section's `virtualSize` is its header's VirtualSize, or SizeOfRawData when that is 0; `fileSize` is SizeOfRawData.
- `hasRelocations` is true when the relocation directory is non-empty and the image isn't marked `IMAGE_FILE_RELOCS_STRIPPED`.
- An import by ordinal has `"name": null` and the ordinal as a number.
- `sha256` is the SHA-256 of the PE file. The runtime refuses a plan whose `sha256` doesn't match the file it was given.

## Loading

1. Check the plan against the file: `sha256`, machine `0x8664`, and the address map rules above.
2. Create the memory. Copy `sizeOfHeaders` bytes of the file to `imageBase`, then each section's `min(fileSize, virtualSize)` bytes from `fileOffset` to `va`. Fresh memory is zero, so bss needs nothing.
3. Initialize the linker:
   - the lookup header at `0x8000_0000`;
   - slots in the runtime heap: a power of two of at least 4096, and at least twice the expected keys;
   - table slot 0 for the host-call function (`HOST_FUNC_INDEX = 0`).
4. Imports:
   - Give every imported function an id and a thunk at `0x8001_0000 + 16 * id`. The thunk's bytes are `0F 0B` (`ud2`) and 14 × `CC`, which is what a guest that reads them sees.
   - Insert the lookup slot (key = thunk address, func 0, entry = id) and write the thunk address into the function's `iat` slot.
   - DLL names compare case-insensitively. Ordinal imports are named `#<ordinal>`.
   - A function the shim lacks still gets a thunk; calling it stops the program (below).
   - A data import (msvcrt's `_fmode`, `_commode`, `__initenv`, …) gets storage instead of a thunk. The shim declares it as `{ data: { size, init } }`, the loader allocates it in the guest heap, and the IAT slot points to it.
5. Link the AOT module (`abi.md`, "Linking a module").
6. PEB, TEB and stack (below). The state block starts zeroed.
7. TLS, if present:
   - allocate a block in the guest heap, copy `[startOfRawData, endOfRawData)` into it and zero-fill `sizeOfZeroFill` bytes;
   - allocate a one-entry pointer array that points to the block, and store it in `TEB.ThreadLocalStoragePointer`;
   - write index 0 (u32) to `addressOfIndex`;
   - call every callback as `callback(imageBase, DLL_PROCESS_ATTACH = 1, 0)` before the entry point.

## PEB, TEB and the main thread

The offsets are those of Windows x64 (`TEB64`, `PEB64`); verify them against Wine 11's `include/winternl.h` before relying on them. Fields not listed stay zero.

| Structure | Offset | Field | Value |
|---|---|---|---|
| TEB | `0x008` | `NtTib.StackBase` | stack top (`0x7FF0_0000`) |
| TEB | `0x010` | `NtTib.StackLimit` | stack top − stack size |
| TEB | `0x030` | `NtTib.Self` | TEB address |
| TEB | `0x040` / `0x048` | `ClientId` (process, thread) | `0x1000` / `0x1004` |
| TEB | `0x058` | `ThreadLocalStoragePointer` | TLS array (if the image has TLS) |
| TEB | `0x060` | `ProcessEnvironmentBlock` | PEB address |
| TEB | `0x068` | `LastErrorValue` | `SetLastError`/`GetLastError` use this field |
| PEB | `0x002` | `BeingDebugged` | 0 |
| PEB | `0x010` | `ImageBaseAddress` | `imageBase` |
| PEB | `0x0B8` | `NumberOfProcessors` | 1 |
| PEB | `0x118` / `0x11C` / `0x120` / `0x124` | OS major / minor / build / platform id | 10 / 0 / 26100 / 2, as on the windows-2025 reference runner |

**Start of the main thread** (and of every host-initiated guest call, such as a TLS callback):
- Let `top` be the 16-byte-aligned top of the free stack. Then `RSP = top − 0x38`, and `[RSP] = SENTINEL_RET`. So RSP ≡ 8 (mod 16), as after a `call`, with 32 bytes of shadow space above the return address.
- For the entry point, `RCX = PEB` (Windows passes it), every other GPR is 0, and `RFLAGS = 0x202`.
- GS base = TEB, FS base = 0.
- The FP state belongs to the thread (M4). It starts as on Windows x64: MXCSR = 0x1F80 (all exceptions masked, round to nearest), FCW = 0x027F, FSW = 0, the tag word 0xFFFF (every x87 register empty), XMM and R0-R7 zero. Host-initiated calls on the thread keep it, as a TLS callback's changes reach the entry point on Windows. (MinGW's CRT startup then runs `fninit`, which sets FCW = 0x037F.)

A guest call ends when the run loop sees `EXIT_LEFT_CODE` at `SENTINEL_RET`. For the entry point, that means the process exits with code `EAX`.

## Host calls

The host-call function sits at table slot 0 (`abi.md`, "Host functions"). It receives `(state, id, ret)` and:
1. Finds the shim function for import `id`. If the shim lacks it, it stops with the error `unimplemented API <dll>!<name>`.
2. Calls it with a context object. Through it, the function reads argument i (Win64: RCX, RDX, R8, R9, then the stack) as a u64, reads and writes guest memory, sets the last error (TEB `0x068`), and can stop the process (`ExitProcess`) or fail with a message.
3. Writes the function's return value (a BigInt or number) into RAX. Nothing is written for a void function.
4. Performs the guest's `ret` and returns the status, exactly as `abi.md` says.

A function that blocks returns a Promise of its value instead; steps 3 and 4 happen when it settles ("Guest threads" below). It can also stop only its thread (`threadExit`).

A stop sets `state.exit = EXIT_HOST` and returns `STATUS_EXIT`, and the runtime records why: an exit code or an error.

**Shim rules:**
- Implement the APIs a test program imports, as Microsoft documents them, for the arguments the program uses.
- A case the shim doesn't handle (for example `WriteFile` with an `OVERLAPPED`, or an unknown handle) is an error stop with the API name and the argument. It never returns a made-up success. Arguments that Microsoft documents as invalid stop too, even where Wine accepts them: a synchronous `WriteFile` needs `lpNumberOfBytesWritten`.
- Shim v0 is what the M2 test program imports from `kernel32.dll`: `GetStdHandle`, `WriteFile`, `ExitProcess`.
  - Standard handles are fixed: stdin `0x3`, stdout `0x7`, stderr `0xB`.
  - `WriteFile` to stdout or stderr appends the bytes to the process's captured output; the CLI also forwards them.

## Calls from the host into guest code

Several shim functions call guest functions: `_initterm`, `exit` (atexit and `_onexit` handlers) and `qsort`. The loader does too, for TLS callbacks. A call from inside a host call:
1. saves the state block and `depth`;
2. builds a call frame below the guest's current RSP, 16-byte aligned:
   - 32 bytes of shadow space, then the stack arguments;
   - `SENTINEL_RET` as the return address;
   - RCX, RDX, R8 and R9 for the first four arguments;
3. runs the run loop until `EXIT_LEFT_CODE` at `SENTINEL_RET`, and takes RAX as the result;
4. restores the state block and `depth`. The interrupted host call then continues as if no guest code had run, except for the callee's effects on memory.

Limits:
- **Stops:** a stop inside the nested call, such as `exit` or an error, stops the whole process.
- **Depth:** nesting is limited to 16 levels, so a runaway chain of callbacks fails loudly.

## msvcrt (M3)

Every value a program can observe follows `msvcrt.dll` on the reference runner (windows-2025). Wine's `dlls/msvcrt` source is the reference for the details, and a difference found later is a bug.
- **FILE:** `__iob_func` returns `_iob`, three 48-byte msvcrt `FILE` structures (stdin, stdout, stderr) in the guest heap, and the stdio functions take those pointers. Output is written through at once.
- **Heap:** `malloc`, `calloc`, `realloc` and `free` use a deterministic allocator in the guest heap, with 16-byte alignment. An invalid `free` stops with an error.
- **errno:** `_errno` returns the address of the thread's errno, and `strerror` returns msvcrt's message texts.
- **Locale:** the C locale. Locale queries (`___lc_codepage_func`, `___mb_cur_max_func`, `localeconv`) return msvcrt's values for it. Conversions between multi-byte and wide strings use code page 1252, the ACP of the reference runner.
- **Startup and exit:**
  - `__getmainargs` builds `argv` from the runtime's command line (the program name unless more is given) and `envp` from an empty environment.
  - `_initterm` calls each non-null entry.
  - `exit` runs the `atexit` and `_onexit` handlers in reverse order, then ends the process with the code.
- **Locks and critical sections:** real owners and waiters since M5 ("Guest threads" below). They still check their arguments.

## Run loop

```text
loop:
  reason = run(state)
  EXIT_LEFT_CODE:
    rip == SENTINEL_RET               -> the guest call returned (entry point: exit code EAX)
    rip in an executable section of the image
                                      -> translate at run time, link, continue
    otherwise                         -> fault: execute at rip (error stop)
  EXIT_UNSUPPORTED -> error stop: "unsupported instruction at 0x… (bytes …)"
  EXIT_HOST        -> exit code or error, as recorded by the host call
  EXIT_FAULT       -> error stop with fault address and kind
```

**Run-time translation:**
1. Build the request (`abi.md`, run-time translator):
   - the code is the executable section that holds `rip`, read from guest memory;
   - `entry = rip`;
   - `known` is every key in the lookup table, sorted. The linker keeps a mirror of the keys, so it doesn't scan the slots.
2. Call `rt_translate`.
3. Link the new module and continue.

If the run-time translator isn't loaded, or reports an error, the process stops with that error.

## Tracing

`TEMPWIND_TRACE` takes a comma-separated list. Traces go to the Node process's stderr, never into the guest's captured output, and each line starts with `[tempwind] `.

| Value | Traces |
|---|---|
| `api` | Each host call: `kernel32.dll!WriteFile(0x7, 0x140003000, 0x1b, 0x7fefff28, 0x0) = 0x1` |
| `translate` | Each run-time translation: address, blocks, wasm bytes, milliseconds |
| `exits` | Each return from `run`: reason and RIP |

## CLI

```text
node runtime/src/run-pe.mjs --exe FILE.exe --aot FILE.wasm --plan FILE.plan.json [--rt x64wasm_rt.wasm] [--fp x64wasm_fp.wasm] [-- ARGS...]
```

- `--fp` (M4) names the floating-point helper module. Without it, `run-pe` uses `target/wasm32-unknown-unknown/release/x64wasm_fp.wasm` under the repository root if that file exists. A module that imports `fp` with neither fails to link, with an error that names `--fp`.
- Arguments after `--` follow the program name in the guest's command line (`__getmainargs`), as CoreMark's seeds and iteration count need.
- `--dir HOST` (M5) loads a host directory into the in-memory file system as `C:\work`, the current directory, and writes it back when the run ends ("Files, handles and the 7-Zip shim (M5)").

- Its stdout and stderr are the guest's, byte for byte, and its exit code is the guest's.
- When the runtime itself stops with an error, it prints `tempwind: <error>` to stderr and exits with 125.

## M2 test program (`tests/pe/`)

`basic.exe` is a `-nostdlib` MinGW program written in assembly. The M1 translator runs only `add`, `sub`, `cmp`, `mov`, `lea`, `push`, `pop`, `call`, `ret`, `jmp`, `jcc` and `nop`, and `testkit census --check` on `basic.exe` proves the program uses no other form.

It exercises:
- output through `GetStdHandle` and `WriteFile`;
- recursion deeper than the call-depth guard;
- indirect calls through a function table;
- a jump table;
- a function that only a computed pointer reaches, which AOT cannot find and which therefore takes the run-time translation path;
- `ExitProcess` with a non-zero code.

On Windows (CI), its stdout and exit code are compared with `tests/pe/basic.expected` and `tests/pe/basic.exitcode`. The translated run in Node must match both, with AOT from all entries and from the entry point only.

## M3 L2 programs (`tests/native/`)

Integer-only C programs, built with MinGW with CRT startup included: `l2_int.c` and `l2_crt.c`.
- Each has a `.expected` stdout and an exit-code file.
- CI runs them natively on Windows and translated in Node.
- `hello.exe` moves to M4: its float output needs SSE2 and x87 arithmetic.

## kernel32 timing (M4)

- `QueryPerformanceFrequency(&f)` stores 10,000,000 (100 ns ticks, as on current Windows) and returns TRUE.
- `QueryPerformanceCounter(&c)` stores a monotonic count of 100 ns ticks since the process started (from `process.hrtime.bigint()` in Node and `performance.now()` in browsers), and returns TRUE.
- A null pointer stops loudly, like every strict shim function.

## M4 programs

- **`hello.exe`** (`tests/native/hello.c`): runs translated, matching `hello.expected` and exit code 0.
- **`l2_float.exe`** (`tests/native/l2_float.c`): deterministic, built with `-O2 -fno-math-errno`. It covers:
  - special values: ±0, denormals, ±∞ and NaN, printed;
  - conversions in every direction, truncating and rounding;
  - compares with NaN;
  - the four rounding modes through `fesetround`;
  - exception flags through `feclearexcept`/`fetestexcept`;
  - FTZ and DAZ through `_mm_setcsr`;
  - loops that GCC vectorizes, over floats and doubles;
  - a `long double` section: arithmetic, `sqrtl`, conversions, compares and printing.

  Its rules: all floating point stays in guest code, so there are no msvcrt math imports, and every import is one the shim already has. It is checked like the M3 programs: natively on Windows in CI, and translated in Node with the exact CRLF bytes and its exit code.
- **CoreMark** (`coremark.exe` from the targets release): a Node test runs it translated with `0x0 0x0 0x66 200` and checks what `tests/targets/smoke.sh` checks: the run type, the four CRCs, and only the "at least 10 secs" error.

## Files, handles and the 7-Zip shim (M5)

**Scope.** The shim implements every import that 7zz.exe executes in the command matrix (`tests/sevenzip/README.md`), by the shim rules above.
- **Sources:**
  - Microsoft's documentation for the contract;
  - Wine's source (the wine-11.0 tarball: `dlls/kernelbase`, `kernel32`, `ntdll`, `advapi32`, `oleaut32`, `user32`, `msvcrt`) for the details;
  - 7-Zip's own source (release `src-7zip-25.01`) for what 7zz actually calls.

  The code is our own: Wine's is read, not copied.
- **Stops loudly:** what the matrix doesn't execute, and these cases:
  - **Exceptions:** `RaiseException`, `RtlCaptureContext`, `RtlLookupFunctionEntry`, `RtlUnwindEx` and `RtlVirtualUnwind`, until Phase 4.
  - **Calls that only optional switches reach:** hard links, NT security and the other `-sn*` features.

**The file system** (`runtime/src/win32/vfs.mjs`) is in memory, with Win32 and NTFS semantics:
- **One volume, `C:`:**
  - NTFS, with a fixed serial number and fixed free space;
  - directories `C:\Windows`, `C:\Windows\System32` and `C:\work`;
  - the program itself, read-only, at `C:\tempwind\<plan file name>`, which `GetModuleFileNameW(NULL)` returns.
- **Names:** UTF-16, case-insensitive and case-preserving, compared by their upper-cased code units. `< > : " | ? *` and control characters are invalid. Win32 path rules apply: `\` and `/`, `.` and `..`, trailing dots and spaces, and `\\?\` prefixes taken literally. Other drives and UNC paths fail with `ERROR_PATH_NOT_FOUND`.
- **Nodes:**
  - **Files:** bytes, attributes, and the creation, last-access and last-write FILETIMEs.
  - **Directories:** their entries. Enumeration (`FindFirstFileW`/`FindNextFileW`, Win32 `*` and `?` matching) follows NTFS order: `.` and `..` first, then names in ordinal order of the upper-cased name.
  - **Attributes:** new files get `FILE_ATTRIBUTE_ARCHIVE` (0x20) and directories `FILE_ATTRIBUTE_DIRECTORY` (0x10). `FILE_ATTRIBUTE_NORMAL` means none.
  - **Times:** a new node gets the current time for all three. A write updates the last-write time, and the last-access time isn't updated.
  - **No hard links, alternate data streams or reparse points:** `FindFirstStreamW` reports only `::$DATA`.
- **Errors:** every failure sets the error Windows sets, for example:
  - `ERROR_FILE_NOT_FOUND`, `ERROR_PATH_NOT_FOUND`, `ERROR_ALREADY_EXISTS`/`ERROR_FILE_EXISTS`;
  - `ERROR_ACCESS_DENIED`, `ERROR_DIR_NOT_EMPTY`, `ERROR_NO_MORE_FILES`, `ERROR_INVALID_NAME`;
  - `ERROR_SHARING_VIOLATION`: share modes are enforced between open handles.
- **`run-pe --dir HOST`** loads the host directory into `C:\work`, which is the current directory.
  - Each file keeps its contents and mtime, which becomes all three times. Files get 0x20, directories 0x10.
  - When the run ends, whatever the outcome, `C:\work` is written back to HOST: new and changed files with their mtimes, new directories, and deletions.
  - Without `--dir`, `C:\work` is empty. msvcrt's file functions use the same file system.

**Handles:**
- **Kinds:** one table holds files, find handles, events, semaphores and file mappings, numbered from `0x100` in steps of 4.
- **Std handles:** the fixed std handles stay `0x3`, `0x7` and `0xB`.
- **Pseudo-handles and modules:**
  - `GetCurrentProcess` is −1;
  - a module handle is the image base for the program, or a fixed pseudo-base for a system DLL that `GetModuleHandleW`/`LoadLibrary*W` name and the reference Windows has;
  - other DLLs fail with `ERROR_MOD_NOT_FOUND`.
- **`GetProcAddress`** returns a thunk, allocated on demand, for a function the shim implements. A function the shim lacks stops loudly with its name, so a run never silently takes a fallback that native Windows wouldn't take.
- **An unknown handle** is an error stop, as before.

**Guest threads (M5): cooperative, on one JS thread.** 7-Zip runs extra threads even with `-mmt1`:
- the coder mixer (`NCoderMixer2::CMixerMT`) for 7z archives with BCJ, BCJ2 or AES, when encoding and decoding;
- the LZMA2 match finder (`MatchFinderMt`) for xz.

`-mtf=off` doesn't help: native 7-Zip fails with it. Real parallel threads (Workers, shared memory) are Phase 4. Until then, guest threads take turns on one JS thread.
- **Mechanism:** JSPI (`WebAssembly.Suspending` and `WebAssembly.promising`; Node 24, Chromium 137+). The code is `runtime/src/threads.mjs` (scheduler, waits, locks) and `process.mjs` (thread start and end).
  - Every guest thread runs `run(state)` through `WebAssembly.promising`.
  - Host calls that can block go through a second host function, wrapped in `WebAssembly.Suspending`, in table slot 1 (modules follow from slot 2): the waits, `EnterCriticalSection`, msvcrt's `_lock`, `Sleep` and `SwitchToThread` (shim entries marked `blocks: true`). When such a call can't complete, it returns a Promise. That suspends the thread's whole wasm stack, and the scheduler resumes another thread.
  - All other host calls stay synchronous: no Promise, no suspension cost.
  - A thread exit (`_endthreadex`, `ExitThread`) isn't a suspension: it is a stop like `ExitProcess` that unwinds the thread's wasm stack, and only that thread ends.
  - V8 throws when a `Suspending` import runs without `promising` on the stack, even if it returns no Promise. So while the host calls into guest code (`callNested`), slot 1 holds the plain host function.
- **Scheduling** is deterministic and non-preemptive. A thread runs until it blocks, yields (`Sleep`, `SwitchToThread`) or ends; the scheduler then resumes the thread that has been runnable longest (a FIFO queue; a yielding thread goes to its end). Signaling an object (`SetEvent`, `ReleaseSemaphore`, `LeaveCriticalSection`, a thread's end) makes waiters runnable, in the order they started waiting, but doesn't switch threads.
- **Threads** come from `_beginthreadex` (and `CreateThread` where a program imports it). Each has:
  - **its own state:** a state block (host region), a TEB (TLS slots, last error, `ClientId`), a stack from the guest heap, the image's TLS data block (a copy of the template), the FP defaults of a new thread, and its own errno (the main thread's is the CRT's).
    - **Stack size**, by Windows' rule (`RtlCreateUserStack`): the requested size is a reservation with `STACK_SIZE_PARAM_IS_A_RESERVATION`, else a commit size; the reservation is that, or the image's stack reserve, at least the commit size and 1 MiB, rounded up to 64 KiB.
    - **Thread ids** are 0x1008, 0x100C, ... (the main thread is 0x1004). A later thread with the same stack size reuses an ended thread's state block, TEB and stack.
  - **a start like the main thread's:** RCX = the argument, RSP = top − 0x38 with `[RSP]` = `SENTINEL_RET`, RIP = the start address. The image's TLS callbacks run first with `DLL_THREAD_ATTACH`, as on Windows.
  - **an end:** returning to the sentinel, `_endthreadex` or `ExitThread`. The return value or the argument becomes its exit code, and the TLS callbacks run with `DLL_THREAD_DETACH`.
  - **`CREATE_SUSPENDED`:** it waits for `ResumeThread`.
  - **its handle:** signaled when the thread ends; `GetExitCodeThread` reads the code (`STILL_ACTIVE`, 259, until then). `GetThreadId`, `ResumeThread` and `SetThreadAffinityMask` take it too.
  - `_beginthreadex` with security attributes, or flags other than `CREATE_SUSPENDED` and `STACK_SIZE_PARAM_IS_A_RESERVATION`, stops. `ExitThread` or `_endthreadex` on the main thread stops (Windows would keep the process until its last thread ends). `CreateThread` waits for a program that imports it.
- **The process ends** when the main thread calls `ExitProcess` or `exit`, or returns from the entry point, whatever the other threads are doing.
- **`depth`** (the call-depth guard) belongs to each thread: it is saved when a thread suspends and restored when it resumes.
- **Synchronization objects** follow Windows:
  - events, manual- or auto-reset;
  - semaphores;
  - critical sections, with an owner, a recursion count and waiters (`TryEnterCriticalSection` fails while another thread owns one). The last `LeaveCriticalSection` hands it to the first waiter at once. The guest sees `OwningThread` (the thread id), `RecursionCount` and `LockCount` (−1 free; held: −2, minus 4 per waiter). msvcrt's `_lock`/`_unlock` locks behave the same;
  - `WaitForSingleObject`, and `WaitForMultipleObjects` with "all" or "any".
  - `OpenEventW` and `OpenFileMappingW` find no named objects (`ERROR_FILE_NOT_FOUND`).
  - `TlsAlloc` and `TlsFree` clear the slot in every thread, as Windows does.
- **Timeouts:**
  - `INFINITE` blocks;
  - 0 polls;
  - a finite timeout blocks until the object is signaled or no other thread can run, and then returns `WAIT_TIMEOUT`, since no real time passes between turns. When several threads wait with finite timeouts, the one that has waited longest times out first.
  - `Sleep(0)` and `SwitchToThread` yield. `Sleep(n)` with n > 0 yields too, and lets time pass: with no other runnable thread, the longest finite wait times out. `Sleep(INFINITE)` waits forever.
- **Deadlock:** when every thread waits with `INFINITE`, the run stops with an error naming each thread's wait, for example `deadlock: no thread can run; thread 0x1004 waits in WaitForSingleObject(0x104, INFINITE); thread 0x1008 waits in EnterCriticalSection(0x10002340), held by thread 0x1004; thread 0x100c is suspended (CREATE_SUSPENDED, never resumed)`. The thread that blocks last, or the thread whose end leaves no thread runnable, finds it.
- **Limit:** blocking inside a call from the host into guest code (`callNested`, for example a `qsort` comparator) is an error stop. Waits that complete at once still work there, and a yield returns without switching.
- **Outputs** don't depend on the interleaving, and the schedule itself is deterministic.

**Console:**
- stdout and stderr are redirected, not consoles. `GetConsoleMode` and `GetConsoleScreenBufferInfo` fail with `ERROR_INVALID_HANDLE`, and `_isatty` returns 0.
- `SetConsoleCtrlHandler` records the handler and returns TRUE; the handler is never called.
- stdin is empty: reads return 0 bytes.

**Time:**
- UTC with no daylight saving: `FileTimeToLocalFileTime` and `LocalFileTimeToFileTime` are the identity.
- `GetSystemTimeAsFileTime` is the host's current time.
- `GetTickCount` counts milliseconds since the process started.
- `FileTimeToSystemTime`, `DosDateTimeToFileTime`, `FileTimeToDosDateTime` and `CompareFileTime` follow Windows exactly, invalid inputs included.
- `GetProcessTimes` reports the process start time and the host's CPU time.

**The machine profile:**
- **`GetSystemInfo`:** AMD64, 1 processor, active mask 1, 4 KiB pages, 64 KiB allocation granularity, and the guest's application address range.
- **Affinity:** `GetProcessAffinityMask` reports 1 and 1. Setting the mask to 1 succeeds; other masks fail with `ERROR_INVALID_PARAMETER`.
- **`GlobalMemoryStatusEx`:** 4 GiB of physical memory, with the guest heap's free bytes available.
- **`IsProcessorFeaturePresent`** agrees with the CPUID profile: SSE and SSE2, CMPXCHG16B and NX are present; SSE3, AVX, RDRAND and the rest are not.
- **`GetLargePageMinimum`** is 0.
- **`GetVersion`** follows Windows' compatibility rule. A program whose manifest doesn't declare Windows 10 sees 6.2.9200; otherwise it sees 10.0.26100, like the PEB.
- **Directories:** `GetWindowsDirectoryW` is `C:\Windows` and `GetSystemDirectoryW` is `C:\Windows\System32`. `GetTempPathW` follows its documented order (TMP, TEMP, USERPROFILE, then the Windows directory), so it returns `C:\Windows\` with the empty environment.
- **Drives:** `GetLogicalDriveStringsW` lists `C:\`, `GetDriveTypeW` reports `DRIVE_FIXED`, and `GetVolumeInformationW` reports NTFS.
- **Command line:** `GetCommandLineW` is the command line Windows would hold. That is the program path, then the arguments, quoted by the rules `CommandLineToArgvW` reverses; 7-Zip parses it itself.

**The rest:**
- **Environment and registry:**
  - `getenv` finds nothing, since the environment is empty;
  - `RegOpenKeyExW` fails with `ERROR_FILE_NOT_FOUND`;
  - registry writes stop loudly.
- **Tokens and privileges** (`OpenProcessToken`, `LookupPrivilegeValueW`, `AdjustTokenPrivileges`) behave as for a normal user. A privilege that isn't held gives `ERROR_NOT_ALL_ASSIGNED`.
- **`SystemFunction036` (RtlGenRandom)** returns random bytes. Matrix outputs must not depend on it.
- **BSTRs** (`SysAllocString`, `SysAllocStringLen`, `SysFreeString`, `SysStringLen`) use Windows' layout: a 4-byte length prefix and a NUL terminator, in the guest heap.
- **`VariantClear` and `VariantCopy`** handle the VARIANT types 7-Zip uses (VT_EMPTY, VT_BOOL, VT_UI4, VT_UI8, VT_BSTR, VT_FILETIME); others stop.
- **user32:** `CharUpperW` takes a string or a single character in the low word. `CharPrevExA` works on code page 1252.
- **Memory:** `VirtualAlloc` and `VirtualFree` reserve and commit in the guest heap with 64 KiB granularity, and `MapViewOfFile` and `UnmapViewOfFile` stop unless the matrix needs them.

**Values the implementation fixes** (M5; the spec above left them open):
- **Volume `C:`:** serial number `0x54454D50`, an empty label, `NTFS` with flags `0x03E702FF` and 255-character components; 256 GiB in all and 128 GiB free, in 4 KiB clusters of 8 sectors of 512 bytes. `GetFileInformationByHandle` reports one link and the node's number as the file index. There are no 8.3 names: `cAlternateFileName` is empty and masks match long names only.
- **Program:** `C:\tempwind\<plan file>` has attributes `0x21` (read-only, archive). The command line starts with that path.
- **Code pages:** conversions take the ANSI code page 1252 and the OEM code page 437 (`CP_OEMCP`; `GetOEMCP` is 437, as on an en-US Windows). 7-Zip writes console text through `CP_OEMCP` after `SetFileApisToOEM`.
- **Modules:** pseudo-bases `0x7FFA_0000_0000` + n × `0x100_0000` for `kernel32`, `kernelbase`, `ntdll`, `advapi32`, `user32`, `oleaut32` and `msvcrt` (loaded at start) and `psapi`, `version` and `shell32` (loaded by `LoadLibrary*W`, also by full path in `C:\Windows\System32`). `GetProcAddress` thunks are allocated after the import thunks, one per function; the functions 7zz asks for in the matrix are `SetDefaultDllDirectories`, `GetActiveProcessorGroupCount` and `GetActiveProcessorCount`.
- **`GetVersion`:** the compatibility rule reads the manifest: Windows 10 values when the image's `.rsrc` holds the Windows 10 `supportedOS` GUID (7zz's does), else 6.2.9200.
- **Memory:** `GetSystemInfo` reports processor level 6, revision 0 (the CPUID profile's family 6, model 0) and the highest application address as the guest memory size − `0x10001`. `GlobalMemoryStatusEx`: total physical memory and page file 4 GiB, available = min(free guest heap, 4 GiB), total virtual `0x7FFF_FFFE_0000`. `VirtualAlloc` also takes `MEM_COMMIT` alone (with no address it reserves too); released regions are reused first-fit and zeroed.
- **`IsProcessorFeaturePresent`:** present means reported by the CPUID profile: 2 (CMPXCHG8B), 3 (MMX), 6 (SSE), 10 (SSE2), 12 (NX), 14 (CMPXCHG16B). PAE (9) is absent, since the profile doesn't report it.
- **`FormatMessageW`:** only `FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS` (with or without `ALLOCATE_BUFFER`), language 0, and the English texts of the errors the shim sets; anything else stops.
- **Write-back:** times reach the host through `utimes` in seconds, exact for whole seconds and to about 0.25 µs otherwise.

**Windows details the implementation follows** (checked against Wine 11 and Microsoft's documentation):
- An open with no read, write or delete access (attributes only) takes no part in share checks.
- `RemoveDirectoryW` on the current directory or an ancestor fails with `ERROR_SHARING_VIOLATION` before the emptiness check.
- `FindFirstFileW` on `file\*` fails with `ERROR_DIRECTORY`; an empty mask (`dir\`) with `ERROR_FILE_NOT_FOUND`; a missing directory with `ERROR_PATH_NOT_FOUND`. Find handles enumerate a snapshot taken by `FindFirstFileW`.
- `SetFileTime` with `0xFFFFFFFF_FFFFFFFF`, or an explicit time, stops later writes through that handle from updating the time; 0 means "unchanged". A new or removed entry updates its directory's last-write time.
- `SetFilePointer` and `GetFileSize` set the last error to 0 when the low part they return is `0xFFFFFFFF`.

**Tests:**
- unit tests per module in `tests/node/`, through `shim-harness.mjs`;
- the translated 7-Zip matrix as the acceptance test.

## Benchmarks (M4)

`tests/bench/coremark.mjs` translates `coremark.exe` (all entries), runs it translated in Node with `0x0 0x0 0x66 0`, and natively under wine64 with the same arguments. Iteration count 0 makes CoreMark calibrate itself to a run of at least 10 seconds, so a valid run prints "Correct operation validated". The script prints one JSON line with:
- the commit and the date;
- Node's version and the CPU model;
- iterations per second, translated and native, each the median of 3 runs;
- their ratio.

A run that isn't validated fails the script.

`.github/workflows/bench.yml` runs it on every push to `main` and on pull requests, and also runs `coremark.exe` natively on windows-2025 for the record.
- **On main,** it appends the JSON line to `coremark.jsonl` on the orphan branch `bench-data`, which the workflow creates on its first run. The commit comes from the workflow's bot, with `contents: write` for this job only.
- **On pull requests,** it compares the ratio with the last line on `bench-data` and emits a warning annotation, never a failure, when it is more than 10 % worse.


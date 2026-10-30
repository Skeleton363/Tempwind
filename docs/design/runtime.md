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

A guest call ends when the run loop sees `EXIT_LEFT_CODE` at `SENTINEL_RET`. For the entry point, that means the process exits with code `EAX`.

## Host calls

The host-call function sits at table slot 0 (`abi.md`, "Host functions"). It receives `(state, id, ret)` and:
1. Finds the shim function for import `id`. If the shim lacks it, it stops with the error `unimplemented API <dll>!<name>`.
2. Calls it with a context object. Through it, the function reads argument i (Win64: RCX, RDX, R8, R9, then the stack) as a u64, reads and writes guest memory, sets the last error (TEB `0x068`), and can stop the process (`ExitProcess`) or fail with a message.
3. Writes the function's return value (a BigInt or number) into RAX. Nothing is written for a void function.
4. Performs the guest's `ret` and returns the status, exactly as `abi.md` says.

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
- **Locks and critical sections:** no-ops with one thread. They still check their arguments.

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
node runtime/src/run-pe.mjs --exe FILE.exe --aot FILE.wasm --plan FILE.plan.json [--rt x64wasm_rt.wasm]
```

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

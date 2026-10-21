# native/

Wine's Unix side (the Unix part of ntdll, win32u, wineserver) built with Emscripten for wasm64. Phase 2 does the real build. Phase 0 has only the spike in `spike/`.

## Emscripten toolchain

`scripts/emcc.sh` runs emcc, or another emsdk tool named as the first argument (`node`, `em++`, `emar`, `wasm-opt`, `bash`, ...), inside `emscripten/emsdk:6.0.12`.

- The repository is mounted at `/src`. The working directory is `/src`, or the matching subdirectory when you call it from inside the repository.
- It runs as your uid:gid, so build outputs belong to you, not root.
- The Emscripten cache persists in `${XDG_CACHE_HOME:-~/.cache}/tempwind/emcache-6.0.12`, or in `EMCC_CACHE_DIR` if set. The image's entrypoint unsets `EM_CACHE`, so the script replaces the entrypoint.
- `EMSDK_IMAGE` overrides the image. `EMCC_DOCKER_ARGS` adds `docker run` arguments.
- It needs only Docker. It works on GitHub's ubuntu-24.04 runners. I tested it with a non-root uid (1001) and an empty cache directory.

```sh
scripts/emcc.sh --version
scripts/emcc.sh -sMEMORY64=1 -pthread foo.c -o out/foo.cjs
scripts/emcc.sh node out/foo.cjs
```

Notes:

- Emscripten 6 marks `-sMEMORY64=1` as deprecated in favour of `-m64`; both produce identical objects. We pass `-Wno-deprecated` so that `-Werror` builds still pass.
- Name Node outputs `.cjs`. The repository `package.json` sets `"type": "module"`, and Emscripten's Node output is CommonJS.
- Host Node 22.22 can't run Emscripten 6 memory64 output. It has V8 12.4, which implements a pre-standard memory64 JS API (`index: 'i64'` with Number sizes, not `address: 'i64'` with BigInt) and an older table64 binary encoding. Converting the API calls in JS is not enough: compilation still fails with `invalid table elements limits flags`. Use the image's Node 24.19, or Node 24+ on the host. Hand-made memory64 modules without table64, like the benchmark's and the translator's, still run on Node 22, provided the JS host creates any imported memory with the old descriptor (`tests/bench/memory/core.js` `newMemory64` shows the fallback).

## Spike (Phase 0, item 6)

```sh
native/spike/run.sh      # ~20 s warm, ~1 min with an empty cache; writes native/spike/out/report.md
```

| File | What |
|---|---|
| `spike/sizes.c` | Prints sizeof/alignof and properties as JSON: integer and pointer types, `off_t`/`time_t`/`clock_t`/`wchar_t`/`wint_t`, floating types with `LDBL_MANT_DIG`, `max_align_t`, `va_list` (and whether it is an array type), pthread/signal/jmp_buf types, `struct stat`/`timespec`/`timeval`/`sockaddr_storage`, `socklen_t`, `fd_set`, endianness, `__int128`, lock-free atomics for 1-16 bytes, arch macros. Also runs 4 pthreads that increment an atomic counter and a mutex-protected counter. |
| `spike/posix.c` | Run-time checks of POSIX behaviour Wine relies on: page size, fault on NULL/PROT_NONE/PROT_READ, `MAP_FIXED`, reserving 1 GiB and 64 GiB, `MAP_SHARED` aliasing, `fork`, `socketpair` + `SCM_RIGHTS`, `pipe`+`poll`, `raise`, `pthread_kill` to a spinning thread, `sigaltstack`, TLS. |
| `spike/features.c`, `spike/probe-features.sh` | Compile/link probes with `-Werror`: `ms_abi`, `__builtin_ms_va_list`, x86 inline and top-level asm, `naked`, TLS, `ucontext_t`, `<linux/futex.h>`, epoll, ptrace, inotify, `SCM_RIGHTS`, dlopen, `__int128`, `getcontext`, `sigaltstack`, setjmp. |
| `spike/compare.mjs` | Diffs the two sides and writes `out/report.md`. Exits 1 if an LP64 core type (int, long, pointer, size_t, ssize_t, ptrdiff_t, intptr_t, off_t, time_t) differs, or if the pthread test fails on either side. |
| `spike/run.sh` | Builds the probes with `gcc` (x86_64-linux-gnu) and with `emcc -sMEMORY64=1 -pthread -sPTHREAD_POOL_SIZE=4 -sEXIT_RUNTIME=1 -sENVIRONMENT=node,worker`. Runs them on the host and with the image's Node, also tries host Node (informational), then compares. |

### Result (Emscripten 6.0.12, gcc 13.3)

The LP64 core matches exactly, and pthreads work under MEMORY64: 4 threads, 400000/400000 on both the atomic and the mutex-protected counter.

| Type | x86_64-linux | wasm64-emscripten | Match |
|---|---|---|---|
| int | 4 / 4 | 4 / 4 | yes |
| long | 8 / 8 | 8 / 8 | yes |
| long long | 8 / 8 | 8 / 8 | yes |
| pointer, function pointer | 8 / 8 | 8 / 8 | yes |
| size_t, ssize_t, ptrdiff_t, intptr_t | 8 / 8 | 8 / 8 | yes |
| off_t, time_t | 8 / 8 | 8 / 8 | yes |
| clock_t | 8 / 8 | **4 / 4** | known |
| wchar_t | 4, signed | 4, signed | yes |
| wint_t | 4, unsigned | 4, **signed** | known |
| long double | 16 / 16, 64-bit mantissa (x87) | 16 / **8**, 113-bit mantissa (binary128, soft-float) | known |
| max_align_t | 32 / 16 | **24 / 8** (`__BIGGEST_ALIGNMENT__` is still 16) | known |
| va_list | 24 / 8, array type | **8 / 8, plain pointer** | known |
| pthread_t / mutex / cond | 8 / 40 / 48 | 8 / 40 / 48 | yes |
| pthread_attr_t | 56 | 88 | known |
| sigset_t / struct sigaction | 128 / 152 | 16 / 40 | known |
| jmp_buf, sigjmp_buf | 200 | 184 | known |
| struct stat | 144 | 104 | known |
| timespec, timeval, sockaddr_storage, fd_set, socklen_t | 16, 16, 128, 128, 4 | same | yes |
| `__int128`, little-endian, `char` signed | yes | yes | yes |
| lock-free atomics 1/2/4/8/16 bytes | yes/yes/yes/yes/**no** | yes/yes/yes/yes/**no** | yes |
| `__x86_64__` / `__wasm64__` / `__LP64__` | 1 / 0 / 1 | **0 / 1** / 1 | known |
| `__linux__` / `__ELF__` / `__unix__` | 1 / 1 / 1 | **0 / 0** / 1 | known |

POSIX behaviour, measured by `posix.c`:

| Check | x86_64-linux | wasm64-emscripten |
|---|---|---|
| `sysconf(_SC_PAGESIZE)` | 4096 | **65536** |
| NULL read faults | SIGSEGV | **succeeds** (address 0 is ordinary memory) |
| write to PROT_NONE / PROT_READ page faults | yes / yes | **no / no** (mprotect returns 0 and does nothing) |
| `mmap` with `MAP_FIXED`, or with an address hint | honoured | **EINVAL** |
| reserve 1 GiB `PROT_NONE` + `MAP_NORESERVE` | 0.00 ms | **0.6-2.2 s**: memory grows and the range is committed and zeroed |
| reserve 64 GiB | ok | **ENOMEM** |
| two `MAP_SHARED` views of one file alias | yes | **no** (each view is a copy) |
| store through `MAP_SHARED` is visible to `pread` | yes | **no** |
| `fork` | works | links, returns **ENOSYS** |
| `socketpair(AF_UNIX)` + `SCM_RIGHTS` | works | **ENOSYS** |
| `pipe` + `poll` | works | works |
| `raise(SIGUSR1)` on the same thread | runs handler | runs handler |
| `pthread_kill(SIGUSR1)` to a spinning thread | handler runs | **never runs** within 1 s |
| `sigaltstack` | works | **ENOSYS** |
| `__thread` | per thread | per thread |

Compile/link probes that fail on wasm64: `ms_abi`, `__builtin_ms_va_list`, x86 inline asm, top-level asm, `naked`, `<linux/futex.h>`, `<sys/ptrace.h>`, `<sys/inotify.h>`, `getcontext`. These compile: `ucontext_t` with `uc_mcontext.gregs` (the header exists, but there is no `getcontext`), epoll headers, `SCM_RIGHTS`, `dlopen` (needs `-sMAIN_MODULE` to do anything), `__int128`, `__builtin_frame_address`/`__builtin_return_address`, setjmp/longjmp. `fork` and `posix_spawn` link only as stubs.

## What Wine's x86-64 Unix side assumes and what wasm64 Emscripten gives

Source: the wine-11.0 tarball from dl.winehq.org (`https://dl.winehq.org/wine/source/11.0/wine-11.0.tar.xz`, sha512 matches `sha512sums.asc`). Note that the `11.x/` directory only has 11.1 and later, plus the 11.0 release candidates. Paths below are relative to the tarball root.

### 1. Data model: fine, if `_WIN64` is defined

- **`LONG`/`ULONG`/`DWORD`**: when `WINE_UNIX_LIB` is defined, `include/winnt.h` (~L474) and `include/minwindef.h` (~L209, L225) make them `int`/`unsigned int` whatever `__LP64__` says. So they are 4 bytes on both targets.
- **`LONG_PTR`/`ULONG_PTR`/`INT_PTR`/`UINT_PTR`**: `include/basetsd.h` makes them `__int64` only under `_WIN64`. **`basetsd.h` L39 defines `_WIN64` only for `__x86_64__`, `__powerpc64__` and `__aarch64__`.** On wasm64, `_WIN64` would be undefined. Then `INT_PTR`/`UINT_PTR`/`HALF_PTR` silently become 32-bit, and `MAXINT_PTR` and friends go wrong, while `LONG_PTR` stays `long` (8 bytes). Phase 2 must add `__wasm64__` there or pass `-D_WIN64`. A `C_ASSERT(sizeof(ULONG_PTR) == 8 && sizeof(INT_PTR) == 8)` in our build would catch a regression.
- **`__int64`** is `long` under `_WIN64`, otherwise `long long` (`basetsd.h` ~L59). It is 8 bytes either way.
- **`WCHAR`** is `unsigned short` in C (`winnt.h` ~L482). The Unix side writes `u""` literals (19 in `dlls/ntdll/unix`) and no `L""`, so the 4-byte `wchar_t` on both targets does not matter.
- **Calling conventions**: with `WINE_UNIX_LIB`, `include/minwindef.h` (L60-86) defines `__stdcall`/`__cdecl` as empty and leaves `__ms_va_list` undefined. So the Unix side uses only the host ABI. That is good, because wasm has no `ms_abi` and no `__builtin_ms_va_list` (probes above).
- **Structures shared with PE code** (TEB, PEB, `CONTEXT`, `EXCEPTION_RECORD`, `KUSER_SHARED_DATA`, unixlib parameter structs) use fixed-size Windows types. They come out identical as long as `_WIN64` is set and the architecture branches pick x86-64 layouts. The catch is that **`winnt.h` picks `CONTEXT` and the `CONTEXT_*` flags under `#ifdef __x86_64__` (~L1420), and likewise `RUNTIME_FUNCTION`/unwind (~L2090), `NtCurrentTeb` (~L2509) and `MemoryBarrier`/`YieldProcessor` (~L7180-7660)**. On wasm64 none of these branches applies, so `CONTEXT` is undefined. We need "guest = AMD64" branches keyed on `__wasm64__`. There are 19 `__x86_64__` conditionals in `winnt.h` and 16 headers with any. `dlls/ntdll/unix/unix_private.h` L44-52 (`current_machine`) needs the same treatment.

### 2. Architecture-specific code in `dlls/ntdll/unix`

- **`signal_x86_64.c`** (3475 lines) is x86-64 Linux machinery and has to be rewritten as a `signal_wasm64.c`, not ported:
  - It has 20 `__ASM_GLOBAL_FUNC`/`__asm__` blocks: `__wine_syscall_dispatcher` (~L3026), `__wine_unix_call_dispatcher` (~L3352), `call_user_mode_callback`/`user_mode_callback_return` (~L1756/L1837), `user_mode_abort_thread`, `signal_start_thread` (~L2985), `alloc_fs_sel`/`modify_ldt`, and `clear_alignment_flag`.
  - It defines `struct syscall_frame` (~L462; `C_ASSERT` size 0x300 with xsave at 0xc0).
  - It uses `%gs`/`%fs` TEB switching through `arch_prctl(ARCH_SET_GS)` (~L2377, L2908).
  - Signal handlers `segv_handler`, `trap_handler`, `fpe_handler`, `int_handler`, `abrt_handler`, `quit_handler`, `usr1_handler` and `sigsys_handler` read and write registers through `ucontext_t` (`RIP_sig`, `RSP_sig`, ...).
  - `setup_raise_exception` builds the x86-64 exception frame and points RIP at `KiUserExceptionDispatcher`.
  - On wasm64 the guest registers live in the translator's CPU state block (`docs/design/abi.md`), not in a ucontext. The replacement must:
    - fill the `syscall_frame` from that block,
    - raise exceptions when the translator reports a fault, and
    - re-enter translated code.
- **Syscall entry**: PE-side thunks come from `__ASM_SYSCALL_FUNC` (`include/wine/asm.h` ~L287). The x86-64 thunk is `mov %rcx,%r10; mov $id,%eax; testb $1,0x7ffe0308; jne; syscall; ret; ...; call *0x7ffe1000`. The translator must recognise `syscall` and `call *0x7ffe1000` and treat them as Unix-side entry. The ids come from `dlls/ntdll/ntsyscalls.h` (258 entries, 0x0000-0x0101) and `dlls/win32u/win32syscalls.h` (1540 entries, 0x1000-0x1603). Both headers choose the 64-bit list with `#ifdef _WIN64`, which is one more reason `_WIN64` must be defined.
- **Typed calls**: Wine's dispatcher calls `KeAddSystemServiceTable` function tables (`ULONG_PTR *funcs`, `dlls/ntdll/unix/loader.c` L143-152) through untyped register-based calls. **Wasm `call_indirect` traps on a signature mismatch**: `NtClose(HANDLE)` is `(i64)->i32`, while `NtWaitForSingleObject(HANDLE, BOOLEAN, const LARGE_INTEGER*)` is `(i64,i32,i64)->i32`. So Phase 2 needs generated per-syscall wrappers that take a uniform `(const ULONG_PTR *args)`. That is 258 ntdll syscalls and 1540 win32u syscalls. By contrast, unixlib entries are uniform (`NTSTATUS (*)(void *args)`, `include/wine/unixlib.h`), and so is `KeUserModeCallback`.
- **unixlib interface**: `__wine_unix_call(handle, code, args)` goes through `__wine_unix_call_dispatcher` (`include/wine/unixlib.h` ~L280). `handle` is the Unix address of the `__wine_unix_call_funcs` table (`dlls/ntdll/unix/virtual.c` ~L839-870, `loader.c` L1640). On wasm64 that table holds wasm function-table indices, so the handle and `args` pointers work unchanged in one shared linear memory. Unix libraries are found with `dlopen`/`dlsym` (`virtual.c` L856, `loader.c` L931). Emscripten dlopen needs `MAIN_MODULE`/`SIDE_MODULE`, so we will probably link the Unix libraries statically and replace `dlopen` with a table.
- **Callbacks into PE code**: `KeUserModeCallback` (`signal_x86_64.c` ~L1905) is used by win32u in about 12 files (window procs, hooks, clipboard, IME). It nests a PE call inside a Unix call, and `NtCallbackReturn` unwinds back. On wasm64 this becomes a nested call into the translator's run loop that returns normally. `user_mode_abort_thread` and unwinding across such nests need Wasm exception handling.
- **Faults inside the Unix side**: `__TRY`/`__EXCEPT` (`include/wine/unixlib.h`) uses `setjmp` plus `ntdll_set_exception_jmp_buf`, which relies on SIGSEGV arriving on a bad guest pointer. On wasm, an in-bounds bad pointer does not fault (see the table above) and an out-of-bounds one traps the whole instance. Unix code must therefore validate guest pointers against the view table (`virtual_check_buffer_for_read/write` already exist).
- **Other inline asm**: `system.c` (cpuid/xgetbv for `get_cpuinfo`, L273 onwards), `thread.c` and `loader.c`. These must report a fixed virtual CPU (the translator's feature set).
- **Page size**: `virtual.c` L172-180 hard-codes `page_shift = 12` and `host_page_size = 0x1000` on x86. The aarch64 path already supports a larger host page. Emscripten reports 65536, but we should not use Emscripten's mmap at all (see §4).

### 3. configure and makedep for a new host CPU

- **`configure.ac` host handling**: `case $host` (L121) handles only `x86_64*`/`arm*`, and `HOST_ARCH` (L179-185) knows only aarch64, arm, i386 and x86_64. A `wasm64-unknown-emscripten` host comes out as `HOST_ARCH=unknown`. That disables `cross_archs=$HOST_ARCH` (L384) and the x86_64-only checks (`-mabi=ms` at L2024-2041, libunwind at L1774, the CFI check at L2081).
- **Defines**: platform defines come from `WINE_CHECK_DEFINE` per host CPU (L2377).
- **PE arch vs Unix arch**: Wine assumes the first PE arch equals the Unix arch, and `tools/makedep.c` L4781 starts `archs` with `HOST_ARCH`. Our mismatch (Unix wasm64, PE x86_64) is new. The closest existing case is WoW64 with i386 PE on an x86_64 host.
- **Build tools**: `tools/tools.h` (`get_default_target`, L548-560; `get_cpu_from_name`) has `#error Unsupported CPU` for unknown CPUs. That does not matter if the tools (winebuild, makedep, widl, wrc) are built natively and passed with `--with-wine-tools`.
- **makedep Unix libraries**: `unix_lib_supported` (`makedep.c` L4778) and `UNIXLDFLAGS = -shared -Wl,-Bsymbolic -Wl,-soname` (`configure.ac` L901) assume ELF shared objects.
- **Recommended plan**: build the native tools and the x86_64 PE DLLs with a stock Wine build in CI. Then build ntdll/win32u/wineserver Unix sources with our own small build (source lists taken from each `Makefile.in` `SOURCES`, filtered to `unix/`) using emcc. That way we don't teach `configure` a new host first. If we change configure later, the patches go in `HOST_ARCH`, `case $host`, and the PE-arch logic.

### 4. mmap, fork/exec, signals, pthreads, futex

These counts come from grepping `dlls/ntdll/unix`, `server` and `dlls/win32u`.

- **Memory** (`virtual.c` only: 28 `mmap`, 28 `mprotect`, 9 `MAP_FIXED`, 10 `munmap`, 3 `madvise`, 5 `MAP_SHARED`):
  - Wine manages the whole address space itself. It reserves ranges with `mmap(MAP_FIXED)`, maps PE images and file views at chosen addresses (`map_file_into_view` ~L2339, with a `pread` fallback at ~L2408 when mmap fails, used only for non-shared views), and implements protection, guard pages and write-watch with `mprotect` plus SIGSEGV (`virtual_handle_fault` ~L4538).
  - Emscripten provides none of this (see the posix table). Phase 2 should give `virtual.c` a backend that treats the wasm memory as the address space: a per-page protection/state table and no host mmap. See `docs/design/guest-memory.md`.
  - Address-space constants that must change: `address_space_limit`/`user_space_limit`/`working_set_limit = 0x7fffffff0000` (L191-193), and the preloader reserve list (`loader/preloader.c` L107-122). The latter covers the DOS area, the low area up to 0x68000000, 0x7f000000-0x7fff0000, and **0x7ffffe000000-0x7fffffff0000 for top-down allocations and Wine's own virtual heap** (`alloc_virtual_heap`, `virtual.c` ~L3600).
  - The server's ASLR DLL range is `free_map_addr(0x600000000000, 0x100000000000)` (`server/mapping.c` L275). All of these lie above 16 GiB.
- **fork/exec**:
  - `process.c` (11 `fork`, e.g. `spawn_process` ~L419-588) and `loader.c` (`posix_spawn` ×6, `execv`) start new processes, the wineserver and the preloader.
  - `server/request.c` ~L860 forks to daemonize.
  - Emscripten `fork` returns ENOSYS. In the browser a new Windows process means a new set of workers and a new wasm memory, which the JS runtime starts.
- **Signals**:
  - `signal_x86_64.c` installs handlers for SIGINT, SIGFPE, SIGQUIT, SIGUSR1, SIGTRAP, SIGSEGV, SIGILL, SIGBUS and SIGSYS (~L2866-2883). `server.c` uses `sigaltstack`.
  - The server sends **SIGUSR1** to suspend a thread or to interrupt it for system APCs (`server/thread.c` L960, L1450, L1467), and **SIGQUIT** to kill threads (L1627). It uses `tgkill` in `server/ptrace.c` and `kill` in `server/request.c`.
  - On wasm: no fault signals arrive, `pthread_kill` to a running thread is not delivered (measured), and `sigaltstack` is ENOSYS. Suspension and APC delivery have to be cooperative: translated code polls a per-thread flag at block boundaries and back-edges, and blocking waits are interruptible.
- **pthreads**:
  - Each Windows thread is a pthread with its own kernel stack: `pthread_attr_setstack` then `pthread_create` (`thread.c` L1446-1450).
  - Mutexes and `pthread_once` are used throughout win32u. `NtCurrentTeb` on the Unix side is `pthread_getspecific(teb_key)` (`thread.c` L1634), which is portable.
  - Emscripten pthreads work (measured), but each thread is a Web Worker. The main browser thread cannot block (`Atomics.wait`), so the whole Unix side must run off the main thread.
- **futex**:
  - `sync.c` L115-135 calls `syscall(__NR_futex, ...)` via `<linux/futex.h>` (Linux only, with a fallback for other systems). It also optionally uses `/dev/ntsync` (`HAVE_LINUX_NTSYNC_H`, L63).
  - Emscripten has no `<linux/futex.h>`. Use the non-Linux path, or map onto `emscripten_futex_wait`/`wake` (Atomics.wait/notify).

### 5. va_list across the PE/Unix boundary

- No syscall, unixlib entry or unixlib parameter struct takes a `va_list`. There are no `va_list` hits in any `dlls/*/unixlib.h`, and no `__ms_va_list` in `dlls/ntdll/unix`, `dlls/win32u` or `server`.
- The Unix side's own `va_list` uses are internal (`fatal_error` and friends).
- Debug output crosses the boundary as a string: `__wine_dbg_write(ptr, len)` (`dlls/ntdll/ntdll.spec` L1758).
- So the different `va_list` (SysV array vs wasm pointer) and the missing `__builtin_ms_va_list` do not matter on the Unix side. Guest varargs are the translator's problem (Win64 ABI: varargs are passed like ordinary args in rcx/rdx/r8/r9 plus the stack).

### 6. What `server/` needs from real processes and sockets

- **IPC**:
  - Clients connect to a Unix-domain master socket. Each request goes over a request pipe, and each reply comes back on a per-thread reply/wait pipe (`dlls/ntdll/unix/server.c`: `send_request` ~L183, `wait_reply` ~L276).
  - **File descriptors are passed with `SCM_RIGHTS`** (`server/request.c`, 4 uses; client `receive_fd`). Every file handle a client uses is a Unix fd the server sent it.
  - Emscripten has no `socketpair`/`SCM_RIGHTS` (ENOSYS, measured), and each Emscripten instance has its own fd table and file system.
  - Phase 2 needs a SharedArrayBuffer request/reply channel, plus a file-system layer shared by the server and all clients so that "passing an fd" can become passing a file identity.
- **Event loop**: `server/fd.c` uses epoll (30 uses), poll and timers. The epoll headers exist in Emscripten, but `poll` on our own channels is what matters.
- **Shared memory**:
  - Anonymous sections are temporary files (`create_temp_file`, `server/mapping.c` ~L362) that clients `mmap(MAP_SHARED)`.
  - The session shared memory (`session_shm_t *shared_session`, `server/mapping.c` L258, `create_session_mapping` ~L1341) is read by win32u (`find_shared_session_object`, `get_shared_desktop` in `input.c` and `sysparams.c`; `shared_session->user_entries` in `window.c`).
  - `KUSER_SHARED_DATA` time fields are written by the server (`set_user_shared_data_time`, `server/fd.c` ~L380) and mapped into every client (`virtual_map_user_shared_data`, `virtual.c` ~L4393).
  - None of this works with Emscripten mmap (no aliasing, measured). In the browser the server must write into each client's memory, or these pages must be replicated.
- **Processes and threads**: `ptrace` (`server/ptrace.c`, 62 uses: attach, debug registers, `read_process_memory`/`write_process_memory` via `PTRACE_PEEKDATA`/`/proc`), `kill`/`tgkill`, `waitpid`, and `/proc` (`procfs.c`). In the browser the server reaches another process's memory through its SharedArrayBuffer, and suspension is cooperative.
- **Others**:
  - `inotify` (`server/change.c`, 46 uses) for change notifications. Emscripten has no inotify, so we will have to poll or hook our FS.
  - `flock`/`fcntl` locks.
  - `init_memory` uses `sysconf(_SC_PAGESIZE)`, which is 65536 under Emscripten.

### Risk list for Phase 2

1. **Address space above 16 GiB.** Wine's x86-64 layout uses 0x7fff_fe00_0000 for its virtual heap and top-down allocations, and 0x6000_0000_0000 for ASLR DLLs. `user_space_limit` is 128 TiB. All of this must move below 16 GiB (`virtual.c` L191-193, `loader/preloader.c` L107-122, `server/mapping.c` L275). This is a design choice, see `docs/design/guest-memory.md`.
2. **No memory protection, faults or aliasing.** Guard pages, stack growth, write-watch, PAGE_NOACCESS, NULL-pointer access violations, and `MAP_SHARED` views (sections, KUSER_SHARED_DATA, session shared memory) all depend on mmap/mprotect/SIGSEGV. The translator and a new `virtual.c` backend must emulate them. This is the biggest piece of new code.
3. **`_WIN64` and the arch branches in the headers.** `basetsd.h` L39 does not define `_WIN64` for wasm64, which silently shrinks `INT_PTR`/`UINT_PTR` to 4 bytes. `winnt.h` defines no `CONTEXT` for wasm64. Patch both, and assert the sizes at build time.
4. **Rewrite `signal_x86_64.c` as `signal_wasm64.c`.** The syscall/unix-call dispatchers, user-mode callbacks, exception raising and thread start are all asm plus ucontext today. This interface is shared with the translator (CPU state block, exit reasons), so it needs an ABI document before coding.
5. **`call_indirect` signature strictness.** Generated typed wrappers are needed for about 1800 ntdll and win32u syscalls. Any other function-pointer casts in Wine that rely on the x86-64 calling convention will trap. `-sEMULATE_FUNCTION_POINTER_CASTS` exists but is slow; find the casts with `-Wcast-function-type`.
6. **wineserver IPC.** Unix sockets, `SCM_RIGHTS` and pipes must become a SharedArrayBuffer channel. All instances need one shared file system so that fds can be "passed" (WasmFS with an OPFS backend, or our own). Cross-process shared memory must be replicated or written by the server.
7. **Signals.** SIGUSR1 suspension and APC interrupts become cooperative polling in translated code and interruptible waits in the Unix side. A guest thread in a tight loop must still reach a poll point.
8. **Processes.** `fork`/`exec`/`posix_spawn` get replaced by a JS-side process launcher: a new memory, new workers, and a server handshake. Each Windows process needs its own copy of the Emscripten Unix-side instance.
9. **Main-thread blocking.** Everything that blocks (server calls, futex, `pthread_join`) must run in workers. The browser main thread can host only the UI/compositor glue.
10. **dlopen.** Unix libraries (`*.so` for ntdll, win32u and the display/audio drivers) are dlopen'ed. Either link them statically into one module with a lookup table, or use Emscripten dynamic linking (`MAIN_MODULE`/`SIDE_MODULE`; check that it supports memory64 before relying on it).
11. **Third-party Unix libraries.** win32u loads FreeType via `dlopen`/`dlsym` (`freetype.c`). Emscripten ports download from GitHub, which sessions can't reach, so FreeType and others must be built in CI from source tarballs.
12. **Toolchain churn.** `-sMEMORY64` is already deprecated (use `-m64`). Host Node 22 can't run Emscripten 6 memory64 output, so CI must use Node 24+ or the image's Node. Pin the emsdk image.
13. **Libc differences that do surface.** `clock_t` is 32-bit, `max_align_t` alignment is 8 (Windows heaps need 16), `long double` is binary128, and `struct stat`/`sigset_t` use different layouts. These are harmless as long as no Wine code shares them with PE code. Review `clock()` users and any `long double` in the Unix side.

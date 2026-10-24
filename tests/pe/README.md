# tests/pe: the M2 test program

`basic.S` is a `-nostdlib` Win64 console program in GNU as (Intel syntax). It imports `GetStdHandle`, `WriteFile` and `ExitProcess` from `kernel32.dll` and uses only `add`, `sub`, `cmp`, `mov`, `lea`, `push`, `pop`, `call`, `ret`, `jcc` and `jmp` forms that have L1 fixtures. `testkit census target/pe/basic.exe --check` proves it.

It exercises:
- `GetStdHandle(-11)` and `WriteFile` output;
- recursion 1000 calls deep (`rec(1000)` = 500500), deeper than the call-depth guard;
- recursive `fib(20)` = 6765;
- indirect calls through a function-pointer table in `.data` (DIR64 relocations);
- a jump table in `.rdata`, dispatched with `jmp qword ptr [rdx+rcx*8]`;
- `hidden`, reached only by `lea rax, [rip+hidden]` + `call rax` (no direct call, export, `.pdata` or relocation), so AOT cannot find it and it takes the run-time translation path;
- decimal printing by repeated subtraction;
- `ExitProcess(42)`.

Exit code: **42**. Expected stdout: `basic.expected` (LF line ends).

- `build.sh` builds `target/pe/basic.exe` (reproducible, with a sha256 file). `basic.ld` is binutils' default PE linker script without the `__CTOR_LIST__`/`__DTOR_LIST__` data it places in `.text`, so `.text` holds only code. The import thunks and nop padding the linker adds are `jmp [rip+x]` and padding.
- `prepare.sh` runs the AOT translations (all entries, entry only) into `target/pe/` and builds the run-time translator module.
- CI: the `windows-x64` job runs `basic.exe` natively and compares stdout byte for byte with `basic.expected` and the exit code with 42. The `linux-x64` job builds it, runs the census check and `prepare.sh`, then the Node tests, which compare the translated runs with `basic.expected` and 42.
- Checked once locally under Wine 9 (`wine64 basic.exe`): same output, exit code 42.

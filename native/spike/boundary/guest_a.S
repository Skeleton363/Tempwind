# Guest code A of the boundary spike, mapped at 0x10000000 (build.sh
# assembles it flat and translates it with x64wasm-cli). Win64 convention.
# The layout constants match boundary.c and driver.mjs.
#
#   IAT      0x18000   IAT[id] = guest address of host function `id` (driver.mjs fills it)
#   RESULTS  0x19000   one 0x40-byte record per thread id: +0 add, +8 block, +0x10 rbx,
#                      +0x18 callback, +0x20 grow
#   CODE_B   0x10100000  guest code B, translated only when the loop reports EXIT_LEFT_CODE
#
# Host ids: 1 add(a, b) = a + b, 2 block(tid), 3 callback(fn, arg), 4 cb_return,
# 5 exit(code), 6 grow(tid), 7 block through a JS wrapper (the negative test).

        .intel_syntax noprefix
        .set IAT, 0x18000
        .set RESULTS, 0x19000
        .set CODE_B, 0x10100000
        .text

# 0x10000000: a green thread's guest entry. rcx = tid.
thread_entry:
        push rbx
        push rsi
        push rdi
        sub rsp, 0x20
        mov rsi, rcx
        imul rbx, rcx, 1000         # must survive the suspension in a GPR
        mov rdi, rcx
        shl rdi, 6
        add rdi, RESULTS
        mov rdx, 40
        mov rax, IAT
        call qword ptr [rax + 8*1]  # add(tid, 40): C, does not block
        mov [rdi], rax
        mov rcx, rsi
        mov rax, IAT
        call qword ptr [rax + 8*2]  # block(tid): C suspends through green_block
        mov [rdi + 8], rax
        mov [rdi + 0x10], rbx
        mov rcx, rsi
        mov rax, CODE_B
        jmp rax                     # not translated yet: EXIT_LEFT_CODE, C calls translate

# 0x10000100: the guest callback of the nested call. rcx = arg; returns 3 * arg + 1.
        .org 0x100
callback:
        sub rsp, 0x28
        lea rcx, [rcx + rcx*2]
        mov rdx, 1
        mov rax, IAT
        call qword ptr [rax + 8*1]  # add(3 * arg, 1), a host call inside the nested loop
        add rsp, 0x28
        ret                         # to the cb_return address C pushed

# 0x10000200: memory growth. rcx = tid.
        .org 0x200
grow_entry:
        sub rsp, 0x28
        mov rdi, rcx
        shl rdi, 6
        add rdi, RESULTS
        mov rax, IAT
        call qword ptr [rax + 8*6]  # grow(tid): rax = an address past the old end of memory
        mov rax, [rax]
        mov [rdi + 0x20], rax
        mov rcx, 0x60
        mov rax, IAT
        call qword ptr [rax + 8*5]  # exit(0x60)
        ud2

# 0x10000300: the negative test: block through a JS function in env.table.
        .org 0x300
neg_entry:
        sub rsp, 0x28
        mov rax, IAT
        call qword ptr [rax + 8*7]
        mov rcx, 0x70
        mov rax, IAT
        call qword ptr [rax + 8*5]  # exit(0x70)
        ud2

/* M2 test program: a -nostdlib Win64 console program in GNU as (Intel syntax).
   Uses only add, sub, cmp, mov, lea, push, pop, call, ret, jcc and jmp.
   See README.md. */
        .intel_syntax noprefix

/* say "text": writes the string to standard output. Clobbers rcx, rdx, rax, r8-r11. */
        .macro say str
        .section .rdata
.Lstr\@:
        .ascii "\str"
        .set .Llen\@, . - .Lstr\@
        .text
        lea     rcx, [rip + .Lstr\@]
        mov     rdx, offset .Llen\@
        call    print
        .endm

        .section .rdata
pow10:
        .quad   1000000000000, 100000000000, 10000000000, 1000000000
        .quad   100000000, 10000000, 1000000, 100000, 10000, 1000, 100, 10, 1
/* Jump table: absolute code addresses (DIR64 relocations). */
jtab:
        .quad   case0, case1, case2, case3

        .data
/* Function-pointer table: absolute code addresses (DIR64 relocations). */
functab:
        .quad   f_add1, f_dbl, f_triple

        .bss
numbuf:
        .skip   16

        .text
        .globl  start
start:
        sub     rsp, 40
        say     "basic.exe start\n"

        say     "rec(1000) = "
        mov     rcx, 1000
        call    rec
        mov     rcx, rax
        call    printnum

        say     "fib(20) = "
        mov     rcx, 20
        call    fib
        mov     rcx, rax
        call    printnum

        lea     rsi, [rip + functab]
        mov     rbx, 0
.Ltloop:
        say     "indirect: "
        mov     rcx, 7
        call    qword ptr [rsi + rbx*8]
        mov     rcx, rax
        call    printnum
        add     rbx, 1
        cmp     rbx, 3
        jb      .Ltloop

        mov     rbx, 0
.Ljloop:
        say     "jump: "
        mov     rcx, rbx
        call    dispatch
        mov     rcx, rax
        call    printnum
        add     rbx, 1
        cmp     rbx, 4
        jb      .Ljloop

        say     "hidden: "
        lea     rax, [rip + hidden]
        call    rax
        mov     rcx, rax
        call    printnum

        say     "done\n"
        mov     rcx, 42
        call    qword ptr [rip + __imp_ExitProcess]

/* print(rcx = pointer, rdx = length): writes to standard output. */
print:
        push    rbx
        push    rsi
        push    rdi
        sub     rsp, 48
        mov     rsi, rcx
        mov     rdi, rdx
        mov     rcx, -11
        call    qword ptr [rip + __imp_GetStdHandle]
        mov     rcx, rax
        mov     rdx, rsi
        mov     r8, rdi
        lea     r9, [rsp + 40]
        mov     qword ptr [rsp + 32], 0
        call    qword ptr [rip + __imp_WriteFile]
        add     rsp, 48
        pop     rdi
        pop     rsi
        pop     rbx
        ret

/* printnum(rcx = value): prints the decimal value and a newline. Each digit is
   found by subtracting powers of ten. */
printnum:
        push    rbx
        push    rsi
        push    rdi
        sub     rsp, 32
        lea     rsi, [rip + pow10]
        lea     rdi, [rip + numbuf]
        mov     rbx, rcx
.Ldigit:
        mov     rax, qword ptr [rsi]
        mov     rdx, 48
.Lsub:
        cmp     rbx, rax
        jb      .Lemit
        sub     rbx, rax
        add     rdx, 1
        jmp     .Lsub
.Lemit:
        mov     byte ptr [rdi], dl
        add     rdi, 1
        add     rsi, 8
        cmp     rax, 1
        jne     .Ldigit
        mov     byte ptr [rdi], 10
        lea     rcx, [rip + numbuf]
        mov     rdx, 14
.Lskip:
        cmp     byte ptr [rcx], 48
        jne     .Lprint
        cmp     rdx, 2
        je      .Lprint
        add     rcx, 1
        sub     rdx, 1
        jmp     .Lskip
.Lprint:
        call    print
        add     rsp, 32
        pop     rdi
        pop     rsi
        pop     rbx
        ret

/* rec(rcx = n) = n + rec(n - 1), rec(0) = 0: n nested calls. */
rec:
        cmp     rcx, 0
        je      .Lrec0
        push    rcx
        sub     rcx, 1
        call    rec
        pop     rcx
        add     rax, rcx
        ret
.Lrec0:
        mov     rax, 0
        ret

/* fib(rcx = n), recursive. */
fib:
        cmp     rcx, 2
        jb      .Lfibsmall
        push    rbx
        push    rcx
        sub     rcx, 1
        call    fib
        mov     rbx, rax
        pop     rcx
        sub     rcx, 2
        call    fib
        add     rax, rbx
        pop     rbx
        ret
.Lfibsmall:
        mov     rax, rcx
        ret

/* Targets of the function-pointer table. */
f_add1:
        lea     rax, [rcx + 1]
        ret
f_dbl:
        lea     rax, [rcx + rcx]
        ret
f_triple:
        lea     rax, [rcx + rcx*2]
        ret

/* dispatch(rcx = index 0..3): tail-jumps through the table in .rdata. */
dispatch:
        lea     rdx, [rip + jtab]
        jmp     qword ptr [rdx + rcx*8]
case0:
        mov     rax, 11
        ret
case1:
        mov     rax, 222
        ret
case2:
        mov     rax, 3333
        ret
case3:
        mov     rax, 44444
        ret

/* Reached only through lea + call: no direct call, no export, no relocation. */
hidden:
        mov     rax, 4242
        ret

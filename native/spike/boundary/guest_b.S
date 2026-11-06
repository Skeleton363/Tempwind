# Guest code B of the boundary spike, mapped at 0x10100000. It is a separate
# translated module that the driver links only when C's run loop reports
# EXIT_LEFT_CODE for this address (guest_a.S jumps here).
# On entry (from thread_entry): rsi = tid, rdi = the thread's result record.

        .intel_syntax noprefix
        .set IAT, 0x18000
        .set CALLBACK, 0x10000100
        .text

b_entry:
        mov rcx, CALLBACK
        lea rdx, [rsi + 5]
        mov rax, IAT
        call qword ptr [rax + 8*3]  # callback(CALLBACK, tid + 5): C runs its loop on it
        mov [rdi + 0x18], rax
        lea rcx, [rsi + 0x50]
        mov rax, IAT
        call qword ptr [rax + 8*5]  # exit(tid + 0x50)
        ud2

/* L2 "translated vs native" program: structured exception handling.
 * Each case raises one exception (a software raise or a CPU fault) under an
 * assembler helper that has its own .seh_proc and an @except or @unwind handler,
 * and prints the record the handler saw. The handler continues, or unwinds to
 * the helper with RtlUnwind. The last case is unhandled: a top-level filter
 * returns EXCEPTION_EXECUTE_HANDLER, so the exit code is the raised code, 42.
 * It prints only what Windows and Wine agree on: codes, flags, parameter counts
 * and parameters that are not addresses. Addresses are compared in the program
 * (with a label or the target) and only the comparison is printed. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <windows.h>

enum { MODE_CONTINUE, MODE_UNWIND };

/* What the handler saw; the case prints it. */
static struct {
    DWORD code, flags, nparams;
    ULONG_PTR params[EXCEPTION_MAXIMUM_PARAMETERS];
    ULONG_PTR addr;
} g_rec;
static volatile int g_hits;      /* @except calls that saw the exception */
static volatile int g_mode;
static volatile int g_term_ran;  /* @unwind calls that saw the unwind */
static void *volatile g_recover; /* where the handler unwinds to */
static volatile ULONG_PTR g_expect_addr; /* label of the faulting instruction */

/* A code address for the "store into .text" case. */
static __attribute__((noinline)) void sentinel(void) { __asm__ volatile("nop"); }

EXCEPTION_DISPOSITION seh_except_handler(EXCEPTION_RECORD *rec, void *frame, CONTEXT *ctx, void *dispatch) {
    (void)dispatch;
    if (rec->ExceptionFlags & (EXCEPTION_UNWINDING | EXCEPTION_EXIT_UNWIND)) return ExceptionContinueSearch;
    g_hits++;
    g_rec.code = rec->ExceptionCode;
    /* Windows sets 0x80 (software originated) on RaiseException's records; Wine doesn't. Not compared. */
    g_rec.flags = rec->ExceptionFlags & ~0x80u;
    g_rec.nparams = rec->NumberParameters;
    memcpy(g_rec.params, rec->ExceptionInformation, sizeof g_rec.params);
    g_rec.addr = (ULONG_PTR)rec->ExceptionAddress;
    if (g_mode == MODE_CONTINUE) {
        /* int3: resume after it, whichever side of it Rip is. */
        if (rec->ExceptionCode == EXCEPTION_BREAKPOINT && ctx->Rip == (DWORD64)rec->ExceptionAddress) ctx->Rip++;
        return ExceptionContinueExecution;
    }
    RtlUnwind(frame, g_recover, rec, (PVOID)1);
    return ExceptionContinueSearch;
}

EXCEPTION_DISPOSITION seh_term_handler(EXCEPTION_RECORD *rec, void *frame, CONTEXT *ctx, void *dispatch) {
    (void)frame; (void)ctx; (void)dispatch;
    if (rec->ExceptionFlags & (EXCEPTION_UNWINDING | EXCEPTION_EXIT_UNWIND)) g_term_ran++;
    return ExceptionContinueSearch;
}

/* int guarded_call(void (*fn)(void)): fn under the @except handler; 1 if it unwound here, else 0. */
extern char guarded_recover[], deep_recover[];
int guarded_call(void (*fn)(void));
/* void term_call(void (*fn)(void)): fn under the @unwind handler. */
void term_call(void (*fn)(void));
/* int deep_call(void (*fn)(void)): sets the nonvolatile registers, calls fn under the @except handler,
 * and after the unwind returns a mask of the registers that did not come back as set (0 = all restored). */
int deep_call(void (*fn)(void));

__asm__(
    ".text\n"
    ".globl guarded_call\n"
    ".seh_proc guarded_call\n"
    "guarded_call:\n"
    "    subq $40, %rsp\n"
    "    .seh_stackalloc 40\n"
    "    .seh_handler seh_except_handler, @except\n"
    "    .seh_endprologue\n"
    "    call *%rcx\n"
    "    xorl %eax, %eax\n"
    ".globl guarded_recover\n"
    "guarded_recover:\n"
    "    addq $40, %rsp\n"
    "    ret\n"
    "    .seh_endproc\n"

    ".globl term_call\n"
    ".seh_proc term_call\n"
    "term_call:\n"
    "    subq $40, %rsp\n"
    "    .seh_stackalloc 40\n"
    "    .seh_handler seh_term_handler, @unwind\n"
    "    .seh_endprologue\n"
    "    call *%rcx\n"
    "    nop\n" /* a call must not end right before the epilogue, or the unwinder reads the return address as inside it */
    "    addq $40, %rsp\n"
    "    ret\n"
    "    .seh_endproc\n"

    ".globl deep_call\n"
    ".seh_proc deep_call\n"
    "deep_call:\n"
    "    pushq %rbx\n    .seh_pushreg %rbx\n"
    "    pushq %rsi\n    .seh_pushreg %rsi\n"
    "    pushq %rdi\n    .seh_pushreg %rdi\n"
    "    pushq %r12\n    .seh_pushreg %r12\n"
    "    pushq %r13\n    .seh_pushreg %r13\n"
    "    pushq %r14\n    .seh_pushreg %r14\n"
    "    pushq %r15\n    .seh_pushreg %r15\n"
    "    subq $32, %rsp\n"
    "    .seh_stackalloc 32\n"
    "    .seh_handler seh_except_handler, @except\n"
    "    .seh_endprologue\n"
    "    movabsq $0x1111111111111111, %rbx\n"
    "    movabsq $0x2222222222222222, %rsi\n"
    "    movabsq $0x3333333333333333, %rdi\n"
    "    movabsq $0x4444444444444444, %r12\n"
    "    movabsq $0x5555555555555555, %r13\n"
    "    movabsq $0x6666666666666666, %r14\n"
    "    movabsq $0x7777777777777777, %r15\n"
    "    call *%rcx\n"
    ".globl deep_recover\n"
    "deep_recover:\n"
    "    xorl %eax, %eax\n"
    "    movabsq $0x1111111111111111, %rdx\n    cmpq %rdx, %rbx\n    je 1f\n    orl $1, %eax\n1:\n"
    "    movabsq $0x2222222222222222, %rdx\n    cmpq %rdx, %rsi\n    je 2f\n    orl $2, %eax\n2:\n"
    "    movabsq $0x3333333333333333, %rdx\n    cmpq %rdx, %rdi\n    je 3f\n    orl $4, %eax\n3:\n"
    "    movabsq $0x4444444444444444, %rdx\n    cmpq %rdx, %r12\n    je 4f\n    orl $8, %eax\n4:\n"
    "    movabsq $0x5555555555555555, %rdx\n    cmpq %rdx, %r13\n    je 5f\n    orl $16, %eax\n5:\n"
    "    movabsq $0x6666666666666666, %rdx\n    cmpq %rdx, %r14\n    je 6f\n    orl $32, %eax\n6:\n"
    "    movabsq $0x7777777777777777, %rdx\n    cmpq %rdx, %r15\n    je 7f\n    orl $64, %eax\n7:\n"
    "    addq $32, %rsp\n"
    "    popq %r15\n    popq %r14\n    popq %r13\n    popq %r12\n    popq %rdi\n    popq %rsi\n    popq %rbx\n"
    "    ret\n"
    "    .seh_endproc\n");

/* Print what the handler saw. addr_params is a bit set of the parameters that are addresses:
 * those are compared with `expect_p` and not printed. check_addr compares the record's
 * address with the label the case set in g_expect_addr. */
static void report(const char *name, unsigned addr_params, ULONG_PTR expect_p, int check_addr, int unwound) {
    printf("%s: hits=%d unwound=%d code=%08lx flags=%lx nparams=%lu", name, g_hits, unwound,
           (unsigned long)g_rec.code, (unsigned long)g_rec.flags, (unsigned long)g_rec.nparams);
    for (DWORD i = 0; i < g_rec.nparams; i++) {
        if (addr_params & (1u << i)) printf(" p%lu==expected:%d", (unsigned long)i, g_rec.params[i] == expect_p);
        else printf(" p%lu=%llx", (unsigned long)i, (unsigned long long)g_rec.params[i]);
    }
    if (check_addr) printf(" addr==label:%d", g_rec.addr == g_expect_addr);
    printf("\n");
}

static void begin(int mode, void *recover) {
    g_hits = 0;
    g_mode = mode;
    g_recover = recover;
    g_expect_addr = 0;
    memset(&g_rec, 0, sizeof g_rec);
}

/* Store the address of the instruction that follows into g_expect_addr (operand %0). */
#define MARK "leaq 1f(%%rip), %%rax\n movq %%rax, %0\n 1: "

static void raise_params(void) {
    ULONG_PTR args[2] = {0x11, 0x22};
    RaiseException(0xE0000001, 0, 2, args);
}
static void null_read(void) {
    volatile int *p = NULL;
    __asm__ volatile(MARK "movl (%1), %%eax" : "=m"(g_expect_addr) : "r"(p) : "rax");
}
static void null_write(void) {
    volatile int *p = NULL;
    __asm__ volatile(MARK "movl $1, (%1)" : "=m"(g_expect_addr) : "r"(p) : "rax", "memory");
}
static void call_null(void) {
    g_expect_addr = 0;
    ((void (*)(void))(uintptr_t)0)();
}
static void divide_zero(void) {
    __asm__ volatile("movl $1, %%eax\n xorl %%ecx, %%ecx\n cltd\n" MARK "idivl %%ecx"
                     : "=m"(g_expect_addr) : : "rax", "rcx", "rdx", "cc");
}
static void undefined(void) {
    __asm__ volatile(MARK "ud2" : "=m"(g_expect_addr) : : "rax");
}
static void breakpoint(void) {
    __asm__ volatile(MARK "int3" : "=m"(g_expect_addr) : : "rax");
}
static void store_text(void) {
    volatile unsigned char *p = (volatile unsigned char *)(void *)sentinel;
    __asm__ volatile(MARK "movb $0x90, (%1)" : "=m"(g_expect_addr) : "r"(p) : "rax", "memory");
}

/* Frames below deep_call: lvl1 -> term_call -> lvl3, which clobbers the nonvolatile registers and faults. */
static __attribute__((noinline)) void lvl3(void) {
    __asm__ volatile("movabsq $0xdeadbeef, %%rbx\n movabsq $0xdeadbeef, %%rsi\n movabsq $0xdeadbeef, %%rdi\n"
                     "movabsq $0xdeadbeef, %%r12\n movabsq $0xdeadbeef, %%r13\n movabsq $0xdeadbeef, %%r14\n"
                     "movabsq $0xdeadbeef, %%r15\n"
                     "xorl %%eax, %%eax\n"
                     "movl (%%rax), %%eax"
                     : : : "rax", "rbx", "rsi", "rdi", "r12", "r13", "r14", "r15", "memory");
}
static __attribute__((noinline)) void lvl1(void) {
    term_call(lvl3);
    __asm__ volatile("" ::: "memory"); /* keep this a frame, not a tail call */
}

static LONG WINAPI top_filter(EXCEPTION_POINTERS *ep) {
    printf("top filter: code=%08lx flags=%lx nparams=%lu\n", (unsigned long)ep->ExceptionRecord->ExceptionCode,
           (unsigned long)(ep->ExceptionRecord->ExceptionFlags & ~0x80u), (unsigned long)ep->ExceptionRecord->NumberParameters);
    return EXCEPTION_EXECUTE_HANDLER;
}

int main(void) {
    setvbuf(stdout, NULL, _IONBF, 0);
    int u;

    begin(MODE_CONTINUE, NULL);
    u = guarded_call(raise_params);
    report("raise", 0, 0, 0, u);
    printf("raise: continued\n");

    begin(MODE_UNWIND, guarded_recover);
    u = guarded_call(null_read);
    report("null read", 2, 0, 1, u);

    begin(MODE_UNWIND, guarded_recover);
    u = guarded_call(null_write);
    report("null write", 2, 0, 1, u);

    begin(MODE_UNWIND, guarded_recover);
    u = guarded_call(call_null);
    report("call null", 2, 0, 1, u);

    begin(MODE_UNWIND, guarded_recover);
    u = guarded_call(divide_zero);
    report("divide by zero", 0, 0, 1, u);

    begin(MODE_UNWIND, guarded_recover);
    u = guarded_call(undefined);
    report("ud2", 0, 0, 1, u);

    begin(MODE_CONTINUE, NULL);
    u = guarded_call(breakpoint);
    report("int3", 0, 0, 1, u);
    printf("int3: continued\n");

    begin(MODE_UNWIND, guarded_recover);
    u = guarded_call(store_text);
    report("store into text", 2, (ULONG_PTR)sentinel, 1, u);

    begin(MODE_UNWIND, deep_recover);
    g_term_ran = 0;
    int missed = deep_call(lvl1);
    report("deep fault", 2, 0, 0, 1);
    printf("deep fault: registers restored=%d termination handler ran=%d\n", missed == 0, g_term_ran);

    SetUnhandledExceptionFilter(top_filter);
    printf("raising unhandled 0x2a\n");
    RaiseException(0x2A, 0, 0, NULL);
    printf("not reached\n");
    return 1;
}

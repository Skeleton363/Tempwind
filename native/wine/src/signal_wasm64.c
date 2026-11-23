/*
 * The machine layer of the wasm64 Unix side, in place of signal_x86_64.c
 * (docs/design/wine-unix.md, sections 1, 8 and 11; abi.md v5, "Wine
 * boundary").
 *
 * - The process and thread setup of section 8, and the start state of the
 *   main thread: BOOT_STOP_AT_ENTRY returns to boot, run mode enters the run
 *   loop (run.c). From M4 every thread's: its frame record
 *   (signal_alloc_thread), its state block and run loop
 *   (signal_start_thread), both freed by signal_free_thread (section 9).
 * - Signal handlers (M4, section 9): SIGQUIT's, as quit_handler's in-syscall
 *   path; pending signals are delivered before a dispatcher returns to guest
 *   code (delivery point 3).
 * - The syscall and unix-call dispatchers (section 11; abi.md v5, "Syscalls
 *   and unix calls"): host functions, raw exports of the guest-function type
 *   (state i64, entry i32, ret i64) -> i32. The frame record is the state
 *   block; the thread data keeps the rest of Wine's struct syscall_frame.
 * - The context functions for the current thread, with the FP conversion
 *   that agrees with the translator's fxsave (translator.md, "fxsave m512").
 *
 * The rest stops loudly until M4-M5 own it (other threads, APCs, exceptions,
 * user callbacks). NtCurrentTeb() needs no entry here: thread.c's reads
 * teb_key on every machine. Line numbers are wine-11.0's
 * dlls/ntdll/unix/signal_x86_64.c unless another file is named.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include "config.h"

#include <pthread.h>
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winternl.h"
#include "wine/unixlib.h"
#include "unix_private.h"

#include "tempwind/abi.h"
#include "tempwind_private.h"

/* The x86-64 user selectors (section 3). The thread's FS selector is 0: no
 * WoW64. */
static const WORD cs64_sel = 0x33;
static const WORD ds64_sel = 0x2b;

/* The main thread's CPU state block (abi.md, "CPU state block"), in the host
 * heap; signal_start_thread fills it. */
static BYTE *main_state;

/* The server's clock (server/fd.c L418-426): writes KUSER's time fields. */
extern void set_current_time( void );

/* loader.c L636-664: Wine's +syscall trace lines. */
extern void trace_syscall( UINT id, ULONG_PTR *args, ULONG len );
extern void trace_sysret( UINT id, ULONG_PTR retval );

/* The generated wrappers by number (DIR/gen/syscall_table.c, section 11). */
typedef ULONG_PTR (*tw_syscall_fn)( const ULONG_PTR *args );
extern const tw_syscall_fn tw_syscall_table[];
extern const BYTE tw_syscall_args[];
extern const unsigned int tw_syscall_count;

/* loader.c's unix_call_funcs, the table __wine_unixlib_handle points to
 * (patch 0301). */
extern const unixlib_entry_t *const tw_unix_call_funcs;
extern const unsigned int tw_unix_call_count;

/* The rest of Wine's struct syscall_frame (L462-495), per thread, in the
 * host heap: the registers are the state block's. ntdll_thread_data's
 * syscall_frame points to it. */
struct syscall_frame
{
    BYTE   *state;          /* the thread's state block, set at each entry */
    BYTE   *own_state;      /* M4: the one signal_start_thread allocated, freed with the frame */
    UINT64  ret_addr;       /* the return address the dispatcher popped */
    DWORD   syscall_id;     /* L3060 */
    DWORD   restore_flags;  /* L3036, L3359 */
};

#define RESTORE_FLAGS_INCOMPLETE_FRAME_CONTEXT 0x00020000  /* L460 */

/* The state block's fields (abi.md, "CPU state block"). */
#define S64(s, off) (*(UINT64 *)((s) + (off)))
#define S32(s, off) (*(UINT32 *)((s) + (off)))
#define S16(s, off) (*(UINT16 *)((s) + (off)))
#define GPR(s, i)   S64( s, TW_STATE_GPR(i) )
enum { RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI, R8, R9, R10, R11, R12, R13, R14, R15 };

/* ---- process and thread setup (section 8) ---- */

/* signal_x86_64.c reads the host's selectors here; ours are constants. */
void signal_init_threading( void )
{
}

/* No signal handlers: nothing faults, and the server's signals are handled
 * in-process (section 7). As signal_x86_64.c does (L2841-2846): the main
 * thread's frame record, and the page at 0x7ffe1000, which holds the syscall
 * dispatcher's address that every PE syscall thunk calls through. */
void signal_init_process( void )
{
    struct ntdll_thread_data *thread_data = ntdll_get_thread_data();
    void *ptr = (char *)user_shared_data + page_size;

    if (!thread_data->syscall_frame &&
        !(thread_data->syscall_frame = calloc( 1, sizeof(struct syscall_frame) )))
        tw_unsupported( "signal_init_process: no host memory for the frame record" );
    if (anon_mmap_fixed( ptr, page_size, PROT_READ | PROT_WRITE, 0 ) != ptr)
        tw_unsupported( "signal_init_process: committing the page at 0x7ffe1000" );
    *(UINT64 *)ptr = TW_SYSCALL_DISPATCHER;
}

/* A new thread's frame record (M4, section 9), from virtual_alloc_teb
 * (unix/virtual.c L4107), in the TEB's thread data; the main thread's TEB
 * comes from virtual_alloc_first_teb, which doesn't call this, so
 * signal_init_process allocates its frame record. No LDT entry to allocate:
 * no WoW64 (section 3). */
NTSTATUS signal_alloc_thread( TEB *teb )
{
    struct ntdll_thread_data *thread_data = (struct ntdll_thread_data *)&teb->GdiTebBatch;

    if (teb->WowTebOffset) tw_unsupported( "signal_alloc_thread: a WoW64 thread" );
    if (!(thread_data->syscall_frame = calloc( 1, sizeof(struct syscall_frame) ))) return STATUS_NO_MEMORY;
    return STATUS_SUCCESS;
}

/* From virtual_free_teb (unix/virtual.c L4152): the frame record and the
 * state block. Wine frees a thread's TEB only after it has ended: the next
 * exiting thread joins it first (unix/thread.c L1503-1510). */
void signal_free_thread( TEB *teb )
{
    struct ntdll_thread_data *thread_data = (struct ntdll_thread_data *)&teb->GdiTebBatch;
    struct syscall_frame *frame = thread_data->syscall_frame;

    if (!frame) return;
    free( frame->own_state );
    free( frame );
    thread_data->syscall_frame = NULL;
}

/* SIGQUIT's handler, quit_handler's path inside a syscall (L2608-2614): our
 * threads are always inside one when a signal is delivered (section 9). The
 * other signals' handlers are WP4's; until then nothing sends them (patch
 * 0201's send_thread_signal stops loudly first). */
void tw_handle_signal( int sig )
{
    switch (sig)
    {
    case SIGQUIT:
        abort_thread( 0 );
    default:
        tw_unsupported( "a pending signal with no handler (M4: SIGQUIT only)" );
    }
}

/* The harness's routine in place of a thread's guest entry; NULL in the
 * real module (tempwind_private.h). */
void (*tw_thread_start_hook)( void *entry, void *arg, void *state );

/* The main thread's state block, once boot has stopped at the entry. */
void *tw_main_state( void )
{
    if (!main_state) tw_unsupported( "tw_main_state: boot has not stopped at the entry" );
    return main_state;
}

/* The thread's start as signal_x86_64.c's init_syscall_frame builds it
 * (section 8): the CONTEXT that LdrInitializeThunk gets, below the stack's
 * top, and the state block that enters LdrInitializeThunk with RCX = that
 * CONTEXT. Then BOOT_STOP_AT_ENTRY returns to boot, and run mode runs it.
 * From M4 the same for every thread, from start_thread (unix/thread.c
 * L1119-1132) on its own green thread; RDX is then the thread's argument. */
void signal_start_thread( PRTL_THREAD_START_ROUTINE entry, void *arg, BOOL suspend, TEB *teb )
{
    struct syscall_frame *frame = get_syscall_frame();
    CONTEXT *ctx, context = { 0 };
    BYTE *state;
    int is_main = green_is_main();

    if (suspend) tw_unsupported( "signal_start_thread: a suspended start (M4 WP4)" );
    if (teb != NtCurrentTeb()) tw_unsupported( "signal_start_thread: a TEB other than the running thread's" );
    if (!frame) tw_unsupported( "signal_start_thread: no frame record (signal_init_process, signal_alloc_thread)" );
    if (frame->own_state) tw_unsupported( "signal_start_thread: a thread started twice" );

    context.ContextFlags = CONTEXT_ALL | CONTEXT_EXCEPTION_REPORTING | CONTEXT_EXCEPTION_ACTIVE;
    context.Rcx    = (ULONG_PTR)entry;
    context.Rdx    = (ULONG_PTR)arg;
    context.Rsp    = (ULONG_PTR)teb->Tib.StackBase - 0x28;
    context.Rip    = (ULONG_PTR)pRtlUserThreadStart;
    context.SegCs  = cs64_sel;
    context.SegDs  = ds64_sel;
    context.SegEs  = ds64_sel;
    context.SegFs  = 0;
    context.SegGs  = ds64_sel;
    context.SegSs  = ds64_sel;
    context.EFlags = 0x200;
    context.FltSave.ControlWord = 0x27f;
    context.FltSave.MxCsr = context.MxCsr = 0x1f80;

    ctx = (CONTEXT *)((ULONG_PTR)context.Rsp & ~15) - 1;
    *ctx = context;
    ctx->ContextFlags = CONTEXT_FULL;

    if (!(state = aligned_alloc( 64, TW_STATE_SIZE )))
        tw_unsupported( "signal_start_thread: no host memory for the state block" );
    memset( state, 0, TW_STATE_SIZE );
    GPR( state, RCX ) = (ULONG_PTR)ctx;
    GPR( state, RDX ) = (ULONG_PTR)arg;      /* the PEB, or a new thread's argument */
    GPR( state, RSP ) = (ULONG_PTR)ctx - 8;
    S64( state, TW_STATE_RIP ) = (ULONG_PTR)pLdrInitializeThunk;
    S64( state, TW_STATE_RFLAGS ) = 0x200;
    S64( state, TW_STATE_GS_BASE ) = (ULONG_PTR)teb;
    S32( state, TW_STATE_MXCSR ) = TW_INIT_MXCSR;
    S16( state, TW_STATE_FCW ) = TW_INIT_FCW;
    S16( state, TW_STATE_FTW ) = TW_INIT_FTW;  /* all x87 registers empty */
    if (is_main) main_state = state;
    frame->own_state = state;
    frame->state = state;

    /* BOOT_STOP_AT_ENTRY; boot rejects every other nonzero value */
    if (is_main && tw_boot_flags) tw_boot_stopped();
    if (tw_thread_start_hook) tw_thread_start_hook( (void *)entry, arg, state );
    else tw_run_loop( (uint64_t)(ULONG_PTR)state );
    /* EXIT_HOST: nothing ends a thread that way in M3 and M4 (section 12): a
     * thread ends in pthread_exit, and process exit longjmps out */
    fprintf( stderr, "tempwind: unsupported: signal_start_thread: thread %d's run loop returned: %s\n",
             green_current->id, tw_thread_start_hook ? "the harness's routine returned" : tw_host_stop_reason );
    abort();
}

/* ---- the dispatchers (section 11; abi.md v5, "Syscalls and unix calls") ---- */

static struct syscall_frame *entry_frame( BYTE *state, const char *who )
{
    struct syscall_frame *frame = get_syscall_frame();

    if (!frame) tw_unsupported( who );
    frame->state = state;
    return frame;
}

/* Wine's return path (L3191-3276; for a unix call, L3438). The state block
 * already holds what Wine restores from the frame: the callee-saved GPRs,
 * MXCSR and XMM6-15, or the full FP state with restore flag 0x48. */
static int32_t dispatcher_return( BYTE *state, struct syscall_frame *frame, ULONG_PTR status, UINT64 ret )
{
    /* M4, delivery point 3 (section 9): a signal the thread sent itself, or
     * one that arrived while it ran, before it returns to guest code */
    green_deliver_signals();
    if (frame->restore_flags & 0x3)  /* CONTEXT_CONTROL | CONTEXT_INTEGER: iretq to the context */
    {
        /* the new-context return (abi.md v5, "Host functions"): with INTEGER
         * every GPR is the frame's, RAX included (L3266-3272) */
        if (!(frame->restore_flags & 0x2)) GPR( state, RAX ) = status;
        S32( state, TW_STATE_CC_OP ) = 0;
        S64( state, TW_STATE_CC_A ) = S64( state, TW_STATE_RFLAGS );
        return TW_STATUS_UNWIND;
    }
    /* the guest's ret with the popped address: Wine jumps to the frame's rip,
     * a ret of its own thunk with the same RSP (L3231-3252) */
    GPR( state, RAX ) = status;
    if (frame->ret_addr == ret) return TW_STATUS_RETURNED;
    S64( state, TW_STATE_RIP ) = frame->ret_addr;
    return TW_STATUS_UNWIND;
}

/* Wine's __wine_syscall_dispatcher (L3026-3334). A PE thunk ran
 * mov r10, rcx; mov eax, id; ...; call *0x7ffe1000 (include/wine/asm.h
 * L287-301): [RSP] is the thunk's return address, [RSP + 8] its caller's. */
int32_t tw_syscall_dispatcher( uint64_t state_addr, int32_t entry, uint64_t ret )
{
    BYTE *state = (BYTE *)(ULONG_PTR)state_addr;
    struct syscall_frame *frame = entry_frame( state, "tw_syscall_dispatcher: a thread with no frame record" );
    ULONG_PTR args[32], status;
    UINT64 rsp;
    UINT id, table, num, i, nargs;
    BOOL trace;

    set_current_time();  /* KUSER's clock, as the server's tick would have kept it */

    /* the frame record (L3029-3063): pop the return address, the thunk's
     * ret after its syscall as the frame's rip, the exact EFLAGS of pushfq */
    rsp = GPR( state, RSP );
    frame->ret_addr = *(const UINT64 *)(ULONG_PTR)rsp;
    GPR( state, RSP ) = rsp += 8;
    S64( state, TW_STATE_RIP ) = frame->ret_addr - 0xb;
    tw_flags_materialize( state_addr );
    frame->restore_flags = 0;

    /* the id (L3139-3146): table in bits 12-13, number in bits 0-11 */
    id = frame->syscall_id = (UINT)GPR( state, RAX );
    table = (id >> 12) & 3;
    num = id & 0xfff;
    if (table)
    {
        if (KeServiceDescriptorTable[table].ServiceLimit)
            tw_unsupported( "tw_syscall_dispatcher: syscall table 1-3 (win32u, M4)" );
        return dispatcher_return( state, frame, (ULONG)STATUS_INVALID_SYSTEM_SERVICE, ret );
    }
    if (num >= tw_syscall_count)
        return dispatcher_return( state, frame, (ULONG)STATUS_INVALID_SYSTEM_SERVICE, ret );  /* L3324-3327 */

    /* the arguments: R10, RDX, R8, R9, then argument i >= 4 at
     * [RSP + 0x28 + 8 * (i - 4)] after the pop (L3099, L3157, L3166-3167) */
    nargs = tw_syscall_args[num] / sizeof(ULONG_PTR);
    if (nargs > ARRAY_SIZE(args)) tw_unsupported( "tw_syscall_dispatcher: too many arguments" );
    args[0] = GPR( state, R10 );
    args[1] = GPR( state, RDX );
    args[2] = GPR( state, R8 );
    args[3] = GPR( state, R9 );
    for (i = 4; i < nargs; i++) args[i] = *(const UINT64 *)(ULONG_PTR)(rsp + 0x28 + 8 * (i - 4));

    trace = ntdll_get_thread_data()->syscall_trace & 1;  /* L3160-3161 */
    if (trace) trace_syscall( id, args, tw_syscall_args[num] );
    status = tw_syscall_table[num]( args );
    if (trace) trace_sysret( id, status );  /* L3293-3322 */
    return dispatcher_return( state, frame, status, ret );
}

/* Wine's __wine_unix_call_dispatcher (L3352-3464): RCX = the handle,
 * EDX = the code, R8 = the argument pointer. */
int32_t tw_unix_call_dispatcher( uint64_t state_addr, int32_t entry, uint64_t ret )
{
    BYTE *state = (BYTE *)(ULONG_PTR)state_addr;
    struct syscall_frame *frame = entry_frame( state, "tw_unix_call_dispatcher: a thread with no frame record" );
    UINT64 rsp = GPR( state, RSP );
    UINT code = (UINT)GPR( state, RDX );
    NTSTATUS status;

    /* the frame record: the return address unadjusted (L3356), and the frame
     * marked incomplete (L3359); no pushfq, so the flags stay lazy */
    frame->ret_addr = *(const UINT64 *)(ULONG_PTR)rsp;
    GPR( state, RSP ) = rsp + 8;
    S64( state, TW_STATE_RIP ) = frame->ret_addr;
    frame->restore_flags = RESTORE_FLAGS_INCOMPLETE_FRAME_CONTEXT;

    /* only ntdll's own unix calls; Wine checks neither */
    if (GPR( state, RCX ) != (ULONG_PTR)tw_unix_call_funcs)
        tw_unsupported( "tw_unix_call_dispatcher: a unix library other than ntdll's" );
    if (code >= tw_unix_call_count)
        tw_unsupported( "tw_unix_call_dispatcher: a code past ntdll's unix calls" );
    status = tw_unix_call_funcs[code]( (void *)(ULONG_PTR)GPR( state, R8 ) );
    return dispatcher_return( state, frame, (ULONG)status, ret );
}

/* ---- contexts on the current thread (section 11) ---- */

static BYTE *context_state( const char *who )
{
    struct syscall_frame *frame = get_syscall_frame();

    if (!frame || !frame->state) tw_unsupported( who );
    return frame->state;
}

/* An x87 register's full tag from its contents (translator.md, "x87",
 * Tags): 00 valid, 01 zero, 10 special. */
static unsigned int x87_tag( const BYTE *reg )
{
    UINT64 significand = *(const UINT64 *)reg;
    unsigned int exponent = *(const UINT16 *)(reg + 8) & 0x7fff;

    if (exponent == 0x7fff) return 2;
    if (!exponent) return significand ? 2 : 1;
    if (!(significand >> 63)) return 2;  /* J bit clear: unnormal */
    return 0;
}

/* The state block's FP state as the translator's fxsave stores it
 * (translator.md, "fxsave m512"): FCW, FSW, the abridged tag byte (bit p:
 * R[p] not empty), FOP, FIP and FDP 0, MXCSR, MXCSR_MASK, ST(0)-ST(7) with
 * the slot padding 0, XMM0-15. Bytes 416-511 are 0 here. */
static void fp_from_state( const BYTE *state, XSAVE_FORMAT *fx )
{
    UINT16 fsw = S16( state, TW_STATE_FSW ), ftw = S16( state, TW_STATE_FTW );
    unsigned int top = (fsw >> 11) & 7, i;

    memset( fx, 0, sizeof(*fx) );
    fx->ControlWord = S16( state, TW_STATE_FCW );
    fx->StatusWord = fsw;
    for (i = 0; i < 8; i++)
        if (((ftw >> (2 * i)) & 3) != 3) fx->TagWord |= 1 << i;
    fx->MxCsr = S32( state, TW_STATE_MXCSR );
    fx->MxCsr_Mask = TW_MXCSR_MASK;
    for (i = 0; i < 8; i++)  /* FloatRegisters[i] = ST(i) = R[(TOP + i) mod 8] */
        memcpy( &fx->FloatRegisters[i], state + TW_STATE_FPR + 16 * ((top + i) & 7), 10 );
    memcpy( fx->XmmRegisters, state + TW_STATE_XMM, sizeof(fx->XmmRegisters) );
}

/* The other way: each register's full tag comes from its contents, or is
 * empty when the abridged bit is clear. MXCSR keeps only the bits
 * MXCSR_MASK allows (fxrstor would fault on the others). */
static void fp_to_state( const XSAVE_FORMAT *fx, BYTE *state )
{
    unsigned int top = (fx->StatusWord >> 11) & 7, p;
    UINT16 ftw = 0;

    S16( state, TW_STATE_FCW ) = fx->ControlWord;
    S16( state, TW_STATE_FSW ) = fx->StatusWord;
    S32( state, TW_STATE_MXCSR ) = fx->MxCsr & TW_MXCSR_MASK;
    for (p = 0; p < 8; p++)
    {
        BYTE *reg = state + TW_STATE_FPR + 16 * p;

        memset( reg, 0, 16 );
        memcpy( reg, &fx->FloatRegisters[(p - top) & 7], 10 );
        ftw |= ((fx->TagWord >> p) & 1 ? x87_tag( reg ) : 3) << (2 * p);
    }
    S16( state, TW_STATE_FTW ) = ftw;
    memcpy( state + TW_STATE_XMM, fx->XmmRegisters, sizeof(fx->XmmRegisters) );
}

/***********************************************************************
 *           signal_set_full_context  (L1083-1090)
 */
NTSTATUS signal_set_full_context( CONTEXT *context )
{
    NTSTATUS status = NtSetContextThread( GetCurrentThread(), context );

    if (!status && (context->ContextFlags & CONTEXT_INTEGER) == CONTEXT_INTEGER)
        get_syscall_frame()->restore_flags |= CONTEXT_INTEGER;
    return status;
}

/***********************************************************************
 *           NtSetContextThread  (L1116-1208), for the current thread
 */
NTSTATUS WINAPI NtSetContextThread( HANDLE handle, const CONTEXT *context )
{
    DWORD flags = context->ContextFlags & ~CONTEXT_AMD64;
    BYTE *state;

    if (handle != GetCurrentThread()) tw_unsupported( "NtSetContextThread: another thread's context (M4)" );
    state = context_state( "NtSetContextThread: outside a syscall" );

    flags &= ~CONTEXT_XSTATE;  /* no extended features: the profile has no AVX */
    /* debug registers other than the cached ones, all 0, need the server */
    if ((flags & CONTEXT_DEBUG_REGISTERS) &&
        (context->Dr0 || context->Dr1 || context->Dr2 || context->Dr3 || context->Dr6 || context->Dr7))
        tw_unsupported( "NtSetContextThread: debug registers (the server's set_thread_context, M4)" );

    if (flags & CONTEXT_INTEGER)
    {
        GPR( state, RAX ) = context->Rax;
        GPR( state, RBX ) = context->Rbx;
        GPR( state, RCX ) = context->Rcx;
        GPR( state, RDX ) = context->Rdx;
        GPR( state, RBP ) = context->Rbp;
        GPR( state, RSI ) = context->Rsi;
        GPR( state, RDI ) = context->Rdi;
        GPR( state, R8 )  = context->R8;
        GPR( state, R9 )  = context->R9;
        GPR( state, R10 ) = context->R10;
        GPR( state, R11 ) = context->R11;
        GPR( state, R12 ) = context->R12;
        GPR( state, R13 ) = context->R13;
        GPR( state, R14 ) = context->R14;
        GPR( state, R15 ) = context->R15;
    }
    if (flags & CONTEXT_CONTROL)
    {
        GPR( state, RSP ) = context->Rsp;
        S64( state, TW_STATE_RIP ) = context->Rip;
        S64( state, TW_STATE_RFLAGS ) = context->EFlags;
    }
    if (flags & CONTEXT_FLOATING_POINT) fp_to_state( &context->FltSave, state );

    get_syscall_frame()->restore_flags |= flags & ~CONTEXT_INTEGER;  /* L1206 */
    return STATUS_SUCCESS;
}

/***********************************************************************
 *           NtGetContextThread  (L1215-1346), for the current thread
 */
NTSTATUS WINAPI NtGetContextThread( HANDLE handle, CONTEXT *context )
{
    DWORD needed_flags = context->ContextFlags & ~CONTEXT_AMD64;
    const BYTE *state;

    if (handle != GetCurrentThread()) tw_unsupported( "NtGetContextThread: another thread's context (M4)" );
    state = context_state( "NtGetContextThread: outside a syscall" );

    if (needed_flags & CONTEXT_INTEGER)
    {
        context->Rax = GPR( state, RAX );
        context->Rbx = GPR( state, RBX );
        context->Rcx = GPR( state, RCX );
        context->Rdx = GPR( state, RDX );
        context->Rbp = GPR( state, RBP );
        context->Rsi = GPR( state, RSI );
        context->Rdi = GPR( state, RDI );
        context->R8  = GPR( state, R8 );
        context->R9  = GPR( state, R9 );
        context->R10 = GPR( state, R10 );
        context->R11 = GPR( state, R11 );
        context->R12 = GPR( state, R12 );
        context->R13 = GPR( state, R13 );
        context->R14 = GPR( state, R14 );
        context->R15 = GPR( state, R15 );
        context->ContextFlags |= CONTEXT_INTEGER;
    }
    if (needed_flags & CONTEXT_CONTROL)
    {
        context->Rsp    = GPR( state, RSP );
        context->Rip    = S64( state, TW_STATE_RIP );
        context->EFlags = (DWORD)S64( state, TW_STATE_RFLAGS );  /* materialized at entry */
        context->SegCs  = cs64_sel;
        context->SegSs  = ds64_sel;
        context->ContextFlags |= CONTEXT_CONTROL;
    }
    if (needed_flags & CONTEXT_SEGMENTS)
    {
        context->SegDs = ds64_sel;
        context->SegEs = ds64_sel;
        context->SegFs = 0;
        context->SegGs = ds64_sel;
        context->ContextFlags |= CONTEXT_SEGMENTS;
    }
    if (needed_flags & CONTEXT_FLOATING_POINT)
    {
        fp_from_state( state, &context->FltSave );
        context->MxCsr = context->FltSave.MxCsr;
        context->ContextFlags |= CONTEXT_FLOATING_POINT;
    }
    if (needed_flags & CONTEXT_DEBUG_REGISTERS)  /* what the server keeps: none set */
    {
        context->Dr0 = context->Dr1 = context->Dr2 = context->Dr3 = context->Dr6 = context->Dr7 = 0;
        context->ContextFlags |= CONTEXT_DEBUG_REGISTERS;
    }
    /* CONTEXT_XSTATE: none, the profile has no AVX */
    set_context_exception_reporting_flags( &context->ContextFlags, CONTEXT_SERVICE_ACTIVE );
    return STATUS_SUCCESS;
}

void *get_native_context( CONTEXT *context )
{
    return context;
}

void *get_wow_context( CONTEXT *context )
{
    tw_unsupported( "get_wow_context" );
}

NTSTATUS get_thread_wow64_context( HANDLE handle, void *ctx, ULONG size )
{
    tw_unsupported( "get_thread_wow64_context" );
}

NTSTATUS set_thread_wow64_context( HANDLE handle, const void *ctx, ULONG size )
{
    tw_unsupported( "set_thread_wow64_context" );
}

NTSTATUS get_thread_ldt_entry( HANDLE handle, THREAD_DESCRIPTOR_INFORMATION *info, ULONG len )
{
    tw_unsupported( "get_thread_ldt_entry" );
}

void set_process_instrumentation_callback( void *callback )
{
    tw_unsupported( "set_process_instrumentation_callback" );
}

/* ---- calls into the guest's dispatchers (M4-M5) ---- */

NTSTATUS call_user_apc_dispatcher( CONTEXT *context_ptr, unsigned int flags, ULONG_PTR arg1, ULONG_PTR arg2,
                                   ULONG_PTR arg3, PNTAPCFUNC func, NTSTATUS status )
{
    tw_unsupported( "call_user_apc_dispatcher" );
}

NTSTATUS call_user_exception_dispatcher( EXCEPTION_RECORD *rec, CONTEXT *context )
{
    tw_unsupported( "call_user_exception_dispatcher" );
}

void call_raise_user_exception_dispatcher( void )
{
    tw_unsupported( "call_raise_user_exception_dispatcher" );
}

NTSTATUS WINAPI NtCallbackReturn( void *ret_ptr, ULONG ret_len, NTSTATUS status )
{
    tw_unsupported( "NtCallbackReturn" );
}

/*
 * The `exception` checks (docs/design/wine-unix.md v4, section 10, "The M5
 * checks"): the raise path of section 11, "Exceptions", and the run loop's
 * records of section 12, on synthetic state blocks with no guest code.
 *
 * pKiUserExceptionDispatcher is a sentinel the harness sets, as the signal
 * harness's APC check does: nothing runs there, so each check reads what the
 * raise wrote, the layout below the guest's RSP and the state block that
 * would enter PE KiUserExceptionDispatcher.
 *
 * The run loop's checks enter tw_run_loop with the exit reason in the state
 * block: the JS test's tw_run returns the state block's exit reason and then
 * leaves EXIT_HOST there (abi.md, "CPU state block": the field holds the
 * value run returns), so the loop acts on that exit once, raises, and returns
 * at the next call with tw_host_stop_reason set. The syscall checks call the
 * dispatcher from C as translated code would, as the syscall harness does.
 *
 * The setup is the syscall harness's (tw_process_start) and a committed guest
 * stack; the JS test puts the prefix and l_intl.nls into MEMFS first. Line
 * numbers are wine-11.0's dlls/ntdll/unix/signal_x86_64.c unless another
 * file is named.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include "config.h"

#include <stdarg.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winnt.h"
#include "winternl.h"
#include "ddk/wdm.h"
#include "unix_private.h"
#include "wine/debug.h"

#include "harness.h"
#include "tempwind/abi.h"
#include "tempwind_private.h"
#include "server.h"

/* The dispatchers (src/signal_wasm64.c) and the generated table. */
int32_t tw_syscall_dispatcher( uint64_t state, int32_t entry, uint64_t ret );
int32_t tw_unix_call_dispatcher( uint64_t state, int32_t entry, uint64_t ret );
extern const char *const tw_syscall_names[];
extern const unsigned int tw_syscall_count;

/* signal_x86_64.c's struct exc_stack_layout (L418-429): what the raise
 * writes below the guest's RSP. */
struct machine_frame { ULONG64 rip, cs, eflags, rsp, ss; };
struct exc_stack_layout
{
    CONTEXT              context;        /* 000 */
    CONTEXT_EX           context_ex;     /* 4d0 */
    EXCEPTION_RECORD     rec;            /* 4f0 */
    ULONG64              align;          /* 588 */
    struct machine_frame machine_frame;  /* 590 */
    ULONG64              align2;         /* 5b8 */
};
_Static_assert( offsetof(struct exc_stack_layout, context_ex) == 0x4d0, "layout" );
_Static_assert( offsetof(struct exc_stack_layout, rec) == 0x4f0, "layout" );
_Static_assert( offsetof(struct exc_stack_layout, machine_frame) == 0x590, "layout" );
_Static_assert( sizeof(struct exc_stack_layout) == 0x5c0, "layout" );

#define EXC_DISPATCHER 0x7ffe1240  /* the sentinel pKiUserExceptionDispatcher */
#define XSTATE_SIZE    64          /* sizeof(XSAVE_AREA_HEADER) (L525): KUSER's XState.Size is 0 */
#define FAULT_RIP      0x180003456ull  /* a faulting instruction's address */
#define THUNK_RET      0x180001234ull  /* [RSP]: the thunk's return address */
#define CALLER_RET     0x180005678ull  /* [RSP + 8]: its caller's */
#define WIN32U_SO      "/wine/lib/wine/x86_64-unix/win32u.so"
#define TF_DF_AC       (0x100 | 0x400 | 0x40000)

#define S16(s, off) (*(UINT16 *)((BYTE *)(s) + (off)))
#define FPR(s, p)   ((BYTE *)(s) + TW_STATE_FPR + 16 * (p))

static UINT64 stack_top;   /* a committed guest stack */
static BYTE *ro_page;      /* a committed read-only page */
static BYTE *rw_page;      /* a committed read-write page */

/* Where the layout goes for a guest RSP (L1604-1606): the section 10 form,
 * (rsp - 0x5c0 - xstate_size) & ~63. */
static struct exc_stack_layout *layout_for( UINT64 rsp )
{
    return (struct exc_stack_layout *)((rsp - sizeof(struct exc_stack_layout) - XSTATE_SIZE) & ~(UINT64)63);
}

/* An x87 register: significand, then sign and exponent. */
static void set_reg( BYTE *reg, UINT64 significand, UINT16 sign_exponent )
{
    memset( reg, 0, 16 );
    memcpy( reg, &significand, 8 );
    memcpy( reg + 8, &sign_exponent, 2 );
}

/* A state block as translated code leaves it at a fault: distinct GPRs and
 * XMM registers, RIP at the fault, TF, DF and AC set besides the status
 * flags, and an x87 state with TOP = 3, R0 valid, R1 zero and R2-R7 empty
 * with contents. */
static BYTE *new_state( UINT64 rsp )
{
    BYTE *state = aligned_alloc( 64, TW_STATE_SIZE );
    unsigned int i;

    if (!state) abort();
    memset( state, 0, TW_STATE_SIZE );
    for (i = 0; i < 16; i++) GPR( state, i ) = 0x0101010101010101ull * (i + 1);
    GPR( state, RSP ) = rsp;
    S64( state, TW_STATE_RIP ) = FAULT_RIP;
    S64( state, TW_STATE_RFLAGS ) = 0x202 | 0x8c1 | TF_DF_AC;  /* IF, CF SF OF, TF DF AC */
    S32( state, TW_STATE_CC_OP ) = 0;
    S64( state, TW_STATE_CC_A ) = S64( state, TW_STATE_RFLAGS );
    for (i = 0; i < 16; i++)
    {
        S64( state, TW_STATE_XMM + 16 * i ) = 0x2222000000000000ull + 0x100 * i;
        S64( state, TW_STATE_XMM + 16 * i + 8 ) = ~(UINT64)i;
    }
    S32( state, TW_STATE_MXCSR ) = 0x1fa0;
    S16( state, TW_STATE_FCW ) = 0x037f;
    S16( state, TW_STATE_FSW ) = (3 << 11) | 0x21;  /* TOP 3, PE and IE */
    S16( state, TW_STATE_FTW ) = 0xfff4;            /* R0 valid, R1 zero, R2-R7 empty */
    set_reg( FPR( state, 0 ), 0x8000000000000000ull, 0x3fff );  /* 1.0 */
    set_reg( FPR( state, 1 ), 0, 0 );
    for (i = 2; i < 8; i++) set_reg( FPR( state, i ), 0x0123456789abcdefull * i, 0x4000 + i );
    return state;
}

/* The run loop on one exit (the file's comment): it raises, or stops. */
static void run_exit( BYTE *state, UINT32 exit, UINT32 kind, UINT64 addr )
{
    S32( state, TW_STATE_EXIT ) = exit;
    S32( state, TW_STATE_FAULT_KIND ) = kind;
    S64( state, TW_STATE_FAULT_ADDR ) = addr;
    tw_host_stop_reason = "the exception harness's tw_run";
    tw_run_loop( (uintptr_t)state );
}

/* Fails unless the state block entered the sentinel dispatcher with the
 * layout for rsp below it: RSP = the layout, RBP and the other GPRs as
 * before, TF, DF and AC cleared, the lazy flags record materialized, and the
 * layout's CONTEXT_EX and machine frame as the raise writes them. */
static const char *check_entry( const BYTE *state, const BYTE *before, UINT64 rsp )
{
    const struct exc_stack_layout *stack = layout_for( rsp );
    UINT64 rflags = S64( before, TW_STATE_RFLAGS ) & ~(UINT64)TF_DF_AC;
    unsigned int i;

    TW_EXPECT( S64( state, TW_STATE_RIP ) == EXC_DISPATCHER );
    TW_EXPECT( GPR( state, RSP ) == (UINT64)(ULONG_PTR)stack );
    for (i = 0; i < 16; i++) TW_EXPECT( i == RSP || GPR( state, i ) == GPR( before, i ) );
    TW_EXPECT( S64( state, TW_STATE_RFLAGS ) == rflags );
    TW_EXPECT( S32( state, TW_STATE_CC_OP ) == 0 && S64( state, TW_STATE_CC_A ) == rflags );
    TW_EXPECT( !memcmp( state + TW_STATE_XMM, before + TW_STATE_XMM, 0x300 ) );  /* XMM, MXCSR, x87 */
    /* context_init_xstate's branch without an xstate buffer (L584-590) */
    TW_EXPECT( stack->context_ex.Legacy.Offset == -(LONG)sizeof(CONTEXT) );
    TW_EXPECT( stack->context_ex.Legacy.Length == sizeof(CONTEXT) );
    TW_EXPECT( stack->context_ex.XState.Offset == 0 && stack->context_ex.XState.Length == 25 );
    TW_EXPECT( stack->context_ex.All.Offset == -(LONG)sizeof(CONTEXT) );
    TW_EXPECT( stack->context_ex.All.Length == sizeof(CONTEXT) + 24 );
    TW_EXPECT( stack->machine_frame.rip == S64( before, TW_STATE_RIP ) );
    TW_EXPECT( stack->machine_frame.rsp == rsp );
    return NULL;
}

/* ---- the layout and the context ---- */

/* exc_stack_layout's address for an RSP on each 8-byte offset of a 64-byte
 * line, and the state block after it (section 10). */
static const char *layout( void )
{
    UINT64 line = (stack_top - 0x1000) & ~(UINT64)63;
    BYTE before[TW_STATE_SIZE];
    const char *err;
    unsigned int k;

    for (k = 0; k < 8; k++)
    {
        BYTE *state = new_state( line + 8 * k );

        memcpy( before, state, TW_STATE_SIZE );
        run_exit( state, TW_EXIT_FAULT, TW_FAULT_DIVIDE, 0 );
        TW_EXPECT( S32( state, TW_STATE_EXIT ) == TW_EXIT_HOST );  /* the loop called tw_run again */
        if ((err = check_entry( state, before, line + 8 * k ))) return err;
        TW_EXPECT( (UINT64)(ULONG_PTR)layout_for( line + 8 * k ) == line - 0x600 );
        free( state );
    }
    return NULL;
}

/* The context in the layout is the state block's from before the fault,
 * with NtGetContextThread's conversion and ContextFlags = CONTEXT_ALL, as
 * save_context gives (L970-1034). */
static const char *context( void )
{
    BYTE *state = new_state( stack_top - 0x1000 - 0x18 );
    BYTE before[TW_STATE_SIZE];
    const struct exc_stack_layout *stack = layout_for( stack_top - 0x1000 - 0x18 );
    const CONTEXT *c = &stack->context;
    unsigned int i, top = 3;
    const char *err;

    memcpy( before, state, TW_STATE_SIZE );
    run_exit( state, TW_EXIT_FAULT, TW_FAULT_INVALID_OPCODE, 0 );
    if ((err = check_entry( state, before, stack_top - 0x1000 - 0x18 ))) return err;
    TW_EXPECT( c->ContextFlags == CONTEXT_ALL );
    for (i = 0; i < 16; i++) TW_EXPECT( (&c->Rax)[i] == GPR( before, i ) );
    TW_EXPECT( c->Rip == FAULT_RIP );
    TW_EXPECT( c->EFlags == (DWORD)S64( before, TW_STATE_RFLAGS ) );
    TW_EXPECT( c->SegCs == 0x33 && c->SegSs == 0x2b );
    TW_EXPECT( c->SegDs == 0x2b && c->SegEs == 0x2b && c->SegGs == 0x2b && c->SegFs == 0 );
    TW_EXPECT( c->MxCsr == 0x1fa0 && c->FltSave.MxCsr == 0x1fa0 && c->FltSave.MxCsr_Mask == TW_MXCSR_MASK );
    TW_EXPECT( c->FltSave.ControlWord == 0x037f && c->FltSave.StatusWord == ((3 << 11) | 0x21) );
    TW_EXPECT( c->FltSave.TagWord == 0x03 );  /* abridged: R0 and R1 not empty */
    for (i = 0; i < 8; i++)  /* FloatRegisters[i] = ST(i) = R[(TOP + i) mod 8] */
        TW_EXPECT( !memcmp( &c->FltSave.FloatRegisters[i], FPR( before, (top + i) & 7 ), 10 ) );
    TW_EXPECT( !memcmp( c->FltSave.XmmRegisters, before + TW_STATE_XMM, 16 * 16 ) );
    TW_EXPECT( !c->Dr0 && !c->Dr1 && !c->Dr2 && !c->Dr3 && !c->Dr6 && !c->Dr7 );
    free( state );
    return NULL;
}

/* ---- each kind's record (section 12, "The records") ---- */

/* The record in the layout after one exit: ExceptionFlags 0,
 * ExceptionAddress = the state block's RIP, and the code and parameters. */
static const char *expect_record( UINT32 exit, UINT32 kind, UINT64 addr, UINT64 rip,
                                  DWORD code, DWORD nparams, UINT64 p0, UINT64 p1 )
{
    BYTE *state = new_state( stack_top - 0x1000 );
    BYTE before[TW_STATE_SIZE];
    const struct exc_stack_layout *stack = layout_for( stack_top - 0x1000 );
    const char *err;

    S64( state, TW_STATE_RIP ) = rip;
    memcpy( before, state, TW_STATE_SIZE );
    run_exit( state, exit, kind, addr );
    if ((err = check_entry( state, before, stack_top - 0x1000 )))
        return tw_fail( "exit %u kind %u: %s", exit, kind, err );
    if (stack->rec.ExceptionCode != code || stack->rec.ExceptionFlags || stack->rec.ExceptionRecord ||
        stack->rec.ExceptionAddress != (void *)(ULONG_PTR)rip || stack->rec.NumberParameters != nparams ||
        (nparams > 0 && stack->rec.ExceptionInformation[0] != p0) ||
        (nparams > 1 && stack->rec.ExceptionInformation[1] != p1))
        return tw_fail( "exit %u kind %u: code %#x flags %#x address %p, %u parameters %#llx %#llx", exit, kind,
                        (unsigned)stack->rec.ExceptionCode, (unsigned)stack->rec.ExceptionFlags,
                        stack->rec.ExceptionAddress, (unsigned)stack->rec.NumberParameters,
                        (unsigned long long)stack->rec.ExceptionInformation[0],
                        (unsigned long long)stack->rec.ExceptionInformation[1] );
    /* no Rip-- (section 11): the context's Rip is the faulting instruction's */
    TW_EXPECT( stack->context.Rip == rip );
    free( state );
    return NULL;
}

/* The eight fault kinds, each through the run loop's raise. */
static const char *fault_records( void )
{
    const UINT64 high = TW_MAX_MEMORY_PAGES << 16;  /* 16 GiB: the rep walk's bound, no view */
    const char *err;

    if ((err = expect_record( TW_EXIT_FAULT, TW_FAULT_READ, 0x18, FAULT_RIP,
                              STATUS_ACCESS_VIOLATION, 2, EXCEPTION_READ_FAULT, 0x18 ))) return err;
    if ((err = expect_record( TW_EXIT_FAULT, TW_FAULT_WRITE, 0x28, FAULT_RIP,
                              STATUS_ACCESS_VIOLATION, 2, EXCEPTION_WRITE_FAULT, 0x28 ))) return err;
    if ((err = expect_record( TW_EXIT_FAULT, TW_FAULT_EXECUTE, 0x38, FAULT_RIP,
                              STATUS_ACCESS_VIOLATION, 2, EXCEPTION_EXECUTE_FAULT, 0x38 ))) return err;
    if ((err = expect_record( TW_EXIT_FAULT, TW_FAULT_DIVIDE, 0, FAULT_RIP,
                              STATUS_INTEGER_DIVIDE_BY_ZERO, 0, 0, 0 ))) return err;
    if ((err = expect_record( TW_EXIT_FAULT, TW_FAULT_INVALID_OPCODE, 0, FAULT_RIP,
                              STATUS_ILLEGAL_INSTRUCTION, 0, 0, 0 ))) return err;
    if ((err = expect_record( TW_EXIT_FAULT, TW_FAULT_GENERAL_PROTECTION, 0, FAULT_RIP,
                              STATUS_ACCESS_VIOLATION, 2, 0, ~0ull ))) return err;
    if ((err = expect_record( TW_EXIT_FAULT, TW_FAULT_BREAKPOINT, 0, FAULT_RIP,
                              STATUS_BREAKPOINT, 1, 0, 0 ))) return err;
    /* a store-check fault on a page Wine has neither writable nor
     * write-copy, and at the rep walk's bound, where Wine has no view */
    if ((err = expect_record( TW_EXIT_FAULT, TW_FAULT_CODE_WRITE, (ULONG_PTR)ro_page + 0x10, FAULT_RIP,
                              STATUS_ACCESS_VIOLATION, 2, EXCEPTION_WRITE_FAULT, (ULONG_PTR)ro_page + 0x10 )))
        return err;
    if ((err = expect_record( TW_EXIT_FAULT, TW_FAULT_CODE_WRITE, high, FAULT_RIP,
                              STATUS_ACCESS_VIOLATION, 2, EXCEPTION_WRITE_FAULT, high ))) return err;
    return NULL;
}

/* EXIT_LEFT_CODE with no executable pages at rip: an execute access
 * violation at rip, on a committed page that isn't executable and outside
 * every view. [0] is EXCEPTION_EXECUTE_FAULT: a 64-bit process's execute
 * flags always have MEM_EXECUTE_OPTION_DISABLE (unix/process.c L1475-1478),
 * so EXCEPTION_READ_FAULT, DEP's other value, can't arise here. */
static const char *execute_records( void )
{
    ULONG flags = 0;
    const char *err;

    TW_EXPECT( !NtQueryInformationProcess( NtCurrentProcess(), ProcessExecuteFlags, &flags, sizeof(flags), NULL ) );
    TW_EXPECT( flags & MEM_EXECUTE_OPTION_DISABLE );
    if ((err = expect_record( TW_EXIT_LEFT_CODE, 0, 0, (ULONG_PTR)rw_page + 0x40,
                              STATUS_ACCESS_VIOLATION, 2, EXCEPTION_EXECUTE_FAULT, (ULONG_PTR)rw_page + 0x40 )))
        return err;
    if ((err = expect_record( TW_EXIT_LEFT_CODE, 0, 0, 0x7fff12340000ull,
                              STATUS_ACCESS_VIOLATION, 2, EXCEPTION_EXECUTE_FAULT, 0x7fff12340000ull )))
        return err;
    return NULL;
}

/* The loud stops that stay (section 12), each alone. */
static const char *code_write_writable_stops( void )
{
    BYTE *state = new_state( stack_top - 0x1000 );

    run_exit( state, TW_EXIT_FAULT, TW_FAULT_CODE_WRITE, (ULONG_PTR)rw_page + 0x20 );
    return tw_fail( "the run loop raised" );
}

static const char *unknown_kind_stops( void )
{
    BYTE *state = new_state( stack_top - 0x1000 );

    run_exit( state, TW_EXIT_FAULT, 99, 0 );
    return tw_fail( "the run loop raised" );
}

static const char *syscall_dispatcher_stops( void )
{
    BYTE *state = new_state( stack_top - 0x1000 );

    S64( state, TW_STATE_RIP ) = TW_SYSCALL_DISPATCHER;
    run_exit( state, TW_EXIT_LEFT_CODE, 0, 0 );
    return tw_fail( "the run loop raised" );
}

static const char *unix_call_dispatcher_stops( void )
{
    BYTE *state = new_state( stack_top - 0x1000 );

    S64( state, TW_STATE_RIP ) = TW_UNIX_CALL_DISPATCHER;
    run_exit( state, TW_EXIT_LEFT_CODE, 0, 0 );
    return tw_fail( "the run loop raised" );
}

/* ---- through the syscall dispatcher ---- */

static UINT syscall_id( const char *name )
{
    UINT i;

    for (i = 0; i < tw_syscall_count; i++) if (!strcmp( tw_syscall_names[i], name )) return i;
    abort();
}

/* What a PE thunk does before call *0x7ffe1000 (include/wine/asm.h
 * L287-301), as the syscall harness does: [RSP] the thunk's return
 * address, EAX the id, R10 and RDX, R8, R9 the first arguments. */
static int32_t do_syscall( BYTE *state, UINT id, const UINT64 *args, unsigned int count )
{
    UINT64 rsp = stack_top - 0x200;

    GPR( state, RSP ) = rsp;
    *(UINT64 *)rsp = THUNK_RET;
    *(UINT64 *)(rsp + 8) = CALLER_RET;
    GPR( state, RAX ) = id;
    if (count > 0) GPR( state, R10 ) = GPR( state, RCX ) = args[0];
    if (count > 1) GPR( state, RDX ) = args[1];
    if (count > 2) GPR( state, R8 ) = args[2];
    if (count > 3) abort();
    S64( state, TW_STATE_RFLAGS ) = 0x202;  /* materialized: the harness host's tw_flags_materialize needs cc_op 0 */
    S32( state, TW_STATE_CC_OP ) = 0;
    S64( state, TW_STATE_CC_A ) = 0x202;
    return tw_syscall_dispatcher( (uintptr_t)state, 0, THUNK_RET );
}

/* NtRaiseException(rec, context, TRUE) on a synthetic frame, as
 * KiUserExceptionDispatcher's caller would make it: the first chance's
 * layout below the context's RSP (not the frame's), Rip-- for
 * EXCEPTION_BREAKPOINT (L1740-1741), RBP the context's, and the
 * dispatcher's new-context return to the sentinel with RAX = the status. */
static const char *raise_first_chance( DWORD code, UINT64 rip_in_layout )
{
    static CONTEXT ctx;
    static EXCEPTION_RECORD rec;
    BYTE *state = new_state( 0 );
    const struct exc_stack_layout *stack;
    UINT64 args[3];
    unsigned int i;
    int32_t ret;

    memset( &ctx, 0, sizeof(ctx) );
    ctx.ContextFlags = CONTEXT_FULL;
    for (i = 0; i < 16; i++) (&ctx.Rax)[i] = 0x5000 + i;
    ctx.Rsp = stack_top - 0x2000 + 0x28;  /* far from the frame's, not 64-byte aligned */
    ctx.Rip = FAULT_RIP;
    ctx.EFlags = 0x202;
    ctx.SegCs = 0x33;
    ctx.SegSs = 0x2b;
    ctx.FltSave.ControlWord = 0x27f;
    ctx.FltSave.MxCsr = ctx.MxCsr = 0x1f80;
    memset( &rec, 0, sizeof(rec) );
    rec.ExceptionCode = code;
    rec.ExceptionAddress = (void *)FAULT_RIP;
    rec.NumberParameters = 1;
    rec.ExceptionInformation[0] = 0x77;
    stack = layout_for( ctx.Rsp );

    args[0] = (uintptr_t)&rec;
    args[1] = (uintptr_t)&ctx;
    args[2] = TRUE;
    ret = do_syscall( state, syscall_id( "NtRaiseException" ), args, 3 );
    TW_EXPECT( ret == TW_STATUS_UNWIND );
    TW_EXPECT( S64( state, TW_STATE_RIP ) == EXC_DISPATCHER );
    TW_EXPECT( GPR( state, RSP ) == (UINT64)(ULONG_PTR)stack );
    TW_EXPECT( GPR( state, RBP ) == ctx.Rbp );
    TW_EXPECT( GPR( state, RAX ) == STATUS_SUCCESS );
    TW_EXPECT( GPR( state, RBX ) == ctx.Rbx && GPR( state, R15 ) == ctx.R15 );  /* NtSetContextThread's */
    TW_EXPECT( stack->rec.ExceptionCode == code && stack->rec.ExceptionAddress == (void *)FAULT_RIP );
    TW_EXPECT( stack->rec.NumberParameters == 1 && stack->rec.ExceptionInformation[0] == 0x77 );
    TW_EXPECT( stack->context.Rip == rip_in_layout && stack->context.Rsp == ctx.Rsp );
    for (i = 0; i < 16; i++) TW_EXPECT( i == RSP || (&stack->context.Rax)[i] == (&ctx.Rax)[i] );
    TW_EXPECT( stack->machine_frame.rip == rip_in_layout && stack->machine_frame.rsp == ctx.Rsp );
    TW_EXPECT( stack->context_ex.XState.Length == 25 && stack->context_ex.All.Length == sizeof(CONTEXT) + 24 );
    free( state );
    return NULL;
}

static const char *first_chance( void )
{
    const char *err;

    if ((err = raise_first_chance( STATUS_BREAKPOINT, FAULT_RIP - 1 ))) return err;
    return raise_first_chance( STATUS_ACCESS_VIOLATION, FAULT_RIP );
}

/* A stub's wrapper raises EXCEPTION_WINE_STUB, noncontinuable, with the
 * module and the stub's name, at the context's Rip: the thunk's ret
 * (section 11, "Exceptions"; unix/loader.c L115-135). */
static const char *expect_stub( BYTE *state, int32_t ret, const char *module, const char *name )
{
    UINT64 rsp = stack_top - 0x200 + 8;  /* the frame's, after the pop */
    const struct exc_stack_layout *stack = layout_for( rsp );

    TW_EXPECT( ret == TW_STATUS_UNWIND );
    TW_EXPECT( S64( state, TW_STATE_RIP ) == EXC_DISPATCHER );
    TW_EXPECT( GPR( state, RSP ) == (UINT64)(ULONG_PTR)stack );
    TW_EXPECT( GPR( state, RAX ) == 0 );  /* the wrapper's 0 */
    TW_EXPECT( stack->rec.ExceptionCode == EXCEPTION_WINE_STUB );
    TW_EXPECT( stack->rec.ExceptionFlags == EXCEPTION_NONCONTINUABLE );
    TW_EXPECT( stack->rec.NumberParameters == 2 );
    TW_EXPECT( !strcmp( (const char *)stack->rec.ExceptionInformation[0], module ) );
    TW_EXPECT( !strcmp( (const char *)stack->rec.ExceptionInformation[1], name ) );
    TW_EXPECT( stack->rec.ExceptionAddress == (void *)(THUNK_RET - 0xb) );
    TW_EXPECT( stack->context.Rip == THUNK_RET - 0xb && stack->context.Rsp == rsp );
    TW_EXPECT( stack->machine_frame.rip == THUNK_RET - 0xb && stack->machine_frame.rsp == rsp );
    return NULL;
}

/* A table-0 stub (NtTraceEvent), then a table-1 stub
 * (NtBindCompositionSurface, id 0x1000) once win32u's init has registered
 * table 1 (win32u's unix call 0). */
static const char *stubs( void )
{
    BYTE *state = new_state( 0 );
    const char *err;
    int32_t ret;

    ret = do_syscall( state, syscall_id( "NtTraceEvent" ), NULL, 0 );
    if ((err = expect_stub( state, ret, "ntdll", "NtTraceEvent" ))) return err;

    GPR( state, RSP ) = stack_top - 0x200;
    *(UINT64 *)(stack_top - 0x200) = THUNK_RET;
    GPR( state, RCX ) = (uintptr_t)tw_unixlib_funcs( WIN32U_SO, 0 );
    GPR( state, RDX ) = 0;
    GPR( state, R8 ) = 0;
    ret = tw_unix_call_dispatcher( (uintptr_t)state, 0, THUNK_RET );
    TW_EXPECT( ret == TW_STATUS_RETURNED && GPR( state, RAX ) == STATUS_SUCCESS );
    ret = do_syscall( state, 0x1000, NULL, 0 );
    if ((err = expect_stub( state, ret, "win32u", "NtBindCompositionSurface" ))) return err;
    free( state );
    return NULL;
}

/* ---- the second chance ---- */

static void WINAPI raises_second_chance( void *arg )
{
    static CONTEXT ctx = { .ContextFlags = CONTEXT_FULL, .Rip = FAULT_RIP };
    static EXCEPTION_RECORD rec = { .ExceptionCode = STATUS_ACCESS_VIOLATION, .ExceptionAddress = (void *)FAULT_RIP };

    NtRaiseException( &rec, &ctx, FALSE );
    printf( "not ok second_chance: NtRaiseException returned\n" );
    fflush( stdout );
}

static void thread_hook( void *entry, void *arg, void *state )
{
    ((PRTL_THREAD_START_ROUTINE)entry)( arg );
    NtTerminateThread( NtCurrentThread(), 0 );
}

/* NtRaiseException(..., FALSE) ends the process with the exception code
 * after Wine's ERR line (unix/thread.c L1618-1626). It runs on a second
 * thread, since process exit on the main thread outside boot stops loudly;
 * the main thread sleeps meanwhile. Alone. */
static const char *second_chance( void )
{
    TW_EXPECT( spawn( raises_second_chance, NULL, 0 ) );
    sleep_ms( 60000 );  /* the scenario: the other thread's process exit cuts it short */
    return tw_fail( "the main thread woke from its sleep" );
}

static const struct tw_check checks[] =
{
    { "layout", layout },
    { "context", context },
    { "fault_records", fault_records },
    { "execute_records", execute_records },
    { "first_chance", first_chance },
    { "stubs", stubs },
    { "code_write_writable_stops", code_write_writable_stops, TW_ALONE },
    { "unknown_kind_stops", unknown_kind_stops, TW_ALONE },
    { "syscall_dispatcher_stops", syscall_dispatcher_stops, TW_ALONE },
    { "unix_call_dispatcher_stops", unix_call_dispatcher_stops, TW_ALONE },
    { "second_chance", second_chance, TW_ALONE },
};

/* A committed region of size bytes with protection prot, or NULL. */
static void *commit( SIZE_T size, ULONG prot )
{
    void *base = NULL;

    if (NtAllocateVirtualMemory( NtCurrentProcess(), &base, 0, &size, MEM_RESERVE | MEM_COMMIT, prot )) return NULL;
    return base;
}

int tw_test_main( void )
{
    void *stack;

    tw_process_start();
    if (!(stack = commit( 0x10000, PAGE_READWRITE )) || !(ro_page = commit( 0x1000, PAGE_READONLY )) ||
        !(rw_page = commit( 0x1000, PAGE_READWRITE )))
    {
        printf( "not ok setup: no guest memory\n" );
        return 1;
    }
    stack_top = (uintptr_t)stack + 0x10000;
    pKiUserExceptionDispatcher = (void *)EXC_DISPATCHER;
    tw_thread_start_hook = thread_hook;
    return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
}

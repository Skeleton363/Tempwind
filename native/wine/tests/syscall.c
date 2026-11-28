/*
 * The `syscall` checks (docs/design/wine-unix.md, section 10, "The M3
 * checks"): the syscall and unix-call dispatchers and the context functions
 * of section 11, on synthetic state blocks, with no guest code. Also
 * table 1, win32u's, and win32u's unix calls.
 *
 * A check builds a state block in the host heap, sets RSP into a committed
 * guest stack where [RSP] is the return address the thunk's
 * call *0x7ffe1000 pushed and [RSP + 8] its caller's, puts the id in EAX
 * and the arguments where abi.md v5 says ("Syscalls and unix calls"), and
 * calls the dispatcher from C as translated code would, with ret = the
 * thunk's return address.
 *
 * The setup runs before the checks, as start_main_thread and
 * server_init_process_done do (loader.c, unix/server.c L1750-1752):
 * virtual_init, the first TEB, dbg_init, the configuration directory,
 * server_init_process (the in-process server), then signal_init_process,
 * the syscall table and the +syscall flag. The JS test puts the prefix and
 * l_intl.nls into MEMFS first.
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
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/time.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winnt.h"
#include "winerror.h"
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
extern const unixlib_entry_t *const tw_unix_call_funcs;
extern const unsigned int tw_unix_call_count;
extern const unsigned int tw_win32_syscall_count;  /* table 1 */

#define THUNK_RET   0x180001234ull  /* [RSP]: the thunk's return address */
#define CALLER_RET  0x180005678ull  /* [RSP + 8]: its caller's */
#define TICKS_1601_TO_1970 116444736000000000ll

#define S16(s, off) (*(UINT16 *)((s) + (off)))
#define FPR(s, p)   ((s) + TW_STATE_FPR + 16 * (p))

static UINT64 stack_top;  /* a committed guest stack */

/* The id of a syscall, from the generated table (ntsyscalls.h's order). */
static UINT syscall_id( const char *name )
{
    UINT i;

    for (i = 0; i < tw_syscall_count; i++) if (!strcmp( tw_syscall_names[i], name )) return i;
    abort();
}

/* A state block as translated code leaves it at a call: GPRs with distinct
 * values, the TW_INIT_* FP state, flags materialized. */
static BYTE *new_state( void )
{
    BYTE *state = aligned_alloc( 64, TW_STATE_SIZE );
    unsigned int i;

    if (!state) abort();
    memset( state, 0, TW_STATE_SIZE );
    for (i = 0; i < 16; i++) GPR( state, i ) = 0x0101010101010101ull * (i + 1);
    S64( state, TW_STATE_RFLAGS ) = 0x202;
    S64( state, TW_STATE_CC_A ) = 0x202;
    S32( state, TW_STATE_MXCSR ) = TW_INIT_MXCSR;
    S16( state, TW_STATE_FCW ) = TW_INIT_FCW;
    S16( state, TW_STATE_FTW ) = TW_INIT_FTW;
    return state;
}

/* What a PE thunk does before call *0x7ffe1000 (include/wine/asm.h
 * L287-301): R10 = RCX, EAX = the id; the caller put arguments 4 and up at
 * [RSP + 0x28 + 8 * (i - 4)] of its own call, so 8 more here. */
static void setup_syscall( BYTE *state, UINT64 id, const UINT64 *args, unsigned int count )
{
    UINT64 rsp = stack_top - 0x200;
    unsigned int i;

    GPR( state, RSP ) = rsp;
    *(UINT64 *)rsp = THUNK_RET;
    *(UINT64 *)(rsp + 8) = CALLER_RET;
    GPR( state, RAX ) = id;
    if (count > 0) GPR( state, R10 ) = GPR( state, RCX ) = args[0];
    if (count > 1) GPR( state, RDX ) = args[1];
    if (count > 2) GPR( state, R8 ) = args[2];
    if (count > 3) GPR( state, R9 ) = args[3];
    for (i = 4; i < count; i++) *(UINT64 *)(rsp + 8 + 0x28 + 8 * (i - 4)) = args[i];
}

static int32_t do_syscall( BYTE *state, const char *name, const UINT64 *args, unsigned int count )
{
    setup_syscall( state, syscall_id( name ), args, count );
    return tw_syscall_dispatcher( (uintptr_t)state, 0, THUNK_RET );
}

/* Fails unless the call returned to the guest as a plain return: the status
 * in RAX, RSP past the return address, RIP the thunk's ret (abi.md v5). */
#define EXPECT_RETURNED(ret, state, status) do { \
        TW_EXPECT( (ret) == TW_STATUS_RETURNED ); \
        if (GPR( state, RAX ) != (UINT64)(ULONG)(status)) \
            return tw_fail( "%s:%d: RAX %#llx, expected %#x", __FILE__, __LINE__, \
                            (unsigned long long)GPR( state, RAX ), (unsigned)(status) ); \
        TW_EXPECT( GPR( state, RSP ) == stack_top - 0x200 + 8 ); \
    } while (0)

static UINT64 now_ticks( void )
{
    struct timeval tv;

    gettimeofday( &tv, NULL );
    return (UINT64)tv.tv_sec * 10000000 + tv.tv_usec * 10 + TICKS_1601_TO_1970;
}

static UINT64 kuser_system_time( void )
{
    const volatile KSYSTEM_TIME *t = &user_shared_data->SystemTime;

    return ((UINT64)t->High1Time << 32) | t->LowPart;
}

/* ---- the checks ---- */

/* The page at 0x7ffe1000 holds the syscall dispatcher's address (section 8). */
static const char *dispatcher_page( void )
{
    TW_EXPECT( *(const UINT64 *)0x7ffe1000 == TW_SYSCALL_DISPATCHER );
    return NULL;
}

/* A call with more than 6 arguments: NtCreateSection (7) and
 * NtMapViewOfSection (10) get their register and stack arguments. The frame
 * record: RSP past the return address, RIP the thunk's ret (the address
 * - 0xb), the flags materialized, the other GPRs kept; a return to an
 * address other than ret unwinds there. */
static const char *stack_arguments( void )
{
    static HANDLE section;
    static LARGE_INTEGER max;
    static void *base;
    static SIZE_T size;
    BYTE *state = new_state();
    MEMORY_BASIC_INFORMATION mbi;
    UINT64 args[10];
    int32_t ret;

    max.QuadPart = 0x3000;
    args[0] = (uintptr_t)&section;
    args[1] = SECTION_ALL_ACCESS;
    args[2] = 0;                    /* no attributes */
    args[3] = (uintptr_t)&max;
    args[4] = PAGE_READWRITE;       /* stack arguments from here */
    args[5] = SEC_COMMIT;
    args[6] = 0;                    /* no file */
    S64( state, TW_STATE_RFLAGS ) = 0x200 | 0x1;  /* CF in RFLAGS, but the record says ZF */
    S64( state, TW_STATE_CC_A ) = 0x40;
    ret = do_syscall( state, "NtCreateSection", args, 7 );
    EXPECT_RETURNED( ret, state, STATUS_SUCCESS );
    TW_EXPECT( section );
    TW_EXPECT( S64( state, TW_STATE_RIP ) == THUNK_RET - 0xb );
    TW_EXPECT( S64( state, TW_STATE_RFLAGS ) == 0x240 );
    TW_EXPECT( S32( state, TW_STATE_CC_OP ) == 0 && S64( state, TW_STATE_CC_A ) == 0x240 );
    TW_EXPECT( GPR( state, RBX ) == 0x0404040404040404ull && GPR( state, R15 ) == 0x1010101010101010ull );

    args[0] = (uintptr_t)section;
    args[1] = (uintptr_t)NtCurrentProcess();
    args[2] = (uintptr_t)&base;
    args[3] = 0;                    /* zero bits */
    args[4] = 0;                    /* commit size */
    args[5] = 0;                    /* offset */
    args[6] = (uintptr_t)&size;
    args[7] = ViewShare;
    args[8] = 0;                    /* allocation type */
    args[9] = PAGE_READWRITE;
    setup_syscall( state, syscall_id( "NtMapViewOfSection" ), args, 10 );
    ret = tw_syscall_dispatcher( (uintptr_t)state, 0, CALLER_RET );  /* not the thunk's: unwind */
    TW_EXPECT( ret == TW_STATUS_UNWIND );
    TW_EXPECT( GPR( state, RAX ) == STATUS_SUCCESS );
    TW_EXPECT( S64( state, TW_STATE_RIP ) == THUNK_RET );
    TW_EXPECT( base && size == 0x3000 );
    TW_EXPECT( !NtQueryVirtualMemory( NtCurrentProcess(), base, MemoryBasicInformation, &mbi, sizeof(mbi), NULL ) );
    TW_EXPECT( mbi.Type == MEM_MAPPED && mbi.Protect == PAGE_READWRITE && mbi.RegionSize == 0x3000 );
    TW_EXPECT( !NtUnmapViewOfSection( NtCurrentProcess(), base ) );
    TW_EXPECT( !NtClose( section ) );
    free( state );
    return NULL;
}

/* The two entries that use variant macros (ntsyscalls.h L295, L360). */
static const char *variant_entries( void )
{
    static LARGE_INTEGER time;
    static PROCESS_BASIC_INFORMATION pbi;
    static ULONG len;
    BYTE *state = new_state();
    UINT64 args[5];
    int32_t ret;

    args[0] = (uintptr_t)&time;
    ret = do_syscall( state, "NtQuerySystemTime", args, 1 );
    EXPECT_RETURNED( ret, state, STATUS_SUCCESS );
    TW_EXPECT( llabs( (long long)(time.QuadPart - now_ticks()) ) < 10000000 );

    args[0] = (uintptr_t)GetCurrentProcess();
    args[1] = ProcessBasicInformation;
    args[2] = (uintptr_t)&pbi;
    args[3] = sizeof(pbi);
    args[4] = (uintptr_t)&len;
    ret = do_syscall( state, "NtQueryInformationProcess", args, 5 );
    EXPECT_RETURNED( ret, state, STATUS_SUCCESS );
    TW_EXPECT( len == sizeof(pbi) );
    free( state );
    return NULL;
}

/* A number past table 0 and a number in the empty table 1 return
 * STATUS_INVALID_SYSTEM_SERVICE (L3145-3146, L3324-3327); bits 14-31 of
 * EAX and RAX's upper half are ignored. */
static const char *bad_id( void )
{
    BYTE *state = new_state();
    UINT64 handle = 0xdead;
    int32_t ret;

    setup_syscall( state, tw_syscall_count, NULL, 0 );
    ret = tw_syscall_dispatcher( (uintptr_t)state, 0, THUNK_RET );
    EXPECT_RETURNED( ret, state, STATUS_INVALID_SYSTEM_SERVICE );
    setup_syscall( state, 0x1000, NULL, 0 );
    ret = tw_syscall_dispatcher( (uintptr_t)state, 0, THUNK_RET );
    EXPECT_RETURNED( ret, state, STATUS_INVALID_SYSTEM_SERVICE );
    setup_syscall( state, 0xabcd0000ffffc000ull | syscall_id( "NtClose" ), &handle, 1 );
    ret = tw_syscall_dispatcher( (uintptr_t)state, 0, THUNK_RET );
    EXPECT_RETURNED( ret, state, STATUS_INVALID_HANDLE );
    free( state );
    return NULL;
}

/* A stub's wrapper stops loudly (section 11). Alone: it doesn't return. */
static const char *stub_stops( void )
{
    BYTE *state = new_state();

    do_syscall( state, "NtTraceEvent", NULL, 0 );
    return tw_fail( "NtTraceEvent returned" );
}

/* An x87 register: significand, then sign and exponent. */
static void set_reg( BYTE *reg, UINT64 significand, UINT16 sign_exponent )
{
    memset( reg, 0, 16 );
    memcpy( reg, &significand, 8 );
    memcpy( reg + 8, &sign_exponent, 2 );
}

/* NtContinue on the current thread: the new-context return, STATUS_UNWIND
 * even with ret = the thunk's, with every field of the CONTEXT_FULL context
 * in the state block and the flags materialized (abi.md v5, "Host
 * functions"). */
static const char *nt_continue( void )
{
    static CONTEXT ctx;
    BYTE *state = new_state();
    UINT64 args[2];
    unsigned int i;
    int32_t ret;

    memset( &ctx, 0, sizeof(ctx) );
    ctx.ContextFlags = CONTEXT_FULL;
    ctx.Rax = 0xa0; ctx.Rbx = 0xb0; ctx.Rcx = 0xc0; ctx.Rdx = 0xd0;
    ctx.Rbp = 0xbb; ctx.Rsi = 0x51; ctx.Rdi = 0xd1;
    ctx.R8 = 0x08; ctx.R9 = 0x09; ctx.R10 = 0x10; ctx.R11 = 0x11;
    ctx.R12 = 0x12; ctx.R13 = 0x13; ctx.R14 = 0x14; ctx.R15 = 0x15;
    ctx.Rsp = stack_top - 0x1000;
    ctx.Rip = 0x180002000;
    ctx.EFlags = 0x200 | 0x80 | 0x1;      /* IF, SF, CF */
    ctx.FltSave.ControlWord = 0x37f;
    ctx.FltSave.StatusWord = 3 << 11;     /* TOP = 3 */
    ctx.FltSave.TagWord = 1 << 3;         /* R3 = ST(0) holds 1.0 */
    memcpy( &ctx.FltSave.FloatRegisters[0], "\0\0\0\0\0\0\0\x80\xff\x3f", 10 );
    ctx.FltSave.MxCsr = ctx.MxCsr = 0x1fa0;
    for (i = 0; i < 16; i++) memset( &ctx.FltSave.XmmRegisters[i], 0x20 + i, 16 );

    args[0] = (uintptr_t)&ctx;
    args[1] = FALSE;
    ret = do_syscall( state, "NtContinue", args, 2 );
    TW_EXPECT( ret == TW_STATUS_UNWIND );
    TW_EXPECT( S64( state, TW_STATE_RIP ) == ctx.Rip && GPR( state, RSP ) == ctx.Rsp );
    TW_EXPECT( GPR( state, RAX ) == ctx.Rax && GPR( state, RBX ) == ctx.Rbx && GPR( state, RCX ) == ctx.Rcx );
    TW_EXPECT( GPR( state, RDX ) == ctx.Rdx && GPR( state, RBP ) == ctx.Rbp && GPR( state, RSI ) == ctx.Rsi );
    TW_EXPECT( GPR( state, RDI ) == ctx.Rdi && GPR( state, R8 ) == ctx.R8 && GPR( state, R9 ) == ctx.R9 );
    TW_EXPECT( GPR( state, R10 ) == ctx.R10 && GPR( state, R11 ) == ctx.R11 && GPR( state, R12 ) == ctx.R12 );
    TW_EXPECT( GPR( state, R13 ) == ctx.R13 && GPR( state, R14 ) == ctx.R14 && GPR( state, R15 ) == ctx.R15 );
    TW_EXPECT( S64( state, TW_STATE_RFLAGS ) == ctx.EFlags );
    TW_EXPECT( S32( state, TW_STATE_CC_OP ) == 0 && S64( state, TW_STATE_CC_A ) == ctx.EFlags );
    TW_EXPECT( S32( state, TW_STATE_MXCSR ) == 0x1fa0 );
    TW_EXPECT( S16( state, TW_STATE_FCW ) == 0x37f && S16( state, TW_STATE_FSW ) == 3 << 11 );
    TW_EXPECT( S16( state, TW_STATE_FTW ) == 0xff3f );  /* R3 valid, the rest empty */
    TW_EXPECT( !memcmp( FPR( state, 3 ), "\0\0\0\0\0\0\0\x80\xff\x3f\0\0\0\0\0", 16 ) );
    for (i = 0; i < 16; i++) TW_EXPECT( !memcmp( state + TW_STATE_XMM + 16 * i, &ctx.FltSave.XmmRegisters[i], 16 ) );
    free( state );
    return NULL;
}

/* NtGetContextThread then NtSetContextThread on the current thread, through
 * the dispatcher, with x87 and XMM state: the FP conversion of section 11
 * both ways, the abridged tags, ST(i) = R[(TOP + i) mod 8], and each full
 * tag recomputed from the contents (translator.md, "x87", Tags). */
static const char *context_round_trip( void )
{
    static CONTEXT ctx;
    static const BYTE slot_padding[6];
    BYTE *state = new_state();
    BYTE saved[TW_STATE_SIZE];
    UINT64 args[2];
    unsigned int i;
    int32_t ret;

    S16( state, TW_STATE_FCW ) = 0x27f;
    S16( state, TW_STATE_FSW ) = (5 << 11) | 0x1;  /* TOP = 5, IE */
    set_reg( FPR( state, 5 ), 0x8000000000000000ull, 0x3fff );  /* 1.0: valid */
    set_reg( FPR( state, 6 ), 0, 0x8000 );                      /* -0: zero */
    set_reg( FPR( state, 7 ), 1, 0 );                           /* a denormal: special */
    set_reg( FPR( state, 0 ), 0x8000000000000000ull, 0x7fff );  /* +inf: special */
    set_reg( FPR( state, 1 ), 0x4000000000000000ull, 0x0001 );  /* J clear: special */
    set_reg( FPR( state, 2 ), 0x1234, 0x4000 );                 /* R2-R4 empty */
    S16( state, TW_STATE_FTW ) = 0x93fa;
    S32( state, TW_STATE_MXCSR ) = 0x1fc0;                      /* DAZ */
    for (i = 0; i < 16; i++) memset( state + TW_STATE_XMM + 16 * i, 0x40 + i, 16 );
    memcpy( saved, state, TW_STATE_SIZE );

    memset( &ctx, 0xcc, sizeof(ctx) );
    ctx.ContextFlags = CONTEXT_FULL;
    args[0] = (uintptr_t)GetCurrentThread();
    args[1] = (uintptr_t)&ctx;
    ret = do_syscall( state, "NtGetContextThread", args, 2 );
    EXPECT_RETURNED( ret, state, STATUS_SUCCESS );
    TW_EXPECT( (ctx.ContextFlags & CONTEXT_FULL) == CONTEXT_FULL );
    TW_EXPECT( ctx.Rip == THUNK_RET - 0xb && ctx.Rsp == stack_top - 0x200 + 8 );
    TW_EXPECT( ctx.EFlags == 0x202 && ctx.SegCs == 0x33 && ctx.SegSs == 0x2b );
    TW_EXPECT( ctx.Rbx == GPR( saved, RBX ) && ctx.Rbp == GPR( saved, RBP ) && ctx.Rsi == GPR( saved, RSI ) );
    TW_EXPECT( ctx.Rdi == GPR( saved, RDI ) && ctx.R12 == GPR( saved, R12 ) && ctx.R15 == GPR( saved, R15 ) );
    TW_EXPECT( ctx.FltSave.ControlWord == 0x27f && ctx.FltSave.StatusWord == ((5 << 11) | 0x1) );
    TW_EXPECT( ctx.FltSave.TagWord == 0xe3 );  /* R0, R1, R5, R6, R7 */
    TW_EXPECT( !ctx.FltSave.Reserved1 && !ctx.FltSave.ErrorOpcode && !ctx.FltSave.ErrorOffset );
    TW_EXPECT( !ctx.FltSave.DataOffset && !ctx.FltSave.ErrorSelector && !ctx.FltSave.DataSelector );
    TW_EXPECT( ctx.FltSave.MxCsr == 0x1fc0 && ctx.MxCsr == 0x1fc0 && ctx.FltSave.MxCsr_Mask == TW_MXCSR_MASK );
    for (i = 0; i < 8; i++)
    {
        const BYTE *st = (const BYTE *)&ctx.FltSave.FloatRegisters[i];

        if (memcmp( st, FPR( saved, (5 + i) & 7 ), 10 ) || memcmp( st + 10, slot_padding, 6 ))
            return tw_fail( "ST(%u) is not R%u", i, (5 + i) & 7 );
    }
    for (i = 0; i < 16; i++) TW_EXPECT( !memcmp( &ctx.FltSave.XmmRegisters[i], saved + TW_STATE_XMM + 16 * i, 16 ) );

    /* clobber the FP state, then set it back from the context */
    memset( state + TW_STATE_FPR, 0x77, 8 * 16 );
    memset( state + TW_STATE_XMM, 0x77, 16 * 16 );
    S16( state, TW_STATE_FTW ) = 0;
    S16( state, TW_STATE_FSW ) = 0;
    S32( state, TW_STATE_MXCSR ) = 0x1f80;
    ctx.ContextFlags = CONTEXT_FLOATING_POINT;
    args[1] = (uintptr_t)&ctx;
    ret = do_syscall( state, "NtSetContextThread", args, 2 );
    EXPECT_RETURNED( ret, state, STATUS_SUCCESS );  /* no CONTROL or INTEGER: a plain return */
    TW_EXPECT( S16( state, TW_STATE_FCW ) == 0x27f && S16( state, TW_STATE_FSW ) == ((5 << 11) | 0x1) );
    if (S16( state, TW_STATE_FTW ) != 0x93fa) return tw_fail( "FTW %#x, expected 0x93fa", S16( state, TW_STATE_FTW ) );
    TW_EXPECT( S32( state, TW_STATE_MXCSR ) == 0x1fc0 );
    TW_EXPECT( !memcmp( state + TW_STATE_FPR, saved + TW_STATE_FPR, 8 * 16 ) );
    TW_EXPECT( !memcmp( state + TW_STATE_XMM, saved + TW_STATE_XMM, 16 * 16 ) );
    free( state );
    return NULL;
}

/* A unix call: ntdll's system_time_precise (unix_call_funcs[7], loader.c
 * L1141-1150) through the handle the Unix side stores. The return address
 * is popped unadjusted, the flags stay lazy (abi.md v5). */
static const char *unix_call( void )
{
    static LONGLONG time;
    BYTE *state = new_state();
    UINT64 rsp = stack_top - 0x200;
    int32_t ret;

    *(UINT64 *)rsp = THUNK_RET;
    GPR( state, RSP ) = rsp;
    GPR( state, RCX ) = (uintptr_t)tw_unix_call_funcs;
    GPR( state, RDX ) = 0xffffffff00000007ull;  /* EDX: the code */
    GPR( state, R8 ) = (uintptr_t)&time;
    S32( state, TW_STATE_CC_OP ) = 5;           /* a lazy record the call must keep */
    ret = tw_unix_call_dispatcher( (uintptr_t)state, 0, THUNK_RET );
    EXPECT_RETURNED( ret, state, STATUS_SUCCESS );
    TW_EXPECT( S32( state, TW_STATE_CC_OP ) == 5 );
    TW_EXPECT( llabs( (long long)(time - now_ticks()) ) < 10000000 );

    *(UINT64 *)(rsp + 8) = THUNK_RET;
    GPR( state, RSP ) = rsp;
    ret = tw_unix_call_dispatcher( (uintptr_t)state, 0, CALLER_RET );
    TW_EXPECT( ret == TW_STATUS_UNWIND && S64( state, TW_STATE_RIP ) == THUNK_RET );
    free( state );
    return NULL;
}

/* A code past ntdll's unix calls stops loudly (abi.md v5). Alone. */
static const char *unix_call_bad_code( void )
{
    BYTE *state = new_state();
    UINT64 rsp = stack_top - 0x200;

    *(UINT64 *)rsp = THUNK_RET;
    GPR( state, RSP ) = rsp;
    GPR( state, RCX ) = (uintptr_t)tw_unix_call_funcs;
    GPR( state, RDX ) = tw_unix_call_count;
    tw_unix_call_dispatcher( (uintptr_t)state, 0, THUNK_RET );
    return tw_fail( "the call returned" );
}

/* ---- table 1 and win32u's unix calls (section 11) ---- */

/* win32u's unix library, as find_builtin_dll records its path. */
#define WIN32U_SO "/wine/lib/wine/x86_64-unix/win32u.so"
#define TABLE1_GET_MOUSE_MOVE_POINTS 0x141f  /* NtUserGetMouseMovePointsEx (win32syscalls.h L2601) */

/* A unix call to win32u's table, through the handle tw_unixlib_funcs gives
 * (the one get_builtin_unix_funcs stores, patch 0107). */
static int32_t win32u_unix_call( BYTE *state, UINT64 code, void *args, UINT64 ret_addr )
{
    UINT64 rsp = stack_top - 0x200;

    *(UINT64 *)rsp = THUNK_RET;
    GPR( state, RSP ) = rsp;
    GPR( state, RCX ) = (uintptr_t)tw_unixlib_funcs( WIN32U_SO, 0 );
    GPR( state, RDX ) = code;
    GPR( state, R8 ) = (uintptr_t)args;
    return tw_unix_call_dispatcher( (uintptr_t)state, 0, ret_addr );
}

/* NtUserGetMouseMovePointsEx with a wrong size (win32u/input.c L1570-1574):
 * five arguments, the fifth on the stack, and an int -1 with
 * ERROR_INVALID_PARAMETER as the last error. */
static int32_t table1_call( BYTE *state )
{
    static UINT64 in[3], out[12];  /* room for MOUSEMOVEPOINTs (24 bytes each) */
    UINT64 args[5] = { 1, (uintptr_t)in, (uintptr_t)out, 4, 1 /* GMMP_USE_DISPLAY_POINTS */ };

    setup_syscall( state, TABLE1_GET_MOUSE_MOVE_POINTS, args, 5 );
    return tw_syscall_dispatcher( (uintptr_t)state, 0, THUNK_RET );
}

/* Before win32u's init registers table 1 with KeAddSystemServiceTable, its
 * limit is 0: a call through its wrapper returns
 * STATUS_INVALID_SYSTEM_SERVICE, as Wine's dispatcher does (L3145-3146,
 * L3324-3327). So does table 2, which nothing registers. */
static const char *table1_before_init( void )
{
    BYTE *state = new_state();
    int32_t ret;

    TW_EXPECT( KeServiceDescriptorTable[1].ServiceLimit == 0 );
    NtCurrentTeb()->LastErrorValue = 0;
    ret = table1_call( state );
    EXPECT_RETURNED( ret, state, STATUS_INVALID_SYSTEM_SERVICE );
    TW_EXPECT( NtCurrentTeb()->LastErrorValue == 0 );
    setup_syscall( state, 0x2000, NULL, 0 );
    ret = tw_syscall_dispatcher( (uintptr_t)state, 0, THUNK_RET );
    EXPECT_RETURNED( ret, state, STATUS_INVALID_SYSTEM_SERVICE );
    free( state );
    return NULL;
}

/* A unix call to win32u's table: code 0, init (win32u/syscall.c L92-111),
 * which registers table 1 (unix/loader.c L625-634) with the generated
 * table's size. The return address is popped unadjusted and the flags stay
 * lazy, as for ntdll's (abi.md v5). */
static const char *win32u_unix_call_init( void )
{
    const void *funcs = tw_unixlib_funcs( WIN32U_SO, 0 );
    BYTE *state = new_state();
    int32_t ret;

    TW_EXPECT( funcs && funcs != (const void *)tw_unix_call_funcs );
    TW_EXPECT( tw_unixlib_count( funcs ) == 1 );
    TW_EXPECT( tw_unixlib_count( tw_unix_call_funcs ) == 0 );
    TW_EXPECT( tw_unixlib_funcs( "win32u.so", 0 ) == funcs );
    S32( state, TW_STATE_CC_OP ) = 5;
    ret = win32u_unix_call( state, 0xffffffff00000000ull, NULL, THUNK_RET );  /* EDX: code 0 */
    EXPECT_RETURNED( ret, state, STATUS_SUCCESS );
    TW_EXPECT( S32( state, TW_STATE_CC_OP ) == 5 );
    TW_EXPECT( KeServiceDescriptorTable[1].ServiceLimit == tw_win32_syscall_count );
    free( state );
    return NULL;
}

/* After the registration the same call reaches its wrapper: RAX is the int
 * -1 zero-extended (section 11), the last error is Wine's. A number at the
 * limit is still a bad id. */
static const char *table1_after_init( void )
{
    BYTE *state = new_state();
    int32_t ret;

    TW_EXPECT( KeServiceDescriptorTable[1].ServiceLimit == tw_win32_syscall_count );
    NtCurrentTeb()->LastErrorValue = 0;
    ret = table1_call( state );
    EXPECT_RETURNED( ret, state, 0xffffffff );
    TW_EXPECT( GPR( state, RAX ) == 0xffffffffull );
    TW_EXPECT( NtCurrentTeb()->LastErrorValue == ERROR_INVALID_PARAMETER );
    TW_EXPECT( S64( state, TW_STATE_RIP ) == THUNK_RET - 0xb );
    setup_syscall( state, 0x1000 + tw_win32_syscall_count, NULL, 0 );
    ret = tw_syscall_dispatcher( (uintptr_t)state, 0, THUNK_RET );
    EXPECT_RETURNED( ret, state, STATUS_INVALID_SYSTEM_SERVICE );
    free( state );
    return NULL;
}

/* A table-1 stub's wrapper stops loudly (section 11): NtBindCompositionSurface,
 * id 0x1000, once table 1 is registered. Alone. */
static const char *table1_stub_stops( void )
{
    BYTE *state = new_state();

    if (win32u_unix_call( state, 0, NULL, THUNK_RET ) != TW_STATUS_RETURNED || GPR( state, RAX ))
        return tw_fail( "win32u's init failed" );
    setup_syscall( state, 0x1000, NULL, 0 );
    tw_syscall_dispatcher( (uintptr_t)state, 0, THUNK_RET );
    return tw_fail( "NtBindCompositionSurface returned" );
}

/* A code past win32u's one unix call stops loudly. Alone. */
static const char *win32u_unix_call_bad_code( void )
{
    BYTE *state = new_state();

    win32u_unix_call( state, 1, NULL, THUNK_RET );
    return tw_fail( "the call returned" );
}

/* A syscall's entry refreshes KUSER's clock (section 11, step 1). */
static const char *kuser_clock( void )
{
    KUSER_SHARED_DATA *kuser = (KUSER_SHARED_DATA *)user_shared_data;
    BYTE *state = new_state();
    int32_t ret;

    memset( (void *)&kuser->SystemTime, 0, sizeof(kuser->SystemTime) );
    memset( (void *)&kuser->InterruptTime, 0, sizeof(kuser->InterruptTime) );
    setup_syscall( state, tw_syscall_count, NULL, 0 );
    ret = tw_syscall_dispatcher( (uintptr_t)state, 0, THUNK_RET );
    EXPECT_RETURNED( ret, state, STATUS_INVALID_SYSTEM_SERVICE );
    TW_EXPECT( llabs( (long long)(kuser_system_time() - now_ticks()) ) < 10000000 );
    TW_EXPECT( kuser->InterruptTime.LowPart || kuser->InterruptTime.High1Time );
    free( state );
    return NULL;
}

/* NtClose of a bad handle, for the +syscall lines: the JS test runs it
 * alone with WINEDEBUG=+syscall and reads stderr. */
static const char *syscall_trace( void )
{
    BYTE *state = new_state();
    UINT64 handle = 0xdead;
    int32_t ret;

    TW_EXPECT( ntdll_get_thread_data()->syscall_trace == TRACE_ON(syscall) );
    ret = do_syscall( state, "NtClose", &handle, 1 );
    EXPECT_RETURNED( ret, state, STATUS_INVALID_HANDLE );
    free( state );
    return NULL;
}

/* Process exit's ends (section 8), each alone. abort_thread of the last
 * thread is Wine's end for errors (unix/thread.c L1477): abort_process keeps
 * its _exit, so the JS test sees Emscripten's exit status 1 and no
 * tempwind: line. */
static const char *abort_thread_exits( void )
{
    abort_thread( 1 );
    return tw_fail( "abort_thread returned" );
}

/* NtTerminateProcess of the current process without RtlExitUserProcess's
 * first call (unix/process.c L982) reaches tw_process_exit, which stops
 * loudly outside boot. Without the hook it would end in _exit. */
static const char *terminate_reaches_exit_hook( void )
{
    BYTE *state = new_state();
    UINT64 args[2] = { (UINT64)(ULONG_PTR)NtCurrentProcess(), 0x25 };

    do_syscall( state, "NtTerminateProcess", args, 2 );
    return tw_fail( "NtTerminateProcess returned" );
}

static const struct tw_check checks[] =
{
    { "dispatcher_page", dispatcher_page },
    { "stack_arguments", stack_arguments },
    { "variant_entries", variant_entries },
    { "bad_id", bad_id },
    { "nt_continue", nt_continue },
    { "context_round_trip", context_round_trip },
    { "unix_call", unix_call },
    { "kuser_clock", kuser_clock },
    { "syscall_trace", syscall_trace },
    { "table1_before_init", table1_before_init },
    { "win32u_unix_call_init", win32u_unix_call_init },
    { "table1_after_init", table1_after_init },
    { "stub_stops", stub_stops, TW_ALONE },
    { "unix_call_bad_code", unix_call_bad_code, TW_ALONE },
    { "table1_stub_stops", table1_stub_stops, TW_ALONE },
    { "win32u_unix_call_bad_code", win32u_unix_call_bad_code, TW_ALONE },
    { "abort_thread_exits", abort_thread_exits, TW_ALONE },
    { "terminate_reaches_exit_hook", terminate_reaches_exit_hook, TW_ALONE },
};

int tw_test_main( void )
{
    void *stack = NULL;
    SIZE_T size = 0x10000;

    tw_process_start();
    if (NtAllocateVirtualMemory( NtCurrentProcess(), &stack, 0, &size, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE ))
    {
        printf( "not ok setup: no guest stack\n" );
        return 1;
    }
    stack_top = (uintptr_t)stack + size;
    return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
}

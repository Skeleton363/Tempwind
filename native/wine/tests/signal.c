/*
 * The `signal` checks (docs/design/wine-unix.md v3, section 10, "The M4
 * checks"): SIGUSR1 delivered at the wait (section 9, "Signals"), suspension
 * and another thread's context through the server (section 11), the
 * per-thread mask, a suspended start, and a user APC in an alertable wait.
 *
 * No guest code, as in thread.c: the harness sets tw_thread_start_hook, so
 * signal_start_thread runs a C routine of this file in place of the thread's
 * run loop, after it has built the thread's state block. That state block is
 * the thread's frame record while the routine runs, so the routine's Nt calls
 * are "inside a syscall" with the state block as the guest's registers.
 *
 * The setup is thread.c's, plus what makes the server send SIGUSR1: the
 * init_process_done request of server_init_process_done (unix/server.c
 * L1758-1765), since stop_thread signals only once process init is done
 * (server/thread.c L955-961), and the main thread's unblock of
 * server_block_set that signal_start_thread would do (signal_x86_64.c
 * L2978). pKiUserApcDispatcher is a sentinel: nothing runs there.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include "config.h"

#include <signal.h>
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
#include "unix_private.h"
#include "wine/server.h"
#include "wine/debug.h"

#include "harness.h"
#include "tempwind/abi.h"
#include "tempwind_private.h"

WINE_DECLARE_DEBUG_CHANNEL(syscall);

#define MS 10000  /* ticks of 100 ns */
#define S64(s, off) (*(UINT64 *)((BYTE *)(s) + (off)))
#define S32(s, off) (*(UINT32 *)((BYTE *)(s) + (off)))
#define GPR(s, i)   S64( s, TW_STATE_GPR(i) )
enum { RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI, R8, R9, R10, R11, R12, R13, R14, R15 };
#define SIG_BIT(sig) ((uint64_t)1 << ((sig) - 1))

/* signal_x86_64.c's struct apc_stack_layout (L432-444): what
 * call_user_apc_dispatcher writes below the thread's RSP. */
struct machine_frame { ULONG64 rip, cs, eflags, rsp, ss; };
struct apc_stack_layout
{
    CONTEXT              context;        /* 000 */
    CONTEXT_EX           context_ex;     /* 4d0 */
    KCONTINUE_ARGUMENT   continue_arg;   /* 4f0 */
    ULONG64              align1;         /* 508 */
    BYTE                 callback_data[0x20];  /* 510, APC_CALLBACK_DATA */
    struct machine_frame machine_frame;  /* 530 */
    ULONG64              align2;         /* 558 */
};
_Static_assert( offsetof(struct apc_stack_layout, continue_arg) == 0x4f0, "layout" );
_Static_assert( offsetof(struct apc_stack_layout, machine_frame) == 0x530, "layout" );
_Static_assert( sizeof(struct apc_stack_layout) == 0x560, "layout" );

#define APC_DISPATCHER 0x7ffe1234  /* the sentinel pKiUserApcDispatcher */
#define APC_FUNC(n)    (0x7ffe2000 + (n))

/* What a harness thread gets and reports. */
struct targ
{
    BYTE *state;              /* its state block, from signal_start_thread */
    int green_id;
    void *ctx;                /* RCX of its start: the CONTEXT LdrInitializeThunk gets */
    HANDLE event;
    volatile int flag, slept, after;
    ULONG prev;               /* NtSuspendThread's previous count, on itself */
    NTSTATUS status;
    const char *fail;
    char buf[512];
};

static void thread_hook( void *entry, void *arg, void *state )
{
    struct targ *a = arg;

    a->state = state;
    a->green_id = pthread_getthreadid_np();
    a->ctx = (void *)(ULONG_PTR)GPR( state, RCX );
    ((PRTL_THREAD_START_ROUTINE)entry)( arg );
    NtTerminateThread( NtCurrentThread(), 0 );
}

static const char *thread_fail( struct targ *a, const char *fmt, ... )
{
    va_list args;

    if (a->fail) return a->fail;
    va_start( args, fmt );
    vsnprintf( a->buf, sizeof(a->buf), fmt, args );
    va_end( args );
    return a->fail = a->buf;
}

#define T_EXPECT(a, cond) do { if (!(cond)) thread_fail( a, "%s:%d: %s", __FILE__, __LINE__, #cond ); } while (0)

static HANDLE spawn( void (WINAPI *routine)( void * ), struct targ *a, ULONG flags )
{
    HANDLE handle = 0;

    if (NtCreateThreadEx( &handle, THREAD_ALL_ACCESS, NULL, NtCurrentProcess(),
                          (PRTL_THREAD_START_ROUTINE)routine, a, flags, 0, 0, 0, NULL ))
        return 0;
    return handle;
}

static HANDLE new_event( void )
{
    HANDLE event = 0;

    if (NtCreateEvent( &event, EVENT_ALL_ACCESS, NULL, NotificationEvent, FALSE )) return 0;
    return event;
}

static NTSTATUS wait_ms( HANDLE handle, LONGLONG ms )
{
    LARGE_INTEGER timeout = { .QuadPart = -ms * MS };

    return NtWaitForSingleObject( handle, FALSE, &timeout );
}

static void sleep_ms( LONGLONG ms )
{
    LARGE_INTEGER timeout = { .QuadPart = -ms * MS };

    NtDelayExecution( FALSE, &timeout );
}

/* Yields until the thread has set its flag and then waits (its green thread
 * has a waiter): it is in the wait it entered after the flag. */
static int until_waiting( struct targ *a )
{
    int i;

    for (i = 0; i < 20 && !a->flag; i++) NtYieldExecution();
    for (i = 0; i < 20 && a->flag && !(green_thread_by_id( a->green_id ) &&
                                      green_thread_by_id( a->green_id )->waiter); i++)
        NtYieldExecution();
    return a->flag && green_thread_by_id( a->green_id ) && green_thread_by_id( a->green_id )->waiter;
}

/* Ends a check: the thread's handle signaled, its own failure if any. */
static const char *joined( HANDLE thread, struct targ *a )
{
    NTSTATUS status = wait_ms( thread, 10000 );

    NtClose( thread );
    if (status) return tw_fail( "the thread didn't end: %#x", (unsigned)status );
    return a->fail;
}

/* ---- NtSuspendThread of another thread ---- */

static void WINAPI waits_on_event( void *arg )
{
    struct targ *a = arg;

    a->flag = 1;
    a->status = NtWaitForSingleObject( a->event, FALSE, NULL );  /* a server wait */
    a->after = 1;
}

/* A thread in a server wait gets SIGUSR1 (stop_thread), stops in
 * wait_suspend, and stays stopped while its count is above 0, though its
 * wait is satisfied; the counts are the server's. */
static const char *suspend_other( void )
{
    struct targ a = { 0 };
    HANDLE thread;
    ULONG prev = 99;

    TW_EXPECT( (a.event = new_event()) );
    TW_EXPECT( (thread = spawn( waits_on_event, &a, 0 )) );
    TW_EXPECT( until_waiting( &a ) );
    TW_EXPECT( !NtSuspendThread( thread, &prev ) && prev == 0 );
    TW_EXPECT( green_thread_by_id( a.green_id )->sig_pending & SIG_BIT( SIGUSR1 ) );
    TW_EXPECT( !NtSuspendThread( thread, &prev ) && prev == 1 );
    TW_EXPECT( !NtSetEvent( a.event, NULL ) );
    TW_EXPECT( wait_ms( thread, 100 ) == STATUS_TIMEOUT && !a.after );
    /* the handler ran and the thread waits in wait_suspend */
    TW_EXPECT( !(green_thread_by_id( a.green_id )->sig_pending & SIG_BIT( SIGUSR1 )) );
    TW_EXPECT( green_thread_by_id( a.green_id )->waiter );
    TW_EXPECT( !NtResumeThread( thread, &prev ) && prev == 2 );
    TW_EXPECT( wait_ms( thread, 100 ) == STATUS_TIMEOUT && !a.after );
    TW_EXPECT( !NtResumeThread( thread, &prev ) && prev == 1 );
    if (joined( thread, &a )) return a.fail ? a.fail : tw_fail( "the thread didn't end" );
    TW_EXPECT( a.after && a.status == STATUS_WAIT_0 );
    NtClose( a.event );
    return NULL;
}

/* ---- a self-suspend ---- */

static void WINAPI suspends_itself( void *arg )
{
    struct targ *a = arg;
    UINT64 before[16];
    int i;

    for (i = 0; i < 16; i++) before[i] = GPR( a->state, i );
    a->flag = 1;
    a->status = NtSuspendThread( NtCurrentThread(), &a->prev );
    a->after = 1;
    /* the handler put back the context it took */
    for (i = 0; i < 16; i++) T_EXPECT( a, GPR( a->state, i ) == before[i] );
}

/* NtSuspendThread of the current thread: the server sends SIGUSR1 to the
 * thread itself, which handles it when wine_server_call unblocks it
 * (delivery point 2) and stops in wait_suspend until another thread resumes
 * it. */
static const char *self_suspend( void )
{
    struct targ a = { 0 };
    HANDLE thread;
    ULONG prev = 99;
    int i;

    TW_EXPECT( (thread = spawn( suspends_itself, &a, 0 )) );
    for (i = 0; i < 20 && !a.flag; i++) NtYieldExecution();
    TW_EXPECT( a.flag );
    sleep_ms( 50 );
    TW_EXPECT( !a.after );
    TW_EXPECT( green_thread_by_id( a.green_id )->waiter );
    TW_EXPECT( !NtResumeThread( thread, &prev ) && prev == 1 );
    if (joined( thread, &a )) return a.fail ? a.fail : tw_fail( "the thread didn't end" );
    TW_EXPECT( a.after && a.status == STATUS_SUCCESS && a.prev == 0 );
    return NULL;
}

/* ---- another thread's context ---- */

static void seed( BYTE *state, UINT64 salt )
{
    int i;

    for (i = 0; i < 16; i++)
        if (i != RSP) GPR( state, i ) = 0x0101010101010101ull * (i + 1) ^ salt;
    for (i = 0; i < 16; i++)
    {
        S64( state, TW_STATE_XMM + 16 * i ) = 0x1111000000000000ull + 0x100 * i + salt;
        S64( state, TW_STATE_XMM + 16 * i + 8 ) = ~(UINT64)i ^ salt;
    }
    S32( state, TW_STATE_MXCSR ) = 0x1fa0;
    S64( state, TW_STATE_RFLAGS ) = 0x246;
}

static const char *same_as_state( const CONTEXT *c, const BYTE *state )
{
    const UINT64 *regs = &c->Rax;  /* Rax, Rcx, Rdx, Rbx, Rsp, Rbp, Rsi, Rdi, R8-R15: the state's order */
    int i;

    for (i = 0; i < 16; i++)
        if (regs[i] != GPR( state, i )) return tw_fail( "GPR %d: %#llx, the state block has %#llx", i,
                                                         (unsigned long long)regs[i], (unsigned long long)GPR( state, i ) );
    if (c->Rip != S64( state, TW_STATE_RIP )) return tw_fail( "RIP %#llx", (unsigned long long)c->Rip );
    if (c->EFlags != (DWORD)S64( state, TW_STATE_RFLAGS )) return tw_fail( "EFlags %#x", (unsigned)c->EFlags );
    if (c->MxCsr != S32( state, TW_STATE_MXCSR )) return tw_fail( "MxCsr %#x", (unsigned)c->MxCsr );
    if (memcmp( c->FltSave.XmmRegisters, state + TW_STATE_XMM, sizeof(c->FltSave.XmmRegisters) ))
        return tw_fail( "the XMM registers differ" );
    return NULL;
}

static void WINAPI waits_seeded( void *arg )
{
    struct targ *a = arg;

    seed( a->state, 0 );
    waits_on_event( arg );
}

/* NtGetContextThread of a suspended thread in a server wait returns its
 * state block as the handler posted it; NtSetContextThread changes what the
 * server keeps, a second Get returns that, and the thread resumes with it in
 * its state block (the handler's NtSetContextThread). */
static const char *context_other( void )
{
    struct targ a = { 0 };
    HANDLE thread;
    CONTEXT c, d;
    BYTE want[TW_STATE_SIZE];
    const char *err;
    int i;

    TW_EXPECT( (a.event = new_event()) );
    TW_EXPECT( (thread = spawn( waits_seeded, &a, 0 )) );
    TW_EXPECT( until_waiting( &a ) );
    TW_EXPECT( !NtSuspendThread( thread, NULL ) );

    memset( &c, 0, sizeof(c) );
    c.ContextFlags = CONTEXT_FULL;
    TW_EXPECT( !NtGetContextThread( thread, &c ) );
    TW_EXPECT( (c.ContextFlags & CONTEXT_FULL) == CONTEXT_FULL );
    if ((err = same_as_state( &c, a.state ))) return err;

    /* new values for every GPR but RSP, the XMM registers, MXCSR and EFLAGS */
    memcpy( want, a.state, TW_STATE_SIZE );
    seed( want, 0x5a5a5a5a00000000ull );
    S64( want, TW_STATE_RFLAGS ) = 0x283;
    d = c;
    for (i = 0; i < 16; i++) (&d.Rax)[i] = GPR( want, i );
    memcpy( d.FltSave.XmmRegisters, want + TW_STATE_XMM, sizeof(d.FltSave.XmmRegisters) );
    d.MxCsr = d.FltSave.MxCsr = S32( want, TW_STATE_MXCSR );
    d.EFlags = S64( want, TW_STATE_RFLAGS );
    d.ContextFlags = CONTEXT_FULL;
    TW_EXPECT( !NtSetContextThread( thread, &d ) );

    memset( &c, 0, sizeof(c) );
    c.ContextFlags = CONTEXT_FULL;
    TW_EXPECT( !NtGetContextThread( thread, &c ) );
    if ((err = same_as_state( &c, want ))) return err;

    TW_EXPECT( !NtSetEvent( a.event, NULL ) );
    TW_EXPECT( !NtResumeThread( thread, NULL ) );
    if (joined( thread, &a )) return a.fail ? a.fail : tw_fail( "the thread didn't end" );
    /* the thread's state block now holds what the other thread set */
    if ((err = same_as_state( &c, a.state ))) return err;
    NtClose( a.event );
    return NULL;
}

/* ---- the mask ---- */

static void WINAPI blocks_then_sleeps( void *arg )
{
    struct targ *a = arg;
    sigset_t set;

    sigemptyset( &set );
    sigaddset( &set, SIGUSR1 );
    pthread_sigmask( SIG_BLOCK, &set, NULL );
    a->flag = 1;
    sleep_ms( 100 );  /* non-alertable: a sleep in select, with SIGUSR1 blocked */
    a->slept = 1;
    pthread_sigmask( SIG_UNBLOCK, &set, NULL );  /* delivery point 2: stops here */
    a->after = 1;
}

/* A SIGUSR1 sent to a thread that blocks it stays pending and doesn't end
 * its sleep; the thread's unblock handles it, and it stops there until it is
 * resumed. */
static const char *mask_blocks_sigusr1( void )
{
    struct targ a = { 0 };
    HANDLE thread;
    sigset_t set;

    TW_EXPECT( (thread = spawn( blocks_then_sleeps, &a, 0 )) );
    TW_EXPECT( until_waiting( &a ) );
    TW_EXPECT( !NtSuspendThread( thread, NULL ) );
    TW_EXPECT( green_thread_by_id( a.green_id )->sig_pending & SIG_BIT( SIGUSR1 ) );
    TW_EXPECT( green_thread_by_id( a.green_id )->sig_blocked & SIG_BIT( SIGUSR1 ) );
    NtYieldExecution();
    TW_EXPECT( green_thread_by_id( a.green_id )->waiter && !a.slept );  /* still asleep */
    sleep_ms( 300 );
    TW_EXPECT( a.slept && !a.after );
    TW_EXPECT( !(green_thread_by_id( a.green_id )->sig_pending & SIG_BIT( SIGUSR1 )) );
    TW_EXPECT( !NtResumeThread( thread, NULL ) );
    if (joined( thread, &a )) return a.fail ? a.fail : tw_fail( "the thread didn't end" );
    TW_EXPECT( a.after );
    /* this thread's own mask didn't change */
    pthread_sigmask( SIG_BLOCK, NULL, &set );
    TW_EXPECT( !sigismember( &set, SIGUSR1 ) );
    return NULL;
}

/* ---- a suspended start ---- */

static void WINAPI starts( void *arg )
{
    struct targ *a = arg;

    a->flag = 1;
}

/* NtCreateThreadEx with THREAD_CREATE_FLAGS_CREATE_SUSPENDED: the thread
 * waits in wait_suspend before its first instruction (signal_x86_64.c
 * L2957); another thread reads its start context and changes it, and the
 * CONTEXT LdrInitializeThunk gets and the state block carry the change. */
static const char *suspended_start( void )
{
    struct targ a = { 0 };
    HANDLE thread;
    CONTEXT c;
    ULONG prev = 99;

    TW_EXPECT( (thread = spawn( starts, &a, THREAD_CREATE_FLAGS_CREATE_SUSPENDED )) );
    memset( &c, 0, sizeof(c) );
    c.ContextFlags = CONTEXT_FULL;
    TW_EXPECT( !NtGetContextThread( thread, &c ) );  /* waits until the start posts it */
    TW_EXPECT( !a.flag && !a.state );
    TW_EXPECT( c.Rcx == (ULONG_PTR)starts && c.Rdx == (ULONG_PTR)&a );
    TW_EXPECT( c.Rip == (ULONG_PTR)pRtlUserThreadStart && c.EFlags == 0x200 );
    c.R12 = 0x1234567887654321ull;
    TW_EXPECT( !NtSetContextThread( thread, &c ) );
    NtYieldExecution();
    TW_EXPECT( !a.flag );
    TW_EXPECT( !NtResumeThread( thread, &prev ) && prev == 1 );
    if (joined( thread, &a )) return a.fail ? a.fail : tw_fail( "the thread didn't end" );
    TW_EXPECT( a.flag && a.state && a.ctx );
    TW_EXPECT( ((CONTEXT *)a.ctx)->R12 == 0x1234567887654321ull && ((CONTEXT *)a.ctx)->Rdx == (ULONG_PTR)&a );
    TW_EXPECT( GPR( a.state, R12 ) == 0x1234567887654321ull && GPR( a.state, RCX ) == (ULONG_PTR)a.ctx );
    return NULL;
}

/* ---- a user APC ---- */

/* What call_user_apc_dispatcher leaves for APC n queued with (n, 2n, 3n):
 * the state block enters the sentinel KiUserApcDispatcher with RSP = the
 * layout below the wait's RSP, which holds the wait's context with RAX =
 * STATUS_USER_APC, the APC in P1Home-P4Home and the continue argument. */
static void check_apc( struct targ *a, const BYTE *before, ULONG_PTR n )
{
    struct apc_stack_layout *stack = (struct apc_stack_layout *)(GPR( before, RSP ) & ~15) - 1;
    int i;

    T_EXPECT( a, S64( a->state, TW_STATE_RIP ) == APC_DISPATCHER );
    T_EXPECT( a, GPR( a->state, RSP ) == (ULONG_PTR)stack );
    T_EXPECT( a, GPR( a->state, RBP ) == GPR( before, RBP ) );
    T_EXPECT( a, stack->context.Rip == S64( before, TW_STATE_RIP ) && stack->context.Rsp == GPR( before, RSP ) );
    T_EXPECT( a, stack->context.Rax == STATUS_USER_APC );
    for (i = 1; i < 16; i++) T_EXPECT( a, i == RSP || (&stack->context.Rax)[i] == GPR( before, i ) );
    T_EXPECT( a, stack->context.P1Home == n && stack->context.P2Home == 2 * n && stack->context.P3Home == 3 * n );
    T_EXPECT( a, stack->context.P4Home == APC_FUNC( n ) );
    T_EXPECT( a, stack->continue_arg.ContinueType == KCONTINUE_RESUME );
    T_EXPECT( a, stack->continue_arg.ContinueFlags == (KCONTINUE_FLAG_TEST_ALERT | KCONTINUE_FLAG_DELIVER_APC) );
    T_EXPECT( a, stack->machine_frame.rip == stack->context.Rip && stack->machine_frame.rsp == stack->context.Rsp );
}

static void WINAPI waits_alertable( void *arg )
{
    struct targ *a = arg;
    BYTE before[TW_STATE_SIZE];
    struct apc_stack_layout *stack;
    NTSTATUS status;
    int i;

    seed( a->state, 0x77 );
    memcpy( before, a->state, TW_STATE_SIZE );
    a->flag = 1;
    a->status = NtWaitForSingleObject( a->event, TRUE, NULL );
    T_EXPECT( a, a->status == STATUS_USER_APC );
    check_apc( a, before, 1 );
    /* what PE KiUserApcDispatcher does after calling the APC: NtContinueEx
     * with the layout's context, which delivers the next APC... */
    stack = (struct apc_stack_layout *)GPR( a->state, RSP );
    status = NtContinueEx( &stack->context, &stack->continue_arg );
    T_EXPECT( a, status == STATUS_USER_APC );
    check_apc( a, before, 2 );
    /* ...and with none left, resumes the wait's context */
    stack = (struct apc_stack_layout *)GPR( a->state, RSP );
    status = NtContinueEx( &stack->context, &stack->continue_arg );
    T_EXPECT( a, status == STATUS_SUCCESS );
    T_EXPECT( a, S64( a->state, TW_STATE_RIP ) == S64( before, TW_STATE_RIP ) );
    T_EXPECT( a, GPR( a->state, RAX ) == STATUS_USER_APC );
    for (i = 1; i < 16; i++) T_EXPECT( a, GPR( a->state, i ) == GPR( before, i ) );
    a->after = 1;
}

/* An alertable server wait ends with STATUS_USER_APC and the APC's
 * dispatch (call_user_apc_dispatcher); NtContinueEx delivers the second
 * queued APC, then restores the wait's context. */
static const char *apc_in_alertable_wait( void )
{
    struct targ a = { 0 };
    HANDLE thread;
    ULONG_PTR n;

    pKiUserApcDispatcher = (void *)APC_DISPATCHER;
    TW_EXPECT( (a.event = new_event()) );
    TW_EXPECT( (thread = spawn( waits_alertable, &a, 0 )) );
    TW_EXPECT( until_waiting( &a ) );
    for (n = 1; n <= 2; n++)
        TW_EXPECT( !NtQueueApcThread( thread, (PNTAPCFUNC)APC_FUNC( n ), n, 2 * n, 3 * n ) );
    if (joined( thread, &a )) return a.fail ? a.fail : tw_fail( "the thread didn't end" );
    TW_EXPECT( a.after );
    NtClose( a.event );
    return NULL;
}

static const struct tw_check checks[] =
{
    { "suspend_other", suspend_other },
    { "self_suspend", self_suspend },
    { "context_other", context_other },
    { "mask_blocks_sigusr1", mask_blocks_sigusr1 },
    { "suspended_start", suspended_start },
    { "apc_in_alertable_wait", apc_in_alertable_wait },
};

int tw_test_main( void )
{
    unsigned int status;

    virtual_init();
    virtual_alloc_first_teb();
    dbg_init();
    config_dir = getenv( "WINEPREFIX" );  /* what set_config_dir does (loader.c) */
    server_init_process();
    /* what server_init_process_done does (unix/server.c L1750-1765) */
    signal_init_process();
    ntdll_get_thread_data()->syscall_table = KeServiceDescriptorTable;
    ntdll_get_thread_data()->syscall_trace = TRACE_ON(syscall);
    SERVER_START_REQ( init_process_done )
    {
        req->teb = wine_server_client_ptr( NtCurrentTeb() );
        req->peb = wine_server_client_ptr( NtCurrentTeb()->Peb );
        status = wine_server_call( req );
    }
    SERVER_END_REQ;
    if (status)
    {
        printf( "not ok init_process_done: %#x\n", status );
        return 1;
    }
    /* what signal_start_thread does for the main thread (signal_x86_64.c L2978) */
    pthread_sigmask( SIG_UNBLOCK, &server_block_set, NULL );
    tw_thread_start_hook = thread_hook;
    return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
}

/*
 * The `thread` checks (docs/design/wine-unix.md v3, section 10, "The M4
 * checks"): green threads (section 9) made by Wine's own NtCreateThreadEx,
 * so each has its TEB, kernel stack, server thread and fds; their ends;
 * process exit on a thread other than the main one (section 8); and
 * SIGQUIT's kill (section 7).
 *
 * No guest code: the harness sets tw_thread_start_hook, so
 * signal_start_thread runs a C routine of this file (the "entry" given to
 * NtCreateThreadEx, with its argument) in place of the thread's run loop,
 * after it has built the thread's state block. The routine ends its thread
 * through Wine: NtTerminateThread of itself (exit_thread), or process exit.
 *
 * The setup is syscall.c's, as start_main_thread and
 * server_init_process_done do it (loader.c, unix/server.c L1750-1752). The
 * JS test (tests/wine/thread.test.mjs) puts the prefix and l_intl.nls into
 * MEMFS first. The checks that end the process run alone: the test asserts
 * the status the process settled with.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include "config.h"

#include <emscripten/stack.h>
#include <errno.h>
#include <fcntl.h>
#include <sched.h>
#include <signal.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winnt.h"
#include "winternl.h"
#include "unix_private.h"
#include "wine/debug.h"

#include "harness.h"
#include "tempwind/abi.h"
#include "tempwind_private.h"
#include "server.h"

/* The running thread's frame record's state block: the first member of
 * src/signal_wasm64.c's struct syscall_frame (wine-unix.md, section 11). */
#define FRAME_STATE() (*(BYTE **)get_syscall_frame())

/* The statuses the process-exit checks end with; the JS test expects them. */
#define EXIT_FROM_THREAD   0x1234
#define EXIT_AFTER_PARK    0x4321
#define EXIT_TWO_ALIVE     0x5678

/* What a harness thread gets and reports. */
struct targ
{
    BYTE *state;              /* its state block, from signal_start_thread */
    int green_id;             /* its green thread's id, for in_wait */
    LONG exit_code;           /* what it ends with */
    HANDLE event_a, event_b;
    volatile int flag;        /* set by the thread when it reached a point */
    volatile int after;       /* set by code that must never run */
    volatile int main_asleep, main_woke;  /* the main thread's, around its sleep */
    int request_fd;           /* its request pipe's client end */
    HANDLE main_thread;       /* a real handle to the main thread */
    HANDLE other;             /* another harness thread */
    const char *fail;         /* its first failure */
    char buf[512];
};

static int order[8], order_count;  /* the order in which threads ended */

/* signal_start_thread's harness hook: runs the routine, then ends the thread
 * as RtlExitUserThread does for a thread that is not the last. */
static void thread_hook( void *entry, void *arg, void *state )
{
    PRTL_THREAD_START_ROUTINE routine = (PRTL_THREAD_START_ROUTINE)entry;
    struct targ *a = arg;

    a->state = state;
    a->green_id = pthread_getthreadid_np();
    routine( arg );
    NtTerminateThread( NtCurrentThread(), a->exit_code );
}

static THREAD_BASIC_INFORMATION thread_info( HANDLE handle )
{
    THREAD_BASIC_INFORMATION info = { 0 };

    NtQueryInformationThread( handle, ThreadBasicInformation, &info, sizeof(info), NULL );
    return info;
}

static struct ntdll_thread_data *teb_thread_data( TEB *teb )
{
    return (struct ntdll_thread_data *)&teb->GdiTebBatch;
}

/* ---- two threads that switch ---- */

#define ROUNDS 3
#define MAIN_DEPTH 7
#define T2_DEPTH   13
#define MAIN_MARK  0x1111111111111111ull
#define T2_MARK    0x2222222222222222ull

static int switch_log[2 * ROUNDS + 2], switch_log_count;

/* One thread's view across a switch: errno, depth, its stack limits, its
 * frame record's state block and TEB. The callers compare their own stack
 * pointer around the switch (a callee's frame would differ). */
struct view
{
    int err;
    int32_t depth;
    uintptr_t base, end;
    BYTE *state;
    TEB *teb;
};

static struct view take_view( int err )
{
    struct view v;

    v.base = emscripten_stack_get_base();
    v.end = emscripten_stack_get_end();
    v.depth = tw_depth_get();
    v.state = FRAME_STATE();
    v.teb = NtCurrentTeb();
    v.err = errno = err;
    return v;
}

static const char *same_view( const struct view *before, int who )
{
    if (errno != before->err) return tw_fail( "thread %d: errno %d, expected %d", who, errno, before->err );
    if (tw_depth_get() != before->depth) return tw_fail( "thread %d: depth %d, expected %d", who, tw_depth_get(), before->depth );
    if (emscripten_stack_get_base() != before->base || emscripten_stack_get_end() != before->end)
        return tw_fail( "thread %d: the stack limits changed", who );
    if (FRAME_STATE() != before->state) return tw_fail( "thread %d: another state block", who );
    if (NtCurrentTeb() != before->teb) return tw_fail( "thread %d: another TEB", who );
    return NULL;
}

static void WINAPI switch_thread( void *arg )
{
    struct targ *a = arg;
    struct ntdll_thread_data *data = ntdll_get_thread_data();
    char *bottom = data->kernel_stack, *top = bottom + kernel_stack_size;
    volatile char buf[64];
    const char *detail;
    CONTEXT *ctx;
    struct view v;
    int i;

    /* a new thread starts with depth 0, on its kernel stack (section 9) */
    T_EXPECT( a, tw_depth_get() == 0 );
    T_EXPECT( a, emscripten_stack_get_base() == (uintptr_t)top );
    T_EXPECT( a, emscripten_stack_get_end() == (uintptr_t)bottom );
    T_EXPECT( a, (char *)buf >= bottom && (char *)buf < top );
    /* its state block, as section 8 builds the main thread's */
    T_EXPECT( a, a->state && FRAME_STATE() == a->state );
    T_EXPECT( a, S64( a->state, TW_STATE_GS_BASE ) == (UINT64)(ULONG_PTR)NtCurrentTeb() );
    T_EXPECT( a, S64( a->state, TW_STATE_RIP ) == (UINT64)(ULONG_PTR)pLdrInitializeThunk );
    T_EXPECT( a, S32( a->state, TW_STATE_MXCSR ) == TW_INIT_MXCSR );
    ctx = (CONTEXT *)(ULONG_PTR)GPR( a->state, RCX );
    T_EXPECT( a, GPR( a->state, RSP ) == (UINT64)(ULONG_PTR)ctx - 8 );
    T_EXPECT( a, GPR( a->state, RDX ) == (UINT64)(ULONG_PTR)a );
    T_EXPECT( a, ctx->Rip == (ULONG_PTR)pRtlUserThreadStart );
    T_EXPECT( a, ctx->Rcx == (ULONG_PTR)switch_thread && ctx->Rdx == (ULONG_PTR)a );
    T_EXPECT( a, ctx->Rsp == (ULONG_PTR)NtCurrentTeb()->Tib.StackBase - 0x28 );
    if (a->fail) return;

    tw_depth_set( T2_DEPTH );
    GPR( a->state, RBX ) = T2_MARK;
    memset( (char *)buf, 2, sizeof(buf) );
    for (i = 0; i < ROUNDS; i++)
    {
        uintptr_t sp = emscripten_stack_get_current();

        switch_log[switch_log_count++] = 2;
        v = take_view( 2000 + i );
        sched_yield();
        if ((detail = same_view( &v, 2 ))) { thread_fail( a, "round %d: %s", i, detail ); break; }
        if (emscripten_stack_get_current() != sp) { thread_fail( a, "round %d: the stack pointer moved", i ); break; }
    }
    /* and across a server wait: the main thread sets event_b */
    v = take_view( 0 );
    NtSetEvent( a->event_a, NULL );
    if (wait_for( a->event_b )) thread_fail( a, "event_b's wait failed" );
    errno = 0;
    if ((detail = same_view( &v, 2 ))) thread_fail( a, "after the server wait: %s", detail );
    for (i = 0; i < sizeof(buf); i++) if (buf[i] != 2) thread_fail( a, "its stack buffer changed" );
    if (GPR( a->state, RBX ) != T2_MARK) thread_fail( a, "its state block changed" );
}

/* Two green threads that block and switch, each keeping its own depth,
 * errno, stack and state block across the switches (section 10). */
static const char *switch_two_threads( void )
{
    struct targ a = { 0 };
    BYTE *main_state = aligned_alloc( 64, TW_STATE_SIZE );
    BYTE *saved_state = FRAME_STATE();
    volatile char buf[64];
    const char *detail = NULL;
    struct view v;
    HANDLE thread;
    int i;

    TW_EXPECT( main_state );
    memset( main_state, 0, TW_STATE_SIZE );
    GPR( main_state, RBX ) = MAIN_MARK;
    FRAME_STATE() = main_state;
    memset( (char *)buf, 1, sizeof(buf) );
    a.event_a = new_event( SynchronizationEvent, FALSE );
    a.event_b = new_event( SynchronizationEvent, FALSE );
    TW_EXPECT( a.event_a && a.event_b );
    tw_depth_set( MAIN_DEPTH );
    switch_log_count = 0;

    TW_EXPECT( (thread = spawn( switch_thread, &a, 0 )) );
    for (i = 0; i < ROUNDS && !detail; i++)
    {
        uintptr_t sp = emscripten_stack_get_current();

        switch_log[switch_log_count++] = 1;
        v = take_view( 1000 + i );
        sched_yield();  /* the spawned thread counts as runnable before it starts */
        if (!(detail = same_view( &v, 1 )) && emscripten_stack_get_current() != sp)
            detail = "thread 1: the stack pointer moved";
    }
    if (!detail)
    {
        v = take_view( 0 );
        if (wait_for( a.event_a )) detail = "event_a's wait failed";
        errno = 0;
        if (!detail) detail = same_view( &v, 1 );
        NtSetEvent( a.event_b, NULL );
    }
    if (wait_for( thread )) detail = detail ? detail : "the thread didn't end";
    for (i = 0; i < sizeof(buf); i++) if (buf[i] != 1) detail = detail ? detail : "the main stack buffer changed";
    if (GPR( main_state, RBX ) != MAIN_MARK) detail = detail ? detail : "the main state block changed";
    tw_depth_set( 0 );
    FRAME_STATE() = saved_state;
    NtClose( thread );
    NtClose( a.event_a );
    NtClose( a.event_b );
    if (detail) return detail;
    if (a.fail) return tw_fail( "thread 2: %s", a.fail );
    /* FIFO: each yield lets the other thread run first. The scheduler's
     * order is the unit under test, and only the threads' own log shows it. */
    if (switch_log_count != 2 * ROUNDS) return tw_fail( "%d rounds logged", switch_log_count );
    for (i = 0; i < 2 * ROUNDS; i++)
        if (switch_log[i] != 1 + (i & 1)) return tw_fail( "round %d ran thread %d", i / 2, switch_log[i] );
    return NULL;
}

/* ---- join and the order of the ends ---- */

static void WINAPI waits_then_ends( void *arg )
{
    struct targ *a = arg;

    a->flag = 1;
    if (wait_for( a->event_a )) thread_fail( a, "its event's wait failed" );
    order[order_count++] = 2;
    a->exit_code = 0x22;
}

static void WINAPI ends_at_once( void *arg )
{
    order[order_count++] = 3;
    ((struct targ *)arg)->exit_code = 0x33;
}

/* Wine's exit_thread joins the previous exiting thread and frees its TEB
 * (unix/thread.c L1503-1510): the thread that ended first is freed only
 * when the next one ends (section 9, "Join"). A thread's end signals its
 * handle through the server's hang-up (section 7, "A thread's end"). */
static const char *join_and_exit_order( void )
{
    struct targ a2 = { 0 }, a3 = { 0 };
    THREAD_BASIC_INFORMATION info;
    HANDLE t2, t3;
    TEB *teb3;

    order_count = 0;
    TW_EXPECT( (a2.event_a = new_event( SynchronizationEvent, FALSE )) );
    TW_EXPECT( (t2 = spawn( waits_then_ends, &a2, 0 )) );
    TW_EXPECT( (t3 = spawn( ends_at_once, &a3, 0 )) );
    if (wait_for( t3 )) return tw_fail( "thread 3's handle wasn't signaled" );
    teb3 = thread_info( t3 ).TebBaseAddress;  /* the server learns it at the thread's init_thread */
    TW_EXPECT( teb3 );
    TW_EXPECT( a2.flag && order_count == 1 && order[0] == 3 );
    info = thread_info( t3 );
    if (info.ExitStatus != 0x33) return tw_fail( "thread 3's exit status %#x", (unsigned)info.ExitStatus );
    /* not joined yet: its frame record is still allocated */
    TW_EXPECT( teb_thread_data( teb3 )->syscall_frame );
    TW_EXPECT( thread_info( t2 ).ExitStatus == STATUS_PENDING );

    NtSetEvent( a2.event_a, NULL );
    if (wait_for( t2 )) return tw_fail( "thread 2's handle wasn't signaled" );
    if (a2.fail) return tw_fail( "thread 2: %s", a2.fail );
    TW_EXPECT( order_count == 2 && order[1] == 2 );
    info = thread_info( t2 );
    if (info.ExitStatus != 0x22) return tw_fail( "thread 2's exit status %#x", (unsigned)info.ExitStatus );
    /* thread 2's exit_thread joined thread 3 and freed its TEB:
     * signal_free_thread cleared the frame record */
    TW_EXPECT( !teb_thread_data( teb3 )->syscall_frame );
    NtClose( t2 );
    NtClose( t3 );
    NtClose( a2.event_a );
    return NULL;
}

/* ---- SIGQUIT's kill ---- */

static void WINAPI sleeps( void *arg )
{
    struct targ *a = arg;
    LARGE_INTEGER timeout = { .QuadPart = -60000 * (LONGLONG)MS };

    a->request_fd = ntdll_get_thread_data()->request_fd;
    a->flag = 1;
    /* the scenario: a 60 s non-alertable sleep, in select, not in the server */
    NtDelayExecution( FALSE, &timeout );
    a->after = 1;
}

/* NtTerminateThread of a sleeping thread: the server's kill_thread sends it
 * SIGQUIT (server/thread.c L1608-1631), which ends its sleep; the thread
 * handles it with abort_thread(0) (section 9) and the process goes on. */
static const char *sigquit_kills_sleeper( void )
{
    struct targ a = { .request_fd = -1 };
    LONGLONG start;
    HANDLE thread;

    TW_EXPECT( (thread = spawn( sleeps, &a, 0 )) );
    UNTIL( a.flag && in_wait( a.green_id ) );  /* in its sleep's select */
    TW_EXPECT( a.request_fd >= 0 && !a.after && !signaled( thread ) );
    start = now();
    if (NtTerminateThread( thread, 0x77 )) return tw_fail( "NtTerminateThread failed" );
    /* kill_thread signals the handle before the thread has run its handler
     * (server/thread.c L1608-1631) */
    if (wait_for( thread )) return tw_fail( "the thread's handle wasn't signaled" );
    /* abort_thread reached pthread_exit_wrapper, which closed its fds */
    UNTIL( fcntl( a.request_fd, F_GETFD ) == -1 && errno == EBADF );
    if (now() - start >= 10000 * MS) return tw_fail( "the kill took %lld ms", (now() - start) / MS );
    if (a.after) return tw_fail( "the sleeper returned from its sleep" );
    if (thread_info( thread ).ExitStatus != 0x77) return tw_fail( "exit status %#x", (unsigned)thread_info( thread ).ExitStatus );
    NtClose( thread );
    return NULL;
}

/* ---- the checks that end the process (alone) ---- */

static void report( const char *name, const char *detail )
{
    if (detail) printf( "not ok %s: %s\n", name, detail );
    else printf( "ok %s\n", name );
    fflush( stdout );
}

/* Whether the main thread (green thread 1) is in its sleep: it set
 * main_asleep just before the sleep, and waits in the sleep's select. */
static BOOL main_sleeps( struct targ *a )
{
    return YIELD_UNTIL( a->main_asleep && in_wait( 1 ) ) && !a->main_woke && !signaled( a->main_thread );
}

static void WINAPI exits_process( void *arg )
{
    struct targ *a = arg;
    struct green_thread *main_green;

    report( "main_sleeps", main_sleeps( a ) ? NULL : "the main thread isn't in its sleep" );
    NtTerminateProcess( 0, EXIT_FROM_THREAD );  /* RtlExitUserProcess's first call: kills the main thread */
    /* Pending is the scheduler's own state (section 9, "Sending"): nothing
     * outside shows a signal that waits for its delivery point. That it was
     * never handled shows: abort_thread would have closed the main thread's
     * fds. */
    main_green = green_thread_by_id( 1 );
    report( "main_sigquit_pending",
            !main_green ? "the main green thread is gone" :
            !(main_green->sig_pending & ((uint64_t)1 << (SIGQUIT - 1))) ? "no SIGQUIT pending on the main thread" :
            fcntl( a->request_fd, F_GETFD ) == -1 ? "the main thread handled its SIGQUIT" : NULL );
    NtTerminateProcess( NtCurrentProcess(), EXIT_FROM_THREAD );
    report( "exits_process", "NtTerminateProcess returned" );
}

/* Process exit from the second thread while the main thread sleeps 60 s in
 * a non-alertable NtDelayExecution: the status arrives through
 * TW_THREAD_PROCESS_EXIT well before the deadline, and the sleeper's SIGQUIT
 * stays pending (section 10). */
static const char *process_exit_from_thread( void )
{
    static struct targ a;

    TW_EXPECT( !NtDuplicateObject( NtCurrentProcess(), NtCurrentThread(), NtCurrentProcess(),
                                   &a.main_thread, 0, 0, DUPLICATE_SAME_ACCESS ) );
    a.request_fd = ntdll_get_thread_data()->request_fd;
    TW_EXPECT( spawn( exits_process, &a, 0 ) );
    a.main_asleep = 1;
    sleep_ms( 60000 );  /* the scenario: the sleep that the other thread's process exit cuts short */
    a.main_woke = 1;
    return tw_fail( "the main thread woke from its sleep" );
}

static void WINAPI exits_after_main( void *arg )
{
    struct targ *a = arg;

    sleep_ms( 10 );  /* the scenario: a timeout that fires with the main thread parked */
    /* The park is the scheduler's own state (section 9, "The main thread"):
     * nothing outside tells a parked thread from one that waits. */
    report( "main_parked", green_parked && green_parked->id == 1 ? NULL : "the main thread isn't parked" );
    /* the main thread's end signaled its handle (the hang-up, section 7) */
    report( "main_handle_signaled", signaled( a->main_thread ) ? NULL : "the main thread's handle isn't signaled" );
    NtTerminateProcess( 0, EXIT_AFTER_PARK );
    NtTerminateProcess( NtCurrentProcess(), EXIT_AFTER_PARK );
    report( "exits_after_main", "NtTerminateProcess returned" );
}

/* The main thread's exit with another thread alive parks it (exit_thread
 * reaches pthread_exit), and that thread's later process exit settles the
 * run (section 10). */
static const char *main_exit_parks( void )
{
    static struct targ a;

    TW_EXPECT( !NtDuplicateObject( NtCurrentProcess(), NtCurrentThread(), NtCurrentProcess(),
                                   &a.main_thread, 0, 0, DUPLICATE_SAME_ACCESS ) );
    TW_EXPECT( spawn( exits_after_main, &a, 0 ) );
    NtTerminateThread( NtCurrentThread(), 0 );
    return tw_fail( "the main thread ran after its exit" );
}

static void WINAPI waits_forever( void *arg )
{
    struct targ *a = arg;

    a->flag = 1;
    NtWaitForSingleObject( a->event_a, FALSE, NULL );  /* a server wait */
    a->after = 1;
}

static void WINAPI exits_with_two_alive( void *arg )
{
    struct targ *a = arg;

    /* The other thread started first and set its flag just before its server
     * wait, so it is in that wait. */
    report( "two_threads_alive", main_sleeps( a ) && a->flag && !a->after && !signaled( a->other ) ? NULL
            : "the main thread isn't asleep, or the other thread isn't in its wait" );
    NtTerminateProcess( 0, EXIT_TWO_ALIVE );
    NtTerminateProcess( NtCurrentProcess(), EXIT_TWO_ALIVE );
    report( "exits_with_two_alive", "NtTerminateProcess returned" );
}

/* An exit with two threads alive (the main thread asleep, another in a
 * server wait) leaves no tempwind: line: process_sigkill is unreached
 * (section 7, "Kills"). */
static const char *exit_with_two_alive( void )
{
    static struct targ waiter;

    TW_EXPECT( !NtDuplicateObject( NtCurrentProcess(), NtCurrentThread(), NtCurrentProcess(),
                                   &waiter.main_thread, 0, 0, DUPLICATE_SAME_ACCESS ) );
    TW_EXPECT( (waiter.event_a = new_event( SynchronizationEvent, FALSE )) );
    TW_EXPECT( (waiter.other = spawn( waits_forever, &waiter, 0 )) );
    TW_EXPECT( spawn( exits_with_two_alive, &waiter, 0 ) );
    waiter.main_asleep = 1;
    sleep_ms( 60000 );  /* the scenario: the sleep that the other thread's process exit cuts short */
    waiter.main_woke = 1;
    return tw_fail( "the main thread woke from its sleep" );
}

static const struct tw_check checks[] =
{
    { "switch_two_threads", switch_two_threads },
    { "join_and_exit_order", join_and_exit_order },
    { "sigquit_kills_sleeper", sigquit_kills_sleeper },
    { "process_exit_from_thread", process_exit_from_thread, TW_ALONE },
    { "main_exit_parks", main_exit_parks, TW_ALONE },
    { "exit_with_two_alive", exit_with_two_alive, TW_ALONE },
};

int tw_test_main( void )
{
    tw_process_start();
    tw_thread_start_hook = thread_hook;
    return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
}

/*
 * What the harnesses that start the in-process server share, next to
 * harness.h (docs/design/wine-unix.md, section 10): the server start every
 * one of them runs at the head of tw_test_main, and, for those that run
 * green threads, the helpers for spawning them, waiting on them and reading
 * a state block.
 *
 * Include it after config.h, ntstatus.h, windef.h, winnt.h, winternl.h,
 * unix_private.h, harness.h and tempwind_private.h (in_wait reads the
 * scheduler's thread). Every definition is static, so a harness that doesn't
 * use one pays nothing.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#ifndef TEMPWIND_SERVER_H
#define TEMPWIND_SERVER_H

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

#include "wine/debug.h"

WINE_DECLARE_DEBUG_CHANNEL(syscall);

/* ---- the server start ---- */

/* What start_main_thread does before the process runs (loader.c):
 * virtual_init, the first TEB, dbg_init, then server_init_process, which
 * starts the server in this thread. The JS test puts the prefix directory and
 * l_intl.nls into MEMFS first. */
static inline void tw_server_start( void )
{
    virtual_init();
    virtual_alloc_first_teb();
    dbg_init();
    config_dir = getenv( "WINEPREFIX" );  /* what set_config_dir does (loader.c) */
    server_init_process();
}

/* The server start, then what server_init_process_done does on this side
 * (unix/server.c L1750-1752). */
static inline void tw_process_start( void )
{
    tw_server_start();
    signal_init_process();
    ntdll_get_thread_data()->syscall_table = KeServiceDescriptorTable;
    ntdll_get_thread_data()->syscall_trace = TRACE_ON(syscall);
}

/* ---- a state block ---- */

#define MS 10000  /* ticks of 100 ns */
#define S64(s, off) (*(UINT64 *)((BYTE *)(s) + (off)))
#define S32(s, off) (*(UINT32 *)((BYTE *)(s) + (off)))
#define GPR(s, i)   S64( s, TW_STATE_GPR(i) )
enum { RAX, RCX, RDX, RBX, RSP, RBP, RSI, RDI, R8, R9, R10, R11, R12, R13, R14, R15 };

/* ---- time and waits ---- */

static inline LONGLONG now( void )
{
    LARGE_INTEGER t;

    NtQueryPerformanceCounter( &t, NULL );
    return t.QuadPart;
}

/* A non-alertable NtDelayExecution: only for a sleep that is the scenario. */
static inline void sleep_ms( LONGLONG ms )
{
    LARGE_INTEGER timeout = { .QuadPart = -ms * MS };

    NtDelayExecution( FALSE, &timeout );
}

/* A wait for another thread's event fails at this deadline: far beyond any
 * check's run, so reaching it means the event never came. */
#define DEADLINE_MS 10000

/* Waits until the object is signaled, or fails at the deadline. */
static inline NTSTATUS wait_for( HANDLE handle )
{
    LARGE_INTEGER timeout = { .QuadPart = -DEADLINE_MS * (LONGLONG)MS };

    return NtWaitForSingleObject( handle, FALSE, &timeout );
}

/* Whether the object is signaled now: a zero-timeout wait. */
static inline BOOL signaled( HANDLE handle )
{
    LARGE_INTEGER timeout = { .QuadPart = 0 };

    return !NtWaitForSingleObject( handle, FALSE, &timeout );
}

/* Whether the green thread with that id is blocked in a wait: a select or a
 * server wait. A check's precondition that nothing outside shows; the
 * scheduler's waiter does, which only a wait sets (green.c, wait_until; a
 * yield sets none). */
static inline BOOL in_wait( int id )
{
    struct green_thread *thread = green_thread_by_id( id );

    return thread && thread->waiter;
}

/* Yields until cond holds and is then 1, or 0 at the deadline. Green threads
 * switch only at blocking points, and sched_yield lets the runnable ones run
 * first (wine-unix.md v3, section 9): so once a flag that another thread sets
 * just before a blocking call shows here, that thread is in the call. A
 * condition that needs a timer to fire never holds here, since a yield with
 * nothing runnable returns at once; wait on an object for that. */
#define YIELD_UNTIL(cond) ({ \
    LONGLONG until_end_ = now() + DEADLINE_MS * (LONGLONG)MS; \
    int until_ok_; \
    while (!(until_ok_ = !!(cond)) && now() < until_end_) NtYieldExecution(); \
    until_ok_; })

/* In a check: fails it, naming the condition, when it doesn't hold by the
 * deadline. */
#define UNTIL(cond) do { if (!YIELD_UNTIL( cond )) \
    return tw_fail( "%s:%d: not true after %d ms: %s", __FILE__, __LINE__, DEADLINE_MS, #cond ); } while (0)

/* ---- threads and events ---- */

/* Makes a thread of Wine's own NtCreateThreadEx running routine( arg ); 0 on
 * failure. flags is THREAD_CREATE_FLAGS_*, 0 for a thread that starts at once. */
static inline HANDLE spawn( void (WINAPI *routine)( void * ), void *arg, ULONG flags )
{
    HANDLE handle = 0;

    if (NtCreateThreadEx( &handle, THREAD_ALL_ACCESS, NULL, NtCurrentProcess(),
                          (PRTL_THREAD_START_ROUTINE)routine, arg, flags, 0, 0, 0, NULL ))
        return 0;
    return handle;
}

/* type is NotificationEvent or SynchronizationEvent; 0 on failure. */
static inline HANDLE new_event( EVENT_TYPE type, BOOLEAN initial )
{
    HANDLE event = 0;

    if (NtCreateEvent( &event, EVENT_ALL_ACCESS, NULL, type, initial )) return 0;
    return event;
}

/* Records a harness thread's first failure in its argument, any struct with
 * a `fail` member and a `buf` to format into; the check that joined the
 * thread returns it. */
#define thread_fail(a, ...) do { \
    if (!(a)->fail) { snprintf( (a)->buf, sizeof((a)->buf), __VA_ARGS__ ); (a)->fail = (a)->buf; } } while (0)

#define T_EXPECT(a, cond) do { if (!(cond)) thread_fail( a, "%s:%d: %s", __FILE__, __LINE__, #cond ); } while (0)

#endif /* TEMPWIND_SERVER_H */

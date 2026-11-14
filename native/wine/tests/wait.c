/*
 * The `wait` checks (docs/design/wine-unix.md, section 10): waits on one
 * thread, through the in-process server (section 7) and green.c (section 9).
 *
 * The setup runs before the checks, as start_main_thread does it (loader.c):
 * virtual_init, the first TEB, dbg_init, then server_init_process, which
 * starts the server in this thread. The JS test puts the prefix directory
 * and l_intl.nls into MEMFS first.
 *
 * Times are measured with NtQueryPerformanceCounter, the monotonic clock the
 * server's relative timeouts use, in ticks of 100 ns.
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
#include <stdarg.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winternl.h"
#include "unix_private.h"

#include "harness.h"
#include "tempwind_private.h"

#define MS 10000  /* ticks of 100 ns */

/* NtDelayExecution's non-alertable sleep counts its deadline with
 * NtQuerySystemTime, whose clock (Emscripten's Date.now) has a resolution of
 * 1 ms; measured on the monotonic clock it may end up to that much early. */
#define REALTIME_RESOLUTION (1 * MS)

/* "Long enough to tell from returning at once": a wait that should not
 * block finishes well within it, one that blocks takes at least the time. */
#define LONG_TIMEOUT (10000 * MS)

static LONGLONG now( void )
{
    LARGE_INTEGER t;

    NtQueryPerformanceCounter( &t, NULL );
    return t.QuadPart;
}

static HANDLE new_event( BOOLEAN signaled )
{
    HANDLE event = 0;

    if (NtCreateEvent( &event, EVENT_ALL_ACCESS, NULL, NotificationEvent, signaled )) return 0;
    return event;
}

/* An event wait with a timeout returns STATUS_TIMEOUT after at least that time. */
static const char *event_timeout( void )
{
    LARGE_INTEGER timeout = { .QuadPart = -100 * MS };
    HANDLE event = new_event( FALSE );
    LONGLONG start, elapsed;
    NTSTATUS status;

    TW_EXPECT( event );
    start = now();
    status = NtWaitForSingleObject( event, FALSE, &timeout );
    elapsed = now() - start;
    NtClose( event );
    if (status != STATUS_TIMEOUT) return tw_fail( "status %#x, expected STATUS_TIMEOUT", (unsigned)status );
    if (elapsed < 100 * MS) return tw_fail( "returned after %lld ticks, before the 100 ms timeout", elapsed );
    return NULL;
}

/* A signaled event returns at once. */
static const char *event_signaled( void )
{
    LARGE_INTEGER timeout = { .QuadPart = -LONG_TIMEOUT };
    HANDLE event = new_event( FALSE );
    LONGLONG start, elapsed;
    NTSTATUS status;

    TW_EXPECT( event );
    TW_EXPECT( !NtSetEvent( event, NULL ) );
    start = now();
    status = NtWaitForSingleObject( event, FALSE, &timeout );
    elapsed = now() - start;
    NtClose( event );
    if (status != STATUS_WAIT_0) return tw_fail( "status %#x, expected STATUS_WAIT_0", (unsigned)status );
    if (elapsed >= LONG_TIMEOUT) return tw_fail( "waited %lld ticks for a signaled event", elapsed );
    return NULL;
}

/* A timer fires: a wait with no timeout on a timer set 50 ms ahead ends when
 * it is signaled. */
static const char *timer_fires( void )
{
    LARGE_INTEGER due = { .QuadPart = -50 * MS };
    HANDLE timer = 0;
    LONGLONG start, elapsed;
    NTSTATUS status;

    TW_EXPECT( !NtCreateTimer( &timer, TIMER_ALL_ACCESS, NULL, NotificationTimer ) );
    start = now();
    TW_EXPECT( !NtSetTimer( timer, &due, NULL, NULL, FALSE, 0, NULL ) );
    status = NtWaitForSingleObject( timer, FALSE, NULL );
    elapsed = now() - start;
    NtClose( timer );
    if (status != STATUS_WAIT_0) return tw_fail( "status %#x, expected STATUS_WAIT_0", (unsigned)status );
    if (elapsed < 50 * MS) return tw_fail( "signaled after %lld ticks, before its 50 ms", elapsed );
    return NULL;
}

/* A zero timeout polls: STATUS_TIMEOUT at once while the event is not
 * signaled, STATUS_WAIT_0 once it is. */
static const char *zero_timeout_polls( void )
{
    LARGE_INTEGER zero = { .QuadPart = 0 };
    HANDLE event = new_event( FALSE );
    LONGLONG start, elapsed;
    NTSTATUS status, status2;

    TW_EXPECT( event );
    start = now();
    status = NtWaitForSingleObject( event, FALSE, &zero );
    elapsed = now() - start;
    NtSetEvent( event, NULL );
    status2 = NtWaitForSingleObject( event, FALSE, &zero );
    NtClose( event );
    if (status != STATUS_TIMEOUT) return tw_fail( "unsignaled: status %#x, expected STATUS_TIMEOUT", (unsigned)status );
    if (elapsed >= LONG_TIMEOUT) return tw_fail( "a poll took %lld ticks", elapsed );
    if (status2 != STATUS_WAIT_0) return tw_fail( "signaled: status %#x, expected STATUS_WAIT_0", (unsigned)status2 );
    return NULL;
}

/* NtDelayExecution sleeps for its time, alertable or not. */
static const char *delay( BOOLEAN alertable, LONGLONG slack )
{
    LARGE_INTEGER timeout = { .QuadPart = -50 * MS };
    LONGLONG start, elapsed;
    NTSTATUS status;

    start = now();
    status = NtDelayExecution( alertable, &timeout );
    elapsed = now() - start;
    if (status != STATUS_SUCCESS) return tw_fail( "status %#x, expected STATUS_SUCCESS", (unsigned)status );
    if (elapsed < 50 * MS - slack) return tw_fail( "slept %lld ticks, less than 50 ms", elapsed );
    return NULL;
}

/* Not alertable: green.c's select sleep (sync.c). */
static const char *delay_not_alertable( void )
{
    return delay( FALSE, REALTIME_RESOLUTION );
}

/* Alertable: a server wait with a timeout. */
static const char *delay_alertable( void )
{
    return delay( TRUE, 0 );
}

/* Alone: a wait nothing can end stops loudly (the scheduler's deadlock). */
static const char *wait_forever_stops( void )
{
    HANDLE event = new_event( FALSE );
    NTSTATUS status;

    TW_EXPECT( event );
    status = NtWaitForSingleObject( event, FALSE, NULL );
    return tw_fail( "the wait returned %#x", (unsigned)status );
}

static const struct tw_check checks[] =
{
    { "event_timeout", event_timeout },
    { "event_signaled", event_signaled },
    { "timer_fires", timer_fires },
    { "zero_timeout_polls", zero_timeout_polls },
    { "delay_not_alertable", delay_not_alertable },
    { "delay_alertable", delay_alertable },
    { "wait_forever_stops", wait_forever_stops, TW_ALONE },
};

int tw_test_main( void )
{
    virtual_init();
    virtual_alloc_first_teb();
    dbg_init();
    server_init_process();
    return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
}

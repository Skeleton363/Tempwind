/*
 * Blocking, sleeping and waking of green threads (docs/design/wine-unix.md,
 * section 9; abi.md v5, "Green threads").
 *
 * green_switch is the only switch point. Above it: waits on wake queues with
 * an optional deadline, sleeping, yielding, the scheduler's entries, and the
 * libc calls Wine blocks in (select, poll, usleep, nanosleep, sched_yield),
 * which this file defines in place of libc's.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <emscripten/stack.h>
#include <emscripten/syscalls.h>
#include <errno.h>
#include <poll.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/select.h>
#include <time.h>
#include <unistd.h>

#include "tempwind_private.h"

/* compiler-rt's stack_ops.S; JS calls the same function as a raw export. */
void _emscripten_stack_restore( uintptr_t sp );

/* The in-process server's (server/fd.c): the clock, and its timeouts in
 * ticks of 100 ns (wine-unix.md, section 7). */
void set_current_time( void );
int64_t inproc_server_next_timeout( void );
void inproc_server_run_timeouts( void );

enum waiter_state { WAITING, WOKEN, TIMED_OUT };

/* A green thread waiting: on a queue, until a deadline, or both. */
struct green_waiter
{
    struct green_waiter *next, *prev;    /* in the queue */
    struct green_waiter *wnext, *wprev;  /* in the list of all waiters */
    struct green_queue *queue;           /* NULL: woken only by the deadline */
    struct green_thread *thread;
    int64_t deadline;                    /* CLOCK_MONOTONIC in ns, or -1 */
    const char *what;
    enum waiter_state state;
    int counted;                         /* counted in `runnable` */
};

static struct green_waiter *waiters;  /* every thread that waits */
static int runnable;                  /* threads woken and not yet resumed */

static int64_t monotonic_ns( void )
{
    struct timespec ts;

    clock_gettime( CLOCK_MONOTONIC, &ts );
    return (int64_t)ts.tv_sec * 1000000000 + ts.tv_nsec;
}

/* Blocks the current green thread in green_block and returns the value JS
 * resumes it with. Around the call it saves and restores the spike's save
 * set: the stack pointer and limits, errno and the current thread, since
 * other green threads run on their own stacks meanwhile. */
int64_t green_switch( void )
{
    uintptr_t sp = emscripten_stack_get_current();
    uintptr_t base = emscripten_stack_get_base();
    uintptr_t end = emscripten_stack_get_end();
    int saved_errno = errno;
    struct green_thread *self = green_current;
    int64_t ret;

    if (self->mutexes_held)
    {
        fprintf( stderr, "tempwind: green: thread %d suspends while holding %d mutexes\n",
                 self->id, self->mutexes_held );
        abort();
    }
    ret = green_block( self->id );
    emscripten_stack_set_limits( (void *)base, (void *)end );
    _emscripten_stack_restore( sp );
    green_current = self;
    set_current_time();  /* the server's clock, at every switch (section 7); may set errno */
    errno = saved_errno;
    return ret;
}

/* Ends the wait of w with state and makes its thread runnable. */
static void wake_waiter( struct green_waiter *w, enum waiter_state state )
{
    if (w->queue)
    {
        if (w->prev) w->prev->next = w->next;
        else w->queue->head = w->next;
        if (w->next) w->next->prev = w->prev;
        else w->queue->tail = w->prev;
        w->queue->waiters--;
    }
    if (w->wprev) w->wprev->wnext = w->wnext;
    else waiters = w->wnext;
    if (w->wnext) w->wnext->wprev = w->wprev;
    w->state = state;
    w->counted = 1;
    runnable++;
    green_wake( w->thread->id );
}

/* Waits on queue (or none) until woken or until deadline (monotonic ns, or
 * -1 for none); 0 or ETIMEDOUT. A deadline that has passed returns at once. */
static int wait_until( struct green_queue *queue, int64_t deadline, const char *what )
{
    struct green_waiter w = { .queue = queue, .thread = green_current, .deadline = deadline,
                              .what = what, .state = WAITING };

    if (deadline >= 0 && deadline <= monotonic_ns()) return ETIMEDOUT;
    if (queue)
    {
        w.prev = queue->tail;
        if (queue->tail) queue->tail->next = &w;
        else queue->head = &w;
        queue->tail = &w;
        queue->waiters++;
    }
    w.wnext = waiters;
    if (waiters) waiters->wprev = &w;
    waiters = &w;

    while (w.state == WAITING) green_switch();
    if (w.counted) runnable--;
    return w.state == TIMED_OUT ? ETIMEDOUT : 0;
}

/* Waits on queue until woken or until abs_deadline (on clock); 0 or
 * ETIMEDOUT. */
int green_wait( struct green_queue *queue, const struct timespec *abs_deadline, int clock )
{
    int64_t deadline = -1;

    if (abs_deadline)
    {
        struct timespec now;
        int64_t rel;

        if (clock_gettime( clock, &now )) return EINVAL;
        rel = (int64_t)(abs_deadline->tv_sec - now.tv_sec) * 1000000000
              + (abs_deadline->tv_nsec - now.tv_nsec);
        if (rel <= 0) return ETIMEDOUT;
        deadline = monotonic_ns() + rel;
    }
    return wait_until( queue, deadline, queue->what ? queue->what : "a condition variable" );
}

/* Wakes the first waiter of queue, or all. */
void green_wake_queue( struct green_queue *queue, int all )
{
    while (queue->head)
    {
        wake_waiter( queue->head, WOKEN );
        if (!all) break;
    }
}

/* Sleeps for ns (> 0). */
static void sleep_ns( int64_t ns, const char *what )
{
    wait_until( NULL, monotonic_ns() + ns, what );
}

/* Lets the runnable green threads run first; with none, returns at once. */
static void yield( void )
{
    struct green_waiter w = { .thread = green_current, .deadline = -1, .what = "a yield" };

    if (!runnable) return;
    /* Not in the list of waiters: it is runnable from the start, after the
     * threads woken before it. */
    w.state = WOKEN;
    w.counted = 1;
    runnable++;
    green_wake( w.thread->id );
    green_switch();
    runnable--;
}

/* ---- the scheduler's entries (abi.md v5, "Host entries of the Unix side") ---- */

/* Milliseconds until the earliest thread deadline or server timeout, rounded
 * up; 0 if one is due; -1 if there is none, after printing each waiting
 * thread's wait for the deadlock report (section 9). */
int64_t tw_next_timeout( void )
{
    struct green_waiter *w;
    int64_t ms = -1, server, now;

    set_current_time();
    if ((server = inproc_server_next_timeout()) >= 0) ms = (server + 9999) / 10000;
    now = monotonic_ns();
    for (w = waiters; w; w = w->wnext)
    {
        int64_t left;

        if (w->deadline < 0) continue;
        left = w->deadline > now ? (w->deadline - now + 999999) / 1000000 : 0;
        if (ms < 0 || left < ms) ms = left;
    }
    if (ms < 0)
    {
        for (w = waiters; w; w = w->wnext)
            fprintf( stderr, "tempwind: green: thread %d waits on %s with no deadline\n",
                     w->thread->id, w->what );
    }
    return ms;
}

/* Runs the expired server timeouts, then wakes the threads whose deadline
 * has passed. */
void tw_run_timeouts( void )
{
    struct green_waiter *w, *next;
    int64_t now;

    inproc_server_run_timeouts();
    now = monotonic_ns();
    for (w = waiters; w; w = next)
    {
        next = w->wnext;
        if (w->deadline >= 0 && w->deadline <= now) wake_waiter( w, TIMED_OUT );
    }
}

/* ---- libc's blocking calls, routed here (section 9) ---- */

int usleep( useconds_t usec )
{
    if (usec) sleep_ns( (int64_t)usec * 1000, "usleep" );
    return 0;
}

int nanosleep( const struct timespec *req, struct timespec *rem )
{
    if (req->tv_sec < 0 || req->tv_nsec < 0 || req->tv_nsec >= 1000000000)
    {
        errno = EINVAL;
        return -1;
    }
    if (req->tv_sec || req->tv_nsec)
        sleep_ns( (int64_t)req->tv_sec * 1000000000 + req->tv_nsec, "nanosleep" );
    if (rem) rem->tv_sec = rem->tv_nsec = 0;
    return 0;
}

int select( int n, fd_set *rfds, fd_set *wfds, fd_set *efds, struct timeval *tv )
{
    if (n > 0 && (rfds || wfds || efds))
        tw_unsupported( "select on fds (green.c handles select with no fds, a sleep)" );
    if (!tv)
    {
        wait_until( NULL, -1, "select with no fds and no timeout (a sleep forever)" );
        return 0;  /* not reached: nothing can end that wait */
    }
    if (tv->tv_sec < 0 || tv->tv_usec < 0)
    {
        errno = EINVAL;
        return -1;
    }
    if (tv->tv_sec || tv->tv_usec)
        sleep_ns( (int64_t)tv->tv_sec * 1000000000 + (int64_t)tv->tv_usec * 1000, "select (a sleep)" );
    return 0;
}

int poll( struct pollfd *fds, nfds_t n, int timeout )
{
    int ret = __syscall_poll_nonblocking( fds, n );

    if (ret < 0)
    {
        errno = -ret;
        return -1;
    }
    if (ret || !timeout) return ret;
    if (n) tw_unsupported( "poll that would block on fds (green.c handles a zero timeout or no fds)" );
    if (timeout < 0) wait_until( NULL, -1, "poll with no fds and no timeout (a sleep forever)" );
    else sleep_ns( (int64_t)timeout * 1000000, "poll with no fds (a sleep)" );
    return 0;
}

int sched_yield( void )
{
    yield();
    return 0;
}

/*
 * Blocking, sleeping and waking of green threads (docs/design/wine-unix.md,
 * section 9; abi.md v5, "Green threads").
 *
 * M2 WP1 skeleton: green_switch, the only switch point, is real; sleeping,
 * the wake queue and the scheduler's entries stop loudly until WP3 owns them.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <emscripten/stack.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "tempwind_private.h"

/* compiler-rt's stack_ops.S; JS calls the same function as a raw export. */
void _emscripten_stack_restore( uintptr_t sp );

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
    errno = saved_errno;
    green_current = self;
    return ret;
}

/* Waits on queue until woken or until abs_deadline (on clock); 0 or
 * ETIMEDOUT. */
int green_wait( struct green_queue *queue, const struct timespec *abs_deadline, int clock )
{
    tw_unsupported( "green_wait" );
}

/* Wakes one waiter of queue, or all. */
void green_wake_queue( struct green_queue *queue, int all )
{
    tw_unsupported( "green_wake_queue" );
}

/* The scheduler's entries (abi.md v5, "Host entries of the Unix side"). */
int64_t tw_next_timeout( void )
{
    tw_unsupported( "tw_next_timeout" );
}

void tw_run_timeouts( void )
{
    tw_unsupported( "tw_run_timeouts" );
}

/*
 * libc functions Wine calls that Emscripten 6.0.12's sysroot lacks
 * (docs/design/wine-unix.md, section 2). Each says why. A stub fails with an
 * errno that Wine's caller handles, or stops loudly; it never pretends to
 * succeed. Later packages append entries, each with its reason.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>

#include "tempwind_private.h"

/* Stopping loudly: the line the tests match, then abort(). */
_Noreturn void tw_unsupported( const char *what )
{
    fprintf( stderr, "tempwind: unsupported: %s\n", what );
    abort();
}

/* libc's clock_nanosleep (under nanosleep and usleep) sleeps through this
 * function of Emscripten's pthread library, which the -ww libc without
 * -pthread lacks. A blocking sleep would stop every green thread; section 9
 * routes Wine's sleeps to green.c, so one that gets here stops loudly. */
void emscripten_thread_sleep( double msecs )
{
    tw_unsupported( "emscripten_thread_sleep (a blocking sleep outside green.c)" );
}

/* libc's dlerror finds its per-thread buffer through the thread pointer,
 * which only Emscripten's pthread library provides. The Unix side links its
 * unixlibs statically and never dlopens (wine-unix.md, section 1), so
 * reaching dlerror is a bug. */
void *__get_tp( void )
{
    tw_unsupported( "__get_tp (dlerror)" );
}

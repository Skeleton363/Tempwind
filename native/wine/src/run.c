/*
 * The run loop (docs/design/wine-unix.md, section 12): runs translated code
 * from a thread's state block. M3's WP5 writes it; until then it stops
 * loudly, so run mode can't pass for working.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <stdint.h>

#include "tempwind_private.h"

void tw_run_loop( uint64_t state )
{
    tw_unsupported( "tw_run_loop (M3 WP5)" );
}

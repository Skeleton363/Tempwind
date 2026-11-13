/*
 * The boot export and the startup glue (docs/design/wine-unix.md, section 8).
 *
 * M2 WP1 skeleton: the host entries exist and stop loudly until WP4.
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

#define BOOT_STOP_AT_ENTRY 1

/* boot(argc, argv, flags), entered through WebAssembly.promising: runs
 * Wine's __wine_main and returns 0 once stopped at the entry. */
int boot( int argc, char **argv, int flags )
{
    tw_unsupported( "boot" );
}

/* The main thread's state block, once boot has stopped at the entry. */
void *tw_main_state( void )
{
    tw_unsupported( "tw_main_state" );
}

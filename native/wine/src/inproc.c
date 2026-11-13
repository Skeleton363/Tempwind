/*
 * The in-process server transport (docs/design/wine-unix.md, section 7).
 *
 * M2 WP1 skeleton: the entry points stop loudly until WP3 owns them.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include "tempwind_private.h"

/* Starts the server in the calling thread, in place of server_connect: what
 * the server's main would run, then the first process and thread. */
void inproc_server_start( void )
{
    tw_unsupported( "inproc_server_start" );
}

/* After send_request: the server reads and handles the request of the
 * current thread, synchronously (read_request). */
void inproc_handle_request( void )
{
    tw_unsupported( "inproc_handle_request" );
}

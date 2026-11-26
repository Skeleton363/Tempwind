/*
 * The `session` checks (docs/design/wine-unix.md, section 10, "The M4
 * checks"): the session's shared memory as one memory (section 7, patches
 * 0203 and 0401). The server's session blocks are ranges of one reservation
 * in the host heap; tw_session_view gives win32u the same bytes.
 *
 * The setup is syscall.c's: virtual_init, the first TEB, dbg_init, the
 * configuration directory, then server_init_process, which starts the
 * in-process server; its init_directories sets the session up
 * (server/directory.c L501-502). The checks then make the server write
 * shared objects with its own alloc_shared_object (server/mapping.c
 * L1431-1459), as its requests do, and read them back through
 * tw_session_view and through win32u's find_shared_session_object
 * (win32u/winstation.c L178-190). The JS test puts the prefix and
 * l_intl.nls into MEMFS first.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include "config.h"

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
#include "wine/server.h"

#include "harness.h"
#include "tempwind_private.h"
#include "server.h"

/* The server's (server/mapping.c, server/file.h): its session header and
 * its shared-object allocator. */
extern session_shm_t *shared_session;
volatile void *alloc_shared_object( mem_size_t shm_size );
struct obj_locator get_shared_object_locator( volatile void *object_shm );
/* win32u's (win32u_private.h L313; winstation.c L178): what init_user fills
 * (class.c L250), and its lookup of a shared object by id and offset. */
extern SYSTEM_BASIC_INFORMATION system_info;
const shared_object_t *find_shared_session_object( object_id_t id, mem_size_t offset );

/* The first block's size, as create_session_mapping computes it
 * (server/mapping.c L1344-1347; the page size is 0x1000, section 7). */
static size_t first_block_size( void )
{
    size_t size = sizeof(session_shm_t) + sizeof(object_shm_t) * 512;

    if (size < 0x10000) size = 0x10000;
    return (size + 0xfff) & ~(size_t)0xfff;
}

/* An object the server allocated: its shared_object_t through the view of
 * its offset rounded down to 64 KiB, as win32u's map_shared_session_block
 * takes it (winstation.c L113). */
static const char *check_object( volatile void *shm, const char *what )
{
    struct obj_locator loc = get_shared_object_locator( shm );
    const shared_object_t *object = CONTAINING_RECORD( shm, shared_object_t, shm );
    size_t off = loc.offset & ~(size_t)0xffff, size = 0;
    const char *view = tw_session_view( off, &size );
    const shared_object_t *seen = (const shared_object_t *)(view + loc.offset - off);

    if (seen != object) return tw_fail( "%s: the view at %#zx doesn't hold the server's object", what, off );
    if (size != TW_SESSION_CAP - off) return tw_fail( "%s: view size %#zx at %#zx", what, size, off );
    if (seen->id != loc.id || !loc.id) return tw_fail( "%s: id %llu, the server wrote %llu", what,
                                                        (unsigned long long)seen->id, (unsigned long long)loc.id );
    if (seen->seq & 1) return tw_fail( "%s: the server's write is still open (seq %lld)", what, (long long)seen->seq );
    /* win32u's lookup through its block cache finds the same bytes */
    if (find_shared_session_object( loc.id, loc.offset ) != object)
        return tw_fail( "%s: win32u's find_shared_session_object doesn't find it", what );
    return NULL;
}

/* ---- the checks ---- */

/* The start of the reservation is the server's session header, and an
 * object the server allocates in the first block reads back through the
 * view. */
static const char *view_start( void )
{
    volatile void *shm;
    struct obj_locator loc;
    size_t size = 0;
    void *view = tw_session_view( 0, &size );

    TW_EXPECT( view == (void *)shared_session );
    TW_EXPECT( size == TW_SESSION_CAP );
    TW_EXPECT( ((uintptr_t)view & 0xffff) == 0 );
    TW_EXPECT( (shm = alloc_shared_object( sizeof(desktop_shm_t) )) );
    loc = get_shared_object_locator( shm );
    TW_EXPECT( loc.offset >= sizeof(session_shm_t) && loc.offset < first_block_size() );
    return check_object( shm, "first block" );
}

/* An object too big for the first block makes the server grow the mapping
 * (grow_session_mapping, L1375-1406): it lands in the range right after the
 * first block, and reads back through a view of its own offset and through
 * win32u's first block, which runs to the reservation's end. */
static const char *view_grown_block( void )
{
    size_t first = first_block_size();
    volatile void *shm;
    struct obj_locator loc;
    const char *detail;

    TW_EXPECT( (shm = alloc_shared_object( first )) );
    loc = get_shared_object_locator( shm );
    TW_EXPECT( loc.offset >= first && loc.offset < TW_SESSION_CAP );
    if ((detail = check_object( shm, "grown block" ))) return detail;
    /* a second small object still fits the first block */
    TW_EXPECT( (shm = alloc_shared_object( sizeof(desktop_shm_t) )) );
    loc = get_shared_object_locator( shm );
    TW_EXPECT( loc.offset < first );
    return check_object( shm, "first block after the growth" );
}

/* A view at the cap stops loudly (section 7). Alone. */
static const char *view_past_cap( void )
{
    size_t size;

    tw_session_view( TW_SESSION_CAP, &size );
    return tw_fail( "tw_session_view returned" );
}

/* Growth past the cap stops loudly rather than moving the memory win32u
 * already points into (section 7). Alone. */
static const char *grow_past_cap( void )
{
    alloc_shared_object( TW_SESSION_CAP );
    return tw_fail( "alloc_shared_object returned" );
}

/* A view before the server has set the session up stops loudly. Alone, and
 * run before the setup (tw_test_main). */
static const char *view_before_setup( void )
{
    size_t size;

    tw_session_view( 0, &size );
    return tw_fail( "tw_session_view returned" );
}

static const struct tw_check checks[] =
{
    { "view_start", view_start },
    { "view_grown_block", view_grown_block },
    { "view_past_cap", view_past_cap, TW_ALONE },
    { "grow_past_cap", grow_past_cap, TW_ALONE },
    { "view_before_setup", view_before_setup, TW_ALONE },
};

int tw_test_main( void )
{
    const char *only = getenv( "TW_TEST_ONLY" );

    if (only && !strcmp( only, "view_before_setup" ))
        return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
    tw_server_start();
    NtQuerySystemInformation( SystemBasicInformation, &system_info, sizeof(system_info), NULL );
    return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
}

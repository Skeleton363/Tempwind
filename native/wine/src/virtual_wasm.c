/*
 * The memory backend under virtual.c (docs/design/wine-unix.md, sections 5
 * and 6).
 *
 * M2 WP1: the one memory grower and the heap cap. The maps, the page-state
 * table and pointer validation are WP2's; tw_page_flags stops loudly until
 * then.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <errno.h>
#include <stdint.h>
#include <unistd.h>

#include "tempwind_private.h"

#define WASM_PAGE 0x10000ull
#define MAX_PAGES 262144ull               /* 16 GiB, the memory's maximum */
#define HOST_REGION_END 0x100000000ull    /* the heap's cap (section 5) */

/* The only memory.grow in the module: grows the memory to cover end, rounded
 * up to 64 KiB, then tells JS (memory_grown). */
int grow_to( uint64_t end )
{
    uint64_t pages = (end + WASM_PAGE - 1) / WASM_PAGE;
    uint64_t cur = __builtin_wasm_memory_size( 0 );

    if (end > MAX_PAGES * WASM_PAGE)
    {
        errno = ENOMEM;
        return -1;
    }
    if (pages <= cur) return 0;
    if (__builtin_wasm_memory_grow( 0, pages - cur ) == (size_t)-1)
    {
        errno = ENOMEM;
        return -1;
    }
    memory_grown();
    return 0;
}

/* -Wl,--wrap=sbrk: the heap never grows the memory. The break stays below
 * the end of the host region, which INITIAL_MEMORY (4 GiB) covers, so
 * emscripten_resize_heap is never reached; past it malloc returns NULL. */
void *__real_sbrk( intptr_t increment );

void *__wrap_sbrk( intptr_t increment )
{
    uintptr_t brk = (uintptr_t)__real_sbrk( 0 );

    if (increment > 0 && (uint64_t)increment > HOST_REGION_END - brk)
    {
        errno = ENOMEM;
        return (void *)-1;
    }
    return __real_sbrk( increment );
}

/* The page-state table's address (abi.md v5): WP2. */
void *tw_page_flags( void )
{
    tw_unsupported( "tw_page_flags" );
}

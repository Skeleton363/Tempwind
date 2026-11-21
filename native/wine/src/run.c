/*
 * The run loop (docs/design/wine-unix.md, section 12; abi.md v5, "Who calls
 * whom"): runs translated code from a thread's state block through the
 * bridge's run (the import tw_run), and acts on each exit reason.
 *
 * - EXIT_LEFT_CODE: the run of committed executable pages around rip, in
 *   rip's view (virtual.c's pages_vprot, VPROT_EXEC: tw_exec_range, patch
 *   0106), goes to tw_translate, and the loop goes on. With no such run, or
 *   when tw_translate fails, it stops loudly: on Windows the first is an
 *   execute access violation, which M5 raises.
 * - EXIT_HOST: the host function that stopped recorded why in
 *   tw_host_stop_reason, and the loop returns to its caller.
 * - EXIT_FAULT and EXIT_UNSUPPORTED (store-check hits included) stop loudly
 *   until M5 and Phase 4.
 *
 * Process exit never comes back here: the exit hook (patch 0302) longjmps
 * to boot through the translated frames.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "tempwind/abi.h"
#include "tempwind_private.h"

const char *tw_host_stop_reason;

#define S64(s, off) (*(uint64_t *)(uintptr_t)((s) + (off)))
#define S32(s, off) (*(uint32_t *)(uintptr_t)((s) + (off)))

static const char *exit_name( uint32_t exit )
{
    switch (exit)
    {
    case TW_EXIT_LEFT_CODE:   return "EXIT_LEFT_CODE";
    case TW_EXIT_UNSUPPORTED: return "EXIT_UNSUPPORTED";
    case TW_EXIT_FAULT:       return "EXIT_FAULT";
    case TW_EXIT_HOST:        return "EXIT_HOST";
    default:                  return "an unknown exit";
    }
}

static const char *fault_name( uint32_t kind )
{
    switch (kind)
    {
    case 0:                              return "none";
    case TW_FAULT_READ:                  return "read";
    case TW_FAULT_WRITE:                 return "write";
    case TW_FAULT_EXECUTE:               return "execute";
    case TW_FAULT_DIVIDE:                return "divide";
    case TW_FAULT_INVALID_OPCODE:        return "invalid-opcode";
    case TW_FAULT_GENERAL_PROTECTION:    return "general-protection";
    default:                             return "unknown";
    }
}

/* The page-state byte of addr (abi.md, "Page-state table"); the table covers
 * the 16 GiB the memory can reach. */
static unsigned int page_state( uint64_t addr )
{
    if (addr >> 34) return 0;
    return ((const uint8_t *)tw_page_flags())[addr >> TW_PAGE_FLAGS_SHIFT];
}

/* The loud stop (section 12): the exit, rip, the fault address and kind,
 * and the page-state byte and Wine's protection byte of the fault address
 * (of rip for any exit other than EXIT_FAULT). */
static _Noreturn void stop( uint64_t state, uint32_t exit, const char *why )
{
    uint64_t rip = S64( state, TW_STATE_RIP );
    uint64_t addr = S64( state, TW_STATE_FAULT_ADDR );
    uint32_t kind = S32( state, TW_STATE_FAULT_KIND );
    uint64_t at = exit == TW_EXIT_FAULT ? addr : rip;
    uint64_t base, size;
    unsigned int vprot = tw_exec_range( at, &base, &size );

    fprintf( stderr, "tempwind: unsupported: run %s rip=%#llx fault=%#llx kind=%s page_state=%#x vprot=%#x%s%s\n",
             exit_name( exit ), (unsigned long long)rip, (unsigned long long)addr, fault_name( kind ),
             page_state( at ), vprot, why ? ": " : "", why ? why : "" );
    abort();
}

void tw_run_loop( uint64_t state )
{
    for (;;)
    {
        uint32_t exit = (uint32_t)tw_run( state );
        uint64_t rip, base, size;

        switch (exit)
        {
        case TW_EXIT_LEFT_CODE:
            rip = S64( state, TW_STATE_RIP );
            tw_exec_range( rip, &base, &size );
            if (!size) stop( state, exit, "no executable view at rip" );
            if (tw_translate( rip, base, size )) stop( state, exit, "tw_translate failed" );
            continue;
        case TW_EXIT_HOST:
            if (!tw_host_stop_reason) stop( state, exit, "no host function recorded a reason" );
            return;
        case TW_EXIT_UNSUPPORTED:
            /* the translator records no address for a store-check hit
             * (translator.md, "Memory checks"): rip is the store's */
            stop( state, exit, "an unsupported instruction or a store-check hit" );
        default:
            stop( state, exit, NULL );
        }
    }
}

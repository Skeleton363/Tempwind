/*
 * The run loop (docs/design/wine-unix.md, section 12; abi.md v5, "Who calls
 * whom"): runs translated code from a thread's state block through the
 * bridge's run (the import tw_run), and acts on each exit reason.
 *
 * - EXIT_LEFT_CODE: the run of committed executable pages around rip, in
 *   rip's view (virtual.c's pages_vprot, VPROT_EXEC: tw_exec_range, patch
 *   0106), goes to tw_translate, and the loop goes on; when tw_translate
 *   fails, it stops loudly. With no such run it raises an execute access
 *   violation, except at the dispatchers' addresses (abi.md, "Addresses"),
 *   where it stops loudly: the runtime lost a slot.
 * - EXIT_HOST: the host function that stopped recorded why in
 *   tw_host_stop_reason, and the loop returns to its caller.
 * - EXIT_FAULT raises the record of section 12, "The records", through
 *   tw_raise_exception (signal_wasm64.c), and the loop goes on at
 *   KiUserExceptionDispatcher. A fault kind it doesn't know, and a
 *   store-check fault on a page Wine has writable (writable code and
 *   write-watched pages wait for Phase 4, section 13), stop loudly.
 * - EXIT_UNSUPPORTED stops loudly.
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
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winternl.h"

#include "tempwind/abi.h"
#include "tempwind_private.h"

const char *tw_host_stop_reason;

#define S64(s, off) (*(uint64_t *)(uintptr_t)((s) + (off)))
#define S32(s, off) (*(uint32_t *)(uintptr_t)((s) + (off)))

/* virtual.c's per-page protection flags (L135-141), which tw_exec_range
 * returns. */
#define VPROT_WRITE      0x02
#define VPROT_WRITECOPY  0x08

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
    case TW_FAULT_BREAKPOINT:            return "breakpoint";
    case TW_FAULT_CODE_WRITE:            return "code-write";
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

/* ---- the records (section 12, "The records"); sx.c is
 * dlls/ntdll/unix/signal_x86_64.c ---- */

/* An access violation (sx.c L2431-2434): [0] = read 0, write 1 or execute 8,
 * [1] = the address. */
static void access_violation( EXCEPTION_RECORD *rec, ULONG_PTR access, ULONG_PTR addr )
{
    rec->ExceptionCode = STATUS_ACCESS_VIOLATION;  /* EXCEPTION_ACCESS_VIOLATION */
    rec->NumberParameters = 2;
    rec->ExceptionInformation[0] = access;
    rec->ExceptionInformation[1] = addr;
}

/* An execute fault's [0] (sx.c L2440-2447): EXCEPTION_EXECUTE_FAULT when the
 * process's execute flags have MEM_EXECUTE_OPTION_DISABLE (DEP; Wine's
 * default, unix/process.c L77), else EXCEPTION_READ_FAULT. */
static ULONG_PTR execute_access( void )
{
    ULONG flags = 0;

    NtQueryInformationProcess( NtCurrentProcess(), ProcessExecuteFlags, &flags, sizeof(flags), NULL );
    return (flags & MEM_EXECUTE_OPTION_DISABLE) ? EXCEPTION_EXECUTE_FAULT : EXCEPTION_READ_FAULT;
}

/* An EXIT_FAULT's record by its kind, or the loud stop. */
static void fault_record( uint64_t state, EXCEPTION_RECORD *rec )
{
    uint64_t addr = S64( state, TW_STATE_FAULT_ADDR ), base, size;

    switch (S32( state, TW_STATE_FAULT_KIND ))
    {
    case TW_FAULT_READ:  /* the NULL check: no view below 0x10000 (virtual.c L4538-4595) */
        access_violation( rec, EXCEPTION_READ_FAULT, addr );
        break;
    case TW_FAULT_WRITE:
        access_violation( rec, EXCEPTION_WRITE_FAULT, addr );
        break;
    case TW_FAULT_EXECUTE:
        access_violation( rec, execute_access(), addr );
        break;
    case TW_FAULT_DIVIDE:  /* FPE_INTDIV (sx.c L2531-2532), a zero divisor and an overflow alike */
        rec->ExceptionCode = STATUS_INTEGER_DIVIDE_BY_ZERO;  /* EXCEPTION_INT_DIVIDE_BY_ZERO */
        break;
    case TW_FAULT_INVALID_OPCODE:  /* TRAP_x86_PRIVINFLT (sx.c L2411-2413) */
        rec->ExceptionCode = STATUS_ILLEGAL_INSTRUCTION;  /* EXCEPTION_ILLEGAL_INSTRUCTION */
        break;
    case TW_FAULT_GENERAL_PROTECTION:  /* TRAP_x86_PROTFLT, error code 0, not privileged (sx.c L2417-2428) */
        access_violation( rec, 0, ~(ULONG_PTR)0 );
        break;
    case TW_FAULT_BREAKPOINT:  /* TRAP_x86_BPTFLT (sx.c L2500-2507) */
        rec->ExceptionCode = STATUS_BREAKPOINT;  /* EXCEPTION_BREAKPOINT */
        rec->NumberParameters = 1;
        rec->ExceptionInformation[0] = 0;
        break;
    case TW_FAULT_CODE_WRITE:
        /* natively such a store faults when Wine's page has neither
         * VPROT_WRITE nor VPROT_WRITECOPY, since the host page then has no
         * PROT_WRITE (virtual.c L1353-1366): image code, as on Windows. A
         * store to code Wine has writable, or to a write-watched page,
         * succeeds natively: Phase 4 (section 13) */
        if (tw_exec_range( addr, &base, &size ) & (VPROT_WRITE | VPROT_WRITECOPY))
            stop( state, TW_EXIT_FAULT, "a store-check fault on a page Wine has writable" );
        access_violation( rec, EXCEPTION_WRITE_FAULT, addr );
        break;
    default:
        stop( state, TW_EXIT_FAULT, "a fault kind the run loop doesn't know" );
    }
}

void tw_run_loop( uint64_t state )
{
    for (;;)
    {
        uint32_t exit = (uint32_t)tw_run( state );
        uint64_t rip = S64( state, TW_STATE_RIP ), base, size;
        /* ExceptionFlags 0, ExceptionAddress the faulting instruction, as
         * Wine's handlers take RIP (sx.c L2400, L2521; L2492 and L2501) */
        EXCEPTION_RECORD rec = { .ExceptionAddress = (void *)(uintptr_t)rip };

        switch (exit)
        {
        case TW_EXIT_LEFT_CODE:
            tw_exec_range( rip, &base, &size );
            if (size)
            {
                if (tw_translate( rip, base, size )) stop( state, exit, "tw_translate failed" );
                continue;
            }
            /* the dispatchers' slots are lookup keys, not code (abi.md,
             * "Addresses"): reaching one means the runtime lost its slot */
            if (rip == TW_SYSCALL_DISPATCHER || rip == TW_UNIX_CALL_DISPATCHER)
                stop( state, exit, "no slot at a dispatcher's address" );
            /* the fetch's page fault (sx.c L2440-2447) */
            access_violation( &rec, execute_access(), rip );
            break;
        case TW_EXIT_HOST:
            if (!tw_host_stop_reason) stop( state, exit, "no host function recorded a reason" );
            return;
        case TW_EXIT_FAULT:
            fault_record( state, &rec );
            break;
        case TW_EXIT_UNSUPPORTED:
            stop( state, exit, "an unsupported instruction" );
        default:
            stop( state, exit, NULL );
        }
        /* the state block now enters KiUserExceptionDispatcher (section 11) */
        tw_raise_exception( state, &rec );
    }
}

/*
 * The `virtual` harness (docs/design/wine-unix.md, section 10): Wine's
 * virtual memory on the wasm memory, without the server. virtual_init and
 * the first TEB (with the KUSER page), then reserve, commit and free through
 * NtAllocateVirtualMemory and NtFreeVirtualMemory, and the backend under
 * them (sections 4-6). tests/wine/virtual.test.mjs runs it.
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
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winnt.h"
#include "winternl.h"
#include "unix_private.h"

#include "harness.h"
#include "tempwind_private.h"

#define KUSER             0x7ffe0000ull
#define LOW_REGION_END    0x7ffe0000ull
#define HOST_REGION_START 0x80000000ull
#define HIGH_REGION_START 0x100000000ull
#define GUEST_END         0x400000000ull
#define WASM_PAGE         0x10000ull

static uint64_t memory_bytes( void )
{
    return (uint64_t)__builtin_wasm_memory_size( 0 ) * WASM_PAGE;
}

/* What boot does first (section 8): virtual_init, then the first TEB. Every
 * check starts here, so a check also runs alone (TW_TEST_ONLY). */
static TEB *first_teb;

static void ensure_init( void )
{
    if (first_teb) return;
    virtual_init();
    first_teb = virtual_alloc_first_teb();
}

static NTSTATUS query( uint64_t addr, MEMORY_BASIC_INFORMATION *info )
{
    SIZE_T len;

    memset( info, 0xcc, sizeof(*info) );
    return NtQueryVirtualMemory( NtCurrentProcess(), (void *)(uintptr_t)addr, MemoryBasicInformation,
                                 info, sizeof(*info), &len );
}

/* Fails unless addr's region has the given base, size, state, protection
 * and type. */
#define EXPECT_REGION(addr, base_, size_, state_, protect_, type_) do { \
        MEMORY_BASIC_INFORMATION mbi_; \
        TW_EXPECT( !query( (addr), &mbi_ ) ); \
        if ((uintptr_t)mbi_.BaseAddress != (base_) || mbi_.RegionSize != (size_) || \
            mbi_.State != (state_) || mbi_.Protect != (protect_) || mbi_.Type != (type_)) \
            return tw_fail( "%s:%d: region at %#llx: base %p size %#zx state %#x protect %#x type %#x", \
                            __FILE__, __LINE__, (unsigned long long)(addr), mbi_.BaseAddress, \
                            (size_t)mbi_.RegionSize, (unsigned)mbi_.State, (unsigned)mbi_.Protect, \
                            (unsigned)mbi_.Type ); \
    } while (0)

/* Fails unless addr is free. */
#define EXPECT_FREE(addr) do { \
        MEMORY_BASIC_INFORMATION mbi_; \
        TW_EXPECT( !query( (addr), &mbi_ ) ); \
        if (mbi_.State != MEM_FREE || (uintptr_t)mbi_.BaseAddress != (addr)) \
            return tw_fail( "%s:%d: %#llx: base %p state %#x, not free", __FILE__, __LINE__, \
                            (unsigned long long)(addr), mbi_.BaseAddress, (unsigned)mbi_.State ); \
    } while (0)

static NTSTATUS alloc( uint64_t *base, SIZE_T size, ULONG type, ULONG protect )
{
    void *ptr = (void *)(uintptr_t)*base;
    NTSTATUS status = NtAllocateVirtualMemory( NtCurrentProcess(), &ptr, 0, &size, type, protect );

    *base = (uintptr_t)ptr;
    return status;
}

static NTSTATUS release( uint64_t base, SIZE_T size, ULONG type )
{
    void *ptr = (void *)(uintptr_t)base;

    return NtFreeVirtualMemory( NtCurrentProcess(), &ptr, &size, type );
}

/* virtual_init and the first TEB: the TEB and the PEB in the low region,
 * committed; NtCurrentTeb follows; the memory did not grow, since Wine's
 * bookkeeping is in the host heap (section 4). */
static const char *init_first_teb( void )
{
    uint64_t teb, peb_addr;

    ensure_init();
    teb = (uintptr_t)first_teb;
    peb_addr = (uintptr_t)peb;
    TW_EXPECT( first_teb && peb );
    TW_EXPECT( NtCurrentTeb() == first_teb );
    TW_EXPECT( first_teb->Peb == peb );
    TW_EXPECT( first_teb->Tib.Self == &first_teb->Tib );
    TW_EXPECT( teb >= 0x10000 && teb < LOW_REGION_END && !(teb & 0xfff) );
    TW_EXPECT( peb_addr >= 0x10000 && peb_addr < LOW_REGION_END && !(peb_addr & 0xfff) );
    EXPECT_REGION( teb, teb, 0x20000, MEM_COMMIT, PAGE_READWRITE, MEM_PRIVATE );
    TW_EXPECT( memory_bytes() == HIGH_REGION_START );
    return NULL;
}

/* The KUSER page is committed read-only at 0x7ffe0000 (section 6). */
static const char *kuser_committed( void )
{
    ensure_init();
    TW_EXPECT( user_shared_data == (void *)(uintptr_t)KUSER );
    EXPECT_REGION( KUSER, KUSER, 0x1000, MEM_COMMIT, PAGE_READONLY, MEM_PRIVATE );
    return NULL;
}

/* Reserve, commit part, write, decommit, release. */
static const char *reserve_commit_free( void )
{
    uint64_t base = 0, part;
    volatile uint32_t *p;

    ensure_init();
    TW_EXPECT( !alloc( &base, 0x30000, MEM_RESERVE, PAGE_READWRITE ) );
    TW_EXPECT( base >= 0x10000 && base + 0x30000 <= LOW_REGION_END && !(base & 0xffff) );
    EXPECT_REGION( base, base, 0x30000, MEM_RESERVE, 0, MEM_PRIVATE );

    part = base + 0x10000;
    TW_EXPECT( !alloc( &part, 0x2000, MEM_COMMIT, PAGE_READWRITE ) );
    TW_EXPECT( part == base + 0x10000 );
    EXPECT_REGION( base, base, 0x10000, MEM_RESERVE, 0, MEM_PRIVATE );
    EXPECT_REGION( part, part, 0x2000, MEM_COMMIT, PAGE_READWRITE, MEM_PRIVATE );
    EXPECT_REGION( part + 0x2000, part + 0x2000, 0x1e000, MEM_RESERVE, 0, MEM_PRIVATE );
    p = (volatile uint32_t *)(uintptr_t)part;
    TW_EXPECT( p[0] == 0 && p[0x7ff] == 0 );
    p[0] = 0x11223344;
    p[0x7ff] = 0x55667788;
    TW_EXPECT( p[0] == 0x11223344 && p[0x7ff] == 0x55667788 );

    TW_EXPECT( !release( part, 0x2000, MEM_DECOMMIT ) );
    EXPECT_REGION( base, base, 0x30000, MEM_RESERVE, 0, MEM_PRIVATE );
    TW_EXPECT( !release( base, 0, MEM_RELEASE ) );
    EXPECT_FREE( base );
    return NULL;
}

/* An allocation at a fixed address in the high region grows the memory to
 * cover it (section 5); its pages read zero and take writes. */
static const char *fixed_high_grows( void )
{
    uint64_t base = 0x140000000ull;
    volatile uint64_t *p;

    ensure_init();
    TW_EXPECT( !alloc( &base, 0x20000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE ) );
    TW_EXPECT( base == 0x140000000ull );
    TW_EXPECT( memory_bytes() >= base + 0x20000 );
    EXPECT_REGION( base, base, 0x20000, MEM_COMMIT, PAGE_READWRITE, MEM_PRIVATE );
    p = (volatile uint64_t *)(uintptr_t)base;
    TW_EXPECT( p[0] == 0 && p[0x3fff] == 0 );
    p[0x3fff] = 0x0123456789abcdefull;
    TW_EXPECT( p[0x3fff] == 0x0123456789abcdefull );
    TW_EXPECT( !release( base, 0, MEM_RELEASE ) );
    EXPECT_FREE( base );
    return NULL;
}

static const struct tw_check checks[] =
{
    { "init_first_teb", init_first_teb },
    { "kuser_committed", kuser_committed },
    { "reserve_commit_free", reserve_commit_free },
    { "fixed_high_grows", fixed_high_grows },
};

int tw_test_main( void )
{
    return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
}

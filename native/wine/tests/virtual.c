/*
 * The `virtual` harness (docs/design/wine-unix.md, section 10): Wine's
 * virtual memory on the wasm memory, without the server. virtual_init and
 * the first TEB (with the KUSER page), then reserve, commit and free through
 * NtAllocateVirtualMemory and NtFreeVirtualMemory, and the backend under
 * them (sections 4-6): zero on recommit, protect and query, write-watch and
 * PAGE_WATCH, the guest regions only, the heap cap, file views and pointer
 * validation. Image placement needs the server's sections, so the `section`
 * harness tests it. tests/wine/virtual.test.mjs runs this one.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <errno.h>
#include <fcntl.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winnt.h"
#include "winternl.h"
#include "ddk/wdm.h"
#include "unix_private.h"

#include "tempwind/abi.h"
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

/* The KUSER page is committed read-only at 0x7ffe0000, and
 * virtual_map_user_shared_data maps nothing over it: the guest and the
 * in-process server share that one page (section 6). */
static const char *kuser_committed( void )
{
    volatile KUSER_SHARED_DATA *data = (KUSER_SHARED_DATA *)(uintptr_t)KUSER;
    uint64_t before;

    ensure_init();
    TW_EXPECT( user_shared_data == (void *)(uintptr_t)KUSER );
    EXPECT_REGION( KUSER, KUSER, 0x1000, MEM_COMMIT, PAGE_READONLY, MEM_PRIVATE );
    before = memory_bytes();
    data->NtMajorVersion = 0x5a5a;
    virtual_map_user_shared_data();
    TW_EXPECT( data->NtMajorVersion == 0x5a5a );
    data->NtMajorVersion = 0;
    EXPECT_REGION( KUSER, KUSER, 0x1000, MEM_COMMIT, PAGE_READONLY, MEM_PRIVATE );
    TW_EXPECT( memory_bytes() == before );
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

/* Decommitted pages read zero when recommitted, and a released range
 * allocated again reads zero (section 6: Wine's decommit is a fixed map). */
static const char *zero_on_recommit( void )
{
    uint64_t base = 0, part, again;
    volatile uint8_t *p;

    ensure_init();
    TW_EXPECT( !alloc( &base, 0x20000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE ) );
    p = (volatile uint8_t *)(uintptr_t)base;
    memset( (void *)(uintptr_t)base, 0xab, 0x20000 );
    TW_EXPECT( !release( base + 0x10000, 0x10000, MEM_DECOMMIT ) );
    part = base + 0x10000;
    TW_EXPECT( !alloc( &part, 0x10000, MEM_COMMIT, PAGE_READWRITE ) );
    TW_EXPECT( p[0] == 0xab && p[0xffff] == 0xab );
    TW_EXPECT( p[0x10000] == 0 && p[0x18000] == 0 && p[0x1ffff] == 0 );
    TW_EXPECT( !release( base, 0, MEM_RELEASE ) );
    again = base;
    TW_EXPECT( !alloc( &again, 0x20000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE ) );
    TW_EXPECT( again == base );
    TW_EXPECT( p[0] == 0 && p[0xffff] == 0 );
    TW_EXPECT( !release( base, 0, MEM_RELEASE ) );
    return NULL;
}

/* NtProtectVirtualMemory changes what NtQueryVirtualMemory reports, page by
 * page; outside the guest regions reads as reserved (section 4). */
static const char *protect_query( void )
{
    uint64_t base = 0;
    void *ptr;
    SIZE_T size;
    ULONG old;
    MEMORY_BASIC_INFORMATION mbi;

    ensure_init();
    TW_EXPECT( !alloc( &base, 0x3000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE ) );
    ptr = (void *)(uintptr_t)(base + 0x1000);
    size = 0x1000;
    TW_EXPECT( !NtProtectVirtualMemory( NtCurrentProcess(), &ptr, &size, PAGE_READONLY, &old ) );
    TW_EXPECT( old == PAGE_READWRITE );
    EXPECT_REGION( base, base, 0x1000, MEM_COMMIT, PAGE_READWRITE, MEM_PRIVATE );
    EXPECT_REGION( base + 0x1000, base + 0x1000, 0x1000, MEM_COMMIT, PAGE_READONLY, MEM_PRIVATE );
    EXPECT_REGION( base + 0x2000, base + 0x2000, 0x1000, MEM_COMMIT, PAGE_READWRITE, MEM_PRIVATE );
    ptr = (void *)(uintptr_t)(base + 0x2000);
    size = 0x1000;
    TW_EXPECT( !NtProtectVirtualMemory( NtCurrentProcess(), &ptr, &size, PAGE_EXECUTE_READ, &old ) );
    TW_EXPECT( old == PAGE_READWRITE );
    EXPECT_REGION( base + 0x2000, base + 0x2000, 0x1000, MEM_COMMIT, PAGE_EXECUTE_READ, MEM_PRIVATE );
    ptr = (void *)(uintptr_t)base;
    size = 0x1000;
    TW_EXPECT( !NtProtectVirtualMemory( NtCurrentProcess(), &ptr, &size, PAGE_READWRITE | PAGE_GUARD, &old ) );
    EXPECT_REGION( base, base, 0x1000, MEM_COMMIT, PAGE_READWRITE | PAGE_GUARD, MEM_PRIVATE );
    TW_EXPECT( !release( base, 0, MEM_RELEASE ) );

    /* outside the guest regions */
    EXPECT_REGION( 0, 0, 0x10000, MEM_RESERVE, PAGE_NOACCESS, MEM_PRIVATE );
    EXPECT_REGION( 0x7fff0000, 0x7fff0000, HIGH_REGION_START - 0x7fff0000, MEM_RESERVE, PAGE_NOACCESS, MEM_PRIVATE );
    EXPECT_REGION( 0x90000000, 0x90000000, HIGH_REGION_START - 0x90000000, MEM_RESERVE, PAGE_NOACCESS, MEM_PRIVATE );
    TW_EXPECT( query( GUEST_END, &mbi ) == STATUS_INVALID_PARAMETER );
    /* a free region ends where the guest regions do */
    TW_EXPECT( !query( 0x3f0000000ull, &mbi ) );
    TW_EXPECT( mbi.State == MEM_FREE && (uintptr_t)mbi.BaseAddress + mbi.RegionSize == GUEST_END );
    return NULL;
}

static uint8_t page_flag( uint64_t addr )
{
    return ((uint8_t *)tw_page_flags())[addr >> TW_PAGE_FLAGS_SHIFT];
}

/* PAGE_WATCH follows Wine's write watches (section 6; abi.md, "Page-state
 * table"): set on commit, cleared by a recorded write (a write fault, or a
 * write check), set again by NtResetWriteWatch, cleared on release. */
static const char *write_watch_page_watch( void )
{
    uint64_t base = 0, plain = 0, i;
    EXCEPTION_RECORD rec;
    void *addrs[8];
    ULONG_PTR count = 8;
    ULONG granularity;

    ensure_init();
    TW_EXPECT( (uintptr_t)tw_page_flags() >= HOST_REGION_START &&
               (uintptr_t)tw_page_flags() < HIGH_REGION_START );
    TW_EXPECT( !alloc( &base, 0x4000, MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH, PAGE_READWRITE ) );
    for (i = 0; i < 4; i++) TW_EXPECT( page_flag( base + i * 0x1000 ) & TW_PAGE_WATCH );

    memset( &rec, 0, sizeof(rec) );
    rec.ExceptionCode = STATUS_ACCESS_VIOLATION;
    rec.NumberParameters = 2;
    rec.ExceptionInformation[0] = EXCEPTION_WRITE_FAULT;
    rec.ExceptionInformation[1] = base + 0x1008;
    TW_EXPECT( virtual_handle_fault( &rec, NULL ) == STATUS_SUCCESS );
    TW_EXPECT( virtual_check_buffer_for_write( (void *)(uintptr_t)(base + 0x2004), 8 ) );
    TW_EXPECT( page_flag( base ) & TW_PAGE_WATCH );
    TW_EXPECT( !(page_flag( base + 0x1000 ) & TW_PAGE_WATCH) );
    TW_EXPECT( !(page_flag( base + 0x2000 ) & TW_PAGE_WATCH) );
    TW_EXPECT( page_flag( base + 0x3000 ) & TW_PAGE_WATCH );

    TW_EXPECT( !NtGetWriteWatch( NtCurrentProcess(), 0, (void *)(uintptr_t)base, 0x4000, addrs, &count,
                                 &granularity ) );
    TW_EXPECT( count == 2 && granularity == 0x1000 );
    TW_EXPECT( (uintptr_t)addrs[0] == base + 0x1000 && (uintptr_t)addrs[1] == base + 0x2000 );
    TW_EXPECT( !NtResetWriteWatch( NtCurrentProcess(), (void *)(uintptr_t)base, 0x4000 ) );
    for (i = 0; i < 4; i++) TW_EXPECT( page_flag( base + i * 0x1000 ) & TW_PAGE_WATCH );

    TW_EXPECT( !alloc( &plain, 0x1000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE ) );
    TW_EXPECT( !(page_flag( plain ) & TW_PAGE_WATCH) );
    TW_EXPECT( !release( plain, 0, MEM_RELEASE ) );
    TW_EXPECT( !release( base, 0, MEM_RELEASE ) );
    for (i = 0; i < 4; i++) TW_EXPECT( !(page_flag( base + i * 0x1000 ) & TW_PAGE_WATCH) );
    return NULL;
}

/* Nothing maps outside the guest regions: not Wine (fixed allocations
 * fail), not the backend (fixed and try-fixed maps fail), and every
 * committed region lies in a guest region (section 4). */
static const char *guest_regions_only( void )
{
    static const uint64_t outside[][2] =
    {
        { 0x1000, 0x1000 },                  /* the NULL region */
        { 0x7fff0000, 0x10000 },             /* the gap */
        { 0x90000000, 0x10000 },             /* the host region */
        { GUEST_END, 0x10000 },              /* above 16 GiB */
        { GUEST_END - 0x10000, 0x20000 },    /* across 16 GiB */
    };
    uint8_t *heap = malloc( 0x2000 );
    volatile uint8_t *hp = (uint8_t *)(((uintptr_t)heap + 0xfff) & ~(uintptr_t)0xfff);
    uint64_t addr, spare = 0x60000000;
    MEMORY_BASIC_INFORMATION mbi;
    unsigned int i;

    ensure_init();
    TW_EXPECT( heap );
    for (i = 0; i < sizeof(outside) / sizeof(outside[0]); i++)
    {
        uint64_t base = outside[i][0];
        if (!alloc( &base, outside[i][1], MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE ))
            return tw_fail( "allocated %#llx at %#llx", (unsigned long long)outside[i][1],
                            (unsigned long long)base );
    }
    /* the backend refuses the host region, and leaves the heap alone */
    memset( (void *)hp, 0x3c, 0x1000 );
    errno = 0;
    TW_EXPECT( tw_mmap( (void *)hp, 0x1000, PROT_READ | PROT_WRITE,
                        MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0 ) == MAP_FAILED && errno == ENOMEM );
    errno = 0;
    TW_EXPECT( tw_mmap( (void *)hp, 0x1000, PROT_READ | PROT_WRITE,
                        MAP_PRIVATE | MAP_ANON | MAP_FIXED_NOREPLACE, -1, 0 ) == MAP_FAILED && errno == EEXIST );
    TW_EXPECT( tw_munmap( (void *)hp, 0x1000 ) == -1 && errno == EINVAL );
    TW_EXPECT( hp[0] == 0x3c && hp[0xfff] == 0x3c );
    free( heap );

    /* the guest regions start reserved: a try-fixed map and a map at no
     * fixed address find room only where something was unmapped */
    errno = 0;
    TW_EXPECT( tw_mmap( (void *)(uintptr_t)spare, 0x10000, PROT_READ | PROT_WRITE,
                        MAP_PRIVATE | MAP_ANON | MAP_FIXED_NOREPLACE, -1, 0 ) == MAP_FAILED && errno == EEXIST );
    errno = 0;
    TW_EXPECT( tw_mmap( NULL, 0x10000, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0 ) == MAP_FAILED &&
               errno == ENOMEM );
    TW_EXPECT( !tw_munmap( (void *)(uintptr_t)spare, 0x10000 ) );
    TW_EXPECT( tw_mmap( NULL, 0x10000, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANON, -1, 0 ) ==
               (void *)(uintptr_t)spare );
    TW_EXPECT( !tw_munmap( (void *)(uintptr_t)spare, 0x10000 ) );
    TW_EXPECT( tw_mmap( (void *)(uintptr_t)spare, 0x10000, PROT_NONE, MAP_PRIVATE | MAP_ANON | MAP_FIXED, -1, 0 ) ==
               (void *)(uintptr_t)spare );  /* reserved again */

    /* every committed region lies in a guest region */
    for (addr = 0; addr < GUEST_END; addr += mbi.RegionSize)
    {
        TW_EXPECT( !query( addr, &mbi ) && mbi.RegionSize );
        if (mbi.State != MEM_COMMIT) continue;
        if (!((addr >= 0x10000 && addr + mbi.RegionSize <= 0x7fff0000) ||
              (addr >= HIGH_REGION_START && addr + mbi.RegionSize <= GUEST_END)))
            return tw_fail( "committed %#llx-%#llx", (unsigned long long)addr,
                            (unsigned long long)(addr + mbi.RegionSize) );
    }
    return NULL;
}

/* Section 5: once the backend has grown the memory past 4 GiB, a malloc
 * that would cross 0x1_0000_0000 returns NULL and the memory stays. */
static const char *heap_cap_after_growth( void )
{
    uint64_t base = 0x150000000ull, grown;
    void *volatile big;

    ensure_init();
    TW_EXPECT( !alloc( &base, 0x10000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE ) );
    grown = memory_bytes();
    TW_EXPECT( grown > HIGH_REGION_START );
    errno = 0;
    big = malloc( (size_t)3 << 30 );
    TW_EXPECT( !big && errno == ENOMEM );
    TW_EXPECT( memory_bytes() == grown );
    TW_EXPECT( !release( base, 0, MEM_RELEASE ) );
    return NULL;
}

#define VIEW_FILE "/tmp/tw-view.bin"
#define VIEW_FILE_SIZE 0x2800

static uint8_t file_byte( unsigned int i ) { return (uint8_t)(i * 7 + 1); }

/* A new file of VIEW_FILE_SIZE bytes, opened with flags. */
static int make_file( int flags )
{
    static uint8_t buf[VIEW_FILE_SIZE];
    unsigned int i;
    int fd = open( VIEW_FILE, O_RDWR | O_CREAT | O_TRUNC, 0644 );

    if (fd < 0) return -1;
    for (i = 0; i < VIEW_FILE_SIZE; i++) buf[i] = file_byte( i );
    if (write( fd, buf, VIEW_FILE_SIZE ) != VIEW_FILE_SIZE) return -1;
    close( fd );
    return open( VIEW_FILE, flags );
}

static int read_file_byte( off_t offset )
{
    uint8_t b;
    int fd = open( VIEW_FILE, O_RDONLY ), ret;

    ret = pread( fd, &b, 1, offset ) == 1 ? b : -1;
    close( fd );
    return ret;
}

/* File views through the backend's own functions are copies: a read-only
 * view and a copy-on-write view take the file's bytes (zero past its end),
 * writes to them never reach the file, and read-only views may repeat. */
static const char *file_view_copy( void )
{
    uint64_t base = 0;
    volatile uint8_t *v;
    unsigned int i;
    int fd = make_file( O_RDONLY );

    ensure_init();
    TW_EXPECT( fd >= 0 );
    TW_EXPECT( !alloc( &base, 0x10000, MEM_RESERVE, PAGE_READWRITE ) );
    v = (volatile uint8_t *)(uintptr_t)base;
    TW_EXPECT( tw_mmap( (void *)v, 0x3000, PROT_READ, MAP_SHARED | MAP_FIXED, fd, 0 ) == (void *)v );
    for (i = 0; i < VIEW_FILE_SIZE; i++)
        if (v[i] != file_byte( i )) return tw_fail( "byte %#x: %#x", i, v[i] );
    for (i = VIEW_FILE_SIZE; i < 0x3000; i++)
        if (v[i]) return tw_fail( "byte %#x past the end: %#x", i, v[i] );
    TW_EXPECT( tw_mmap( (void *)(v + 0x4000), 0x1000, PROT_READ, MAP_SHARED | MAP_FIXED, fd, 0x1000 ) ==
               (void *)(v + 0x4000) );
    TW_EXPECT( v[0x4000] == file_byte( 0x1000 ) );
    TW_EXPECT( tw_mmap( (void *)(v + 0x8000), 0x1000, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_FIXED,
                        fd, 0x1000 ) == (void *)(v + 0x8000) );
    TW_EXPECT( v[0x8000] == file_byte( 0x1000 ) );
    v[0x8000] = 0xee;
    close( fd );
    TW_EXPECT( !release( base, 0, MEM_RELEASE ) );
    TW_EXPECT( read_file_byte( 0x1000 ) == file_byte( 0x1000 ) );
    return NULL;
}

/* A writable shared view writes back on NtFlushVirtualMemory and when Wine
 * releases its range, never past the file's end (section 6). */
static const char *file_view_write_back( void )
{
    uint64_t base = 0;
    volatile uint8_t *v;
    const void *addr;
    SIZE_T size = 0x3000;
    struct stat st;
    int fd = make_file( O_RDWR );

    ensure_init();
    TW_EXPECT( fd >= 0 );
    TW_EXPECT( !alloc( &base, 0x10000, MEM_RESERVE, PAGE_READWRITE ) );
    v = (volatile uint8_t *)(uintptr_t)base;
    TW_EXPECT( tw_mmap( (void *)v, 0x3000, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_FIXED, fd, 0 ) == (void *)v );
    close( fd );  /* the view keeps its own */
    TW_EXPECT( v[0x10] == file_byte( 0x10 ) );
    v[0x10] = 0x5a;
    v[0x2900] = 0x77;  /* past the end of the file */
    TW_EXPECT( read_file_byte( 0x10 ) == file_byte( 0x10 ) );
    addr = (const void *)v;
    TW_EXPECT( !NtFlushVirtualMemory( NtCurrentProcess(), &addr, &size, 0 ) );
    TW_EXPECT( read_file_byte( 0x10 ) == 0x5a );
    v[0x2000] = 0x66;
    TW_EXPECT( !release( base, 0, MEM_RELEASE ) );
    TW_EXPECT( read_file_byte( 0x2000 ) == 0x66 );
    TW_EXPECT( !stat( VIEW_FILE, &st ) && st.st_size == VIEW_FILE_SIZE );
    return NULL;
}

/* Alone: a second writable view of a file range that has one stops loudly
 * (section 6). */
static const char *second_writable_view( void )
{
    uint64_t base = 0;
    int fd = make_file( O_RDWR );

    ensure_init();
    TW_EXPECT( fd >= 0 );
    TW_EXPECT( !alloc( &base, 0x10000, MEM_RESERVE, PAGE_READWRITE ) );
    TW_EXPECT( tw_mmap( (void *)(uintptr_t)base, 0x2000, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_FIXED, fd, 0 ) ==
               (void *)(uintptr_t)base );
    tw_mmap( (void *)(uintptr_t)(base + 0x4000), 0x1000, PROT_READ | PROT_WRITE, MAP_SHARED | MAP_FIXED, fd, 0x1000 );
    return tw_fail( "the second writable view was mapped" );
}

/* The three __TRY sites check against the views (section 6). */
static const char *pointer_validation( void )
{
    uint64_t base = 0, part, gone = 0;
    uint8_t *p, *heap = malloc( 64 ), buf[16];
    ULONG old;
    void *ptr;
    SIZE_T size, done;

    ensure_init();
    TW_EXPECT( heap );
    TW_EXPECT( !alloc( &base, 0x5000, MEM_RESERVE, PAGE_READWRITE ) );
    p = (uint8_t *)(uintptr_t)base;
    part = base;
    TW_EXPECT( !alloc( &part, 0x1000, MEM_COMMIT, PAGE_READWRITE ) );
    part = base + 0x1000;
    TW_EXPECT( !alloc( &part, 0x1000, MEM_COMMIT, PAGE_READONLY ) );
    part = base + 0x2000;
    TW_EXPECT( !alloc( &part, 0x1000, MEM_COMMIT, PAGE_READWRITE ) );
    ptr = (void *)(uintptr_t)part;
    size = 0x1000;
    TW_EXPECT( !NtProtectVirtualMemory( NtCurrentProcess(), &ptr, &size, PAGE_READWRITE | PAGE_GUARD, &old ) );
    part = base + 0x4000;
    TW_EXPECT( !alloc( &part, 0x1000, MEM_COMMIT, PAGE_NOACCESS ) );
    /* base + 0x3000 stays reserved */
    TW_EXPECT( !alloc( &gone, 0x1000, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE ) );
    TW_EXPECT( !release( gone, 0, MEM_RELEASE ) );

    TW_EXPECT( virtual_check_buffer_for_read( p, 0x1000 ) );
    TW_EXPECT( virtual_check_buffer_for_read( p + 0xff0, 0x20 ) );       /* read-write into read-only */
    TW_EXPECT( virtual_check_buffer_for_read( p + 0x1000, 0x1000 ) );
    TW_EXPECT( !virtual_check_buffer_for_read( p, 0x2001 ) );            /* into the guard page */
    TW_EXPECT( !virtual_check_buffer_for_read( p + 0x3000, 1 ) );        /* reserved */
    TW_EXPECT( !virtual_check_buffer_for_read( p + 0x4000, 1 ) );        /* no access */
    TW_EXPECT( !virtual_check_buffer_for_read( (void *)(uintptr_t)gone, 1 ) );  /* free */
    TW_EXPECT( !virtual_check_buffer_for_read( (void *)0x1000, 4 ) );    /* the NULL region */
    TW_EXPECT( !virtual_check_buffer_for_read( NULL, 4 ) );
    TW_EXPECT( virtual_check_buffer_for_read( NULL, 0 ) );
    TW_EXPECT( virtual_check_buffer_for_read( heap, 64 ) );              /* the host region */
    TW_EXPECT( virtual_check_buffer_for_write( heap, 64 ) );
    TW_EXPECT( virtual_check_buffer_for_write( p, 0x1000 ) );
    TW_EXPECT( !virtual_check_buffer_for_write( p + 0xff0, 0x20 ) );     /* into read-only */
    TW_EXPECT( !virtual_check_buffer_for_write( p + 0x1000, 1 ) );
    TW_EXPECT( !virtual_check_buffer_for_write( p + 0x2000, 1 ) );       /* guard */
    TW_EXPECT( !virtual_check_buffer_for_write( p + 0x3000, 1 ) );
    EXPECT_REGION( base + 0x2000, base + 0x2000, 0x1000, MEM_COMMIT, PAGE_READWRITE | PAGE_GUARD, MEM_PRIVATE );

    memset( p, 0x42, 16 );
    memset( buf, 0, sizeof(buf) );
    TW_EXPECT( !NtReadVirtualMemory( NtCurrentProcess(), p, buf, 16, &done ) );
    TW_EXPECT( done == 16 && buf[0] == 0x42 && buf[15] == 0x42 );
    done = 99;
    TW_EXPECT( NtReadVirtualMemory( NtCurrentProcess(), p + 0x3000, buf, 16, &done ) == STATUS_PARTIAL_COPY );
    TW_EXPECT( done == 0 );
    TW_EXPECT( NtReadVirtualMemory( NtCurrentProcess(), p, p + 0x1000, 16, &done ) == STATUS_ACCESS_VIOLATION );
    TW_EXPECT( p[0x1000] == 0 );
    TW_EXPECT( !release( base, 0, MEM_RELEASE ) );
    free( heap );
    return NULL;
}

static const struct tw_check checks[] =
{
    { "init_first_teb", init_first_teb },
    { "kuser_committed", kuser_committed },
    { "reserve_commit_free", reserve_commit_free },
    { "fixed_high_grows", fixed_high_grows },
    { "zero_on_recommit", zero_on_recommit },
    { "protect_query", protect_query },
    { "write_watch_page_watch", write_watch_page_watch },
    { "guest_regions_only", guest_regions_only },
    { "heap_cap_after_growth", heap_cap_after_growth },
    { "file_view_copy", file_view_copy },
    { "file_view_write_back", file_view_write_back },
    { "pointer_validation", pointer_validation },
    { "second_writable_view", second_writable_view, TW_ALONE },
};

int tw_test_main( void )
{
    return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
}

/*
 * The `section` checks (docs/design/wine-unix.md, section 10): sections
 * through the in-process server. Image placement through NtCreateSection
 * and NtMapViewOfSection (section 4): at the preferred base, in a ranges64
 * slot when the base is taken, then a search in the image range. A
 * writable shared file view written back (section 6).
 *
 * The setup runs before the checks, as start_main_thread does it (loader.c):
 * virtual_init, the first TEB, dbg_init, the configuration directory (the
 * prefix, as set_config_dir does), then server_init_process, which starts
 * the server in this thread. The JS test puts the prefix with its
 * dosdevices/z: (z: is /), l_intl.nls and the images under /tempwind into
 * MEMFS first.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include "config.h"

#include <fcntl.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winnt.h"
#include "winternl.h"
#include "unix_private.h"

#include "harness.h"
#include "tempwind_private.h"

#define WASM_PAGE        0x10000ull
#define IMAGE_RANGE      0x100000000ull  /* the server's ranges64 (section 4) */
#define IMAGE_RANGE_END  0x170000000ull
#define DATA_FILE        "/tempwind/data.bin"
#define DATA_SIZE        0x3000

static uint64_t memory_bytes( void )
{
    return (uint64_t)__builtin_wasm_memory_size( 0 ) * WASM_PAGE;
}

/* Opens \??\Z:<path> (z: is /) with the given access. */
static HANDLE open_file( const char *path, ACCESS_MASK access )
{
    static const char prefix[] = "\\??\\Z:";
    WCHAR name[256];
    UNICODE_STRING str;
    OBJECT_ATTRIBUTES attr;
    IO_STATUS_BLOCK io;
    HANDLE file = 0;
    unsigned int i, n = 0;

    for (i = 0; prefix[i]; i++) name[n++] = prefix[i];
    for (i = 0; path[i] && n < 255; i++) name[n++] = path[i] == '/' ? '\\' : path[i];
    str.Buffer = name;
    str.Length = str.MaximumLength = n * sizeof(WCHAR);
    InitializeObjectAttributes( &attr, &str, OBJ_CASE_INSENSITIVE, 0, NULL );
    if (NtCreateFile( &file, access | SYNCHRONIZE, &attr, &io, NULL, 0, FILE_SHARE_READ | FILE_SHARE_WRITE,
                      FILE_OPEN, FILE_SYNCHRONOUS_IO_NONALERT | FILE_NON_DIRECTORY_FILE, NULL, 0 ))
        return 0;
    return file;
}

/* An image section of the file at path. */
static HANDLE image_section( const char *path )
{
    HANDLE file = open_file( path, GENERIC_READ | GENERIC_EXECUTE ), section = 0;

    if (!file) return 0;
    if (NtCreateSection( &section, SECTION_ALL_ACCESS, NULL, NULL, PAGE_EXECUTE_READ, SEC_IMAGE, file ))
        section = 0;
    NtClose( file );
    return section;
}

static NTSTATUS map( HANDLE section, uint64_t *base, ULONG protect )
{
    void *ptr = NULL;
    SIZE_T size = 0;
    NTSTATUS status = NtMapViewOfSection( section, NtCurrentProcess(), &ptr, 0, 0, NULL, &size, ViewShare,
                                          0, protect );

    *base = (uintptr_t)ptr;
    return status;
}

/* The image's SizeOfImage, read from its headers in memory. */
static uint32_t size_of_image( uint64_t base )
{
    const IMAGE_DOS_HEADER *dos = (const IMAGE_DOS_HEADER *)(uintptr_t)base;
    const IMAGE_NT_HEADERS64 *nt = (const IMAGE_NT_HEADERS64 *)(uintptr_t)(base + dos->e_lfanew);

    return nt->OptionalHeader.SizeOfImage;
}

/* Fails unless addr's view is an image view based at base. */
#define EXPECT_IMAGE(base) do { \
        MEMORY_BASIC_INFORMATION mbi_; \
        TW_EXPECT( !NtQueryVirtualMemory( NtCurrentProcess(), (void *)(uintptr_t)(base), MemoryBasicInformation, \
                                          &mbi_, sizeof(mbi_), NULL ) ); \
        TW_EXPECT( mbi_.Type == MEM_IMAGE && (uintptr_t)mbi_.AllocationBase == (base) ); \
        TW_EXPECT( ((const IMAGE_DOS_HEADER *)(uintptr_t)(base))->e_magic == IMAGE_DOS_SIGNATURE ); \
    } while (0)

/* The EXE maps at its preferred base (hello.exe: 0x1_4000_0000). */
static const char *image_exe_preferred_base( void )
{
    HANDLE section = image_section( "/tempwind/hello.exe" );
    uint64_t base;

    TW_EXPECT( section );
    TW_EXPECT( map( section, &base, PAGE_EXECUTE_READ ) == STATUS_SUCCESS );
    TW_EXPECT( base == 0x140000000ull );
    EXPECT_IMAGE( base );
    TW_EXPECT( memory_bytes() >= base + size_of_image( base ) );
    TW_EXPECT( !NtUnmapViewOfSection( NtCurrentProcess(), (void *)(uintptr_t)base ) );
    NtClose( section );
    return NULL;
}

/* A relocated DLL maps at its preferred base while that is free, first
 * (sas.dll: 0x1_8000_0000), before any server slot. When the base is taken,
 * a second DLL with the same base (softpub.dll) gets the server's first
 * ranges64 slot: assigned top-down, its size rounded to 64 KiB plus 64 KiB
 * of space, so it ends 64 KiB below 0x1_7000_0000 (server/mapping.c
 * assign_map_address). A second view of that DLL, with both taken, comes
 * from the search in the image range, top-down, so the memory stays small. */
static const char *image_dll_slot( void )
{
    HANDLE first = image_section( "/tempwind/sas.dll" ), second = image_section( "/tempwind/softpub.dll" );
    uint64_t base, slot, again, size;
    NTSTATUS status;

    TW_EXPECT( first && second );
    TW_EXPECT( map( first, &base, PAGE_EXECUTE_READ ) == STATUS_SUCCESS );
    TW_EXPECT( base == 0x180000000ull );
    EXPECT_IMAGE( base );

    /* at its slot the image is at its map address: STATUS_SUCCESS, as the
     * server compares with that (server/mapping.c, map_image_view) */
    if ((status = map( second, &slot, PAGE_EXECUTE_READ )) != STATUS_SUCCESS)
        return tw_fail( "softpub.dll: status %#x at %#llx", (unsigned)status, (unsigned long long)slot );
    EXPECT_IMAGE( slot );
    size = (size_of_image( slot ) + 0xffff) & ~0xffffull;
    if (slot != IMAGE_RANGE_END - size - 0x10000)
        return tw_fail( "softpub.dll at %#llx, not in the first ranges64 slot %#llx", (unsigned long long)slot,
                        (unsigned long long)(IMAGE_RANGE_END - size - 0x10000) );

    TW_EXPECT( map( second, &again, PAGE_EXECUTE_READ ) == STATUS_IMAGE_NOT_AT_BASE );
    EXPECT_IMAGE( again );
    if (again < IMAGE_RANGE || again + size > IMAGE_RANGE_END || again == slot)
        return tw_fail( "the second softpub.dll view at %#llx", (unsigned long long)again );
    TW_EXPECT( memory_bytes() < 0x200000000ull );

    TW_EXPECT( !NtUnmapViewOfSection( NtCurrentProcess(), (void *)(uintptr_t)again ) );
    TW_EXPECT( !NtUnmapViewOfSection( NtCurrentProcess(), (void *)(uintptr_t)slot ) );
    TW_EXPECT( !NtUnmapViewOfSection( NtCurrentProcess(), (void *)(uintptr_t)base ) );
    NtClose( first );
    NtClose( second );
    return NULL;
}

static uint8_t data_byte( unsigned int i ) { return (uint8_t)(i * 13 + 5); }

static int read_data_byte( off_t offset )
{
    uint8_t b;
    int fd = open( DATA_FILE, O_RDONLY ), ret;

    ret = pread( fd, &b, 1, offset ) == 1 ? b : -1;
    close( fd );
    return ret;
}

/* A writable shared view of a file section copies the file's bytes and
 * writes them back on NtFlushVirtualMemory and on NtUnmapViewOfSection. */
static const char *file_view_written_back( void )
{
    static uint8_t buf[DATA_SIZE];
    HANDLE file, section = 0;
    volatile uint8_t *v;
    const void *addr;
    uint64_t base;
    SIZE_T size = DATA_SIZE;
    unsigned int i;
    int fd = open( DATA_FILE, O_RDWR | O_CREAT | O_TRUNC, 0644 );

    TW_EXPECT( fd >= 0 );
    for (i = 0; i < DATA_SIZE; i++) buf[i] = data_byte( i );
    TW_EXPECT( write( fd, buf, DATA_SIZE ) == DATA_SIZE );
    close( fd );

    TW_EXPECT( (file = open_file( DATA_FILE, GENERIC_READ | GENERIC_WRITE )) );
    TW_EXPECT( !NtCreateSection( &section, SECTION_ALL_ACCESS, NULL, NULL, PAGE_READWRITE, SEC_COMMIT, file ) );
    NtClose( file );
    TW_EXPECT( !map( section, &base, PAGE_READWRITE ) );
    v = (volatile uint8_t *)(uintptr_t)base;
    for (i = 0; i < DATA_SIZE; i++)
        if (v[i] != data_byte( i )) return tw_fail( "byte %#x: %#x", i, v[i] );
    v[0x100] = 0xa1;
    TW_EXPECT( read_data_byte( 0x100 ) == data_byte( 0x100 ) );
    addr = (const void *)v;
    TW_EXPECT( !NtFlushVirtualMemory( NtCurrentProcess(), &addr, &size, 0 ) );
    TW_EXPECT( read_data_byte( 0x100 ) == 0xa1 );
    v[0x2fff] = 0xb2;
    TW_EXPECT( !NtUnmapViewOfSection( NtCurrentProcess(), (void *)v ) );
    TW_EXPECT( read_data_byte( 0x2fff ) == 0xb2 );
    NtClose( section );
    return NULL;
}

static const struct tw_check checks[] =
{
    { "image_exe_preferred_base", image_exe_preferred_base },
    { "image_dll_slot", image_dll_slot },
    { "file_view_written_back", file_view_written_back },
};

int tw_test_main( void )
{
    virtual_init();
    virtual_alloc_first_teb();
    dbg_init();
    config_dir = getenv( "WINEPREFIX" );  /* what set_config_dir does (loader.c) */
    server_init_process();
    return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
}

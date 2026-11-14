/*
 * The `boot` checks (docs/design/wine-unix.md, section 10): the Unix side
 * boots to LdrInitializeThunk without running guest code.
 *
 * tw_test_main boots as the JS host would (section 8): boot( 2, argv,
 * BOOT_STOP_AT_ENTRY ) with argv[1] = C:\tempwind\hello.exe, then checks the
 * state it stopped in. The JS test writes the boot set into MEMFS first
 * (runtime/src/wine/boot-set.mjs), and kuser.bin, native Wine 11's KUSER
 * page, at KUSER_ORACLE; it checks the list of files the boot opened itself.
 *
 * The checks find the images by walking the guest regions with
 * NtQueryVirtualMemory and read their headers and export tables in memory,
 * not through Wine's own bookkeeping.
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
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winnt.h"
#include "winternl.h"
#include "ddk/wdm.h"
#include "unix_private.h"

#include "harness.h"
#include "tempwind/abi.h"
#include "tempwind_private.h"

#define BOOT_STOP_AT_ENTRY 1
#define KUSER_ORACLE "/tempwind-test/kuser.bin"
#define KUSER_ADDR ((const BYTE *)0x7ffe0000)
#define SYSCALL_SLOT ((const UINT64 *)0x7ffe1000)

int boot( int argc, char **argv, int flags );
void *tw_main_state( void );

static int boot_ret = -1;

/* ---- reading images in memory ---- */

static const IMAGE_NT_HEADERS64 *nt_headers( const BYTE *base )
{
    const IMAGE_DOS_HEADER *dos = (const IMAGE_DOS_HEADER *)base;
    const IMAGE_NT_HEADERS64 *nt;

    if (dos->e_magic != IMAGE_DOS_SIGNATURE) return NULL;
    nt = (const IMAGE_NT_HEADERS64 *)(base + dos->e_lfanew);
    if (nt->Signature != IMAGE_NT_SIGNATURE || nt->OptionalHeader.Magic != IMAGE_NT_OPTIONAL_HDR64_MAGIC) return NULL;
    return nt;
}

static const IMAGE_EXPORT_DIRECTORY *exports( const BYTE *base )
{
    const IMAGE_NT_HEADERS64 *nt = nt_headers( base );
    const IMAGE_DATA_DIRECTORY *dir;

    if (!nt || nt->OptionalHeader.NumberOfRvaAndSizes <= IMAGE_DIRECTORY_ENTRY_EXPORT) return NULL;
    dir = &nt->OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_EXPORT];
    if (!dir->VirtualAddress || !dir->Size) return NULL;
    return (const IMAGE_EXPORT_DIRECTORY *)(base + dir->VirtualAddress);
}

/* The address of a named export, or 0. */
static ULONG_PTR find_export( const BYTE *base, const char *name )
{
    const IMAGE_EXPORT_DIRECTORY *exp = exports( base );
    const DWORD *names, *functions;
    const WORD *ordinals;
    DWORD i;

    if (!exp) return 0;
    names = (const DWORD *)(base + exp->AddressOfNames);
    ordinals = (const WORD *)(base + exp->AddressOfNameOrdinals);
    functions = (const DWORD *)(base + exp->AddressOfFunctions);
    for (i = 0; i < exp->NumberOfNames; i++)
        if (!strcmp( (const char *)base + names[i], name )) return (ULONG_PTR)base + functions[ordinals[i]];
    return 0;
}

/* The base of the MEM_IMAGE view whose export directory names `dll`, or
 * NULL: a walk over the guest regions. */
static const BYTE *find_image( const char *dll )
{
    MEMORY_BASIC_INFORMATION info;
    const BYTE *addr = (const BYTE *)0x10000;

    while ((ULONG_PTR)addr < 0x400000000 &&
           !NtQueryVirtualMemory( NtCurrentProcess(), addr, MemoryBasicInformation, &info, sizeof(info), NULL ))
    {
        if (info.Type == MEM_IMAGE && info.AllocationBase == info.BaseAddress)
        {
            const IMAGE_EXPORT_DIRECTORY *exp = exports( info.BaseAddress );
            if (exp && !strcmp( (const char *)info.BaseAddress + exp->Name, dll )) return info.BaseAddress;
        }
        addr = (const BYTE *)info.BaseAddress + info.RegionSize;
    }
    return NULL;
}

/* The view (allocation) that holds addr: its base, type and size, through
 * NtQueryVirtualMemory. */
static NTSTATUS query( const void *addr, MEMORY_BASIC_INFORMATION *info )
{
    return NtQueryVirtualMemory( NtCurrentProcess(), addr, MemoryBasicInformation, info, sizeof(*info), NULL );
}

/* ---- the checks ---- */

/* The state block's RIP is LdrInitializeThunk, as the test reads it from
 * ntdll's export table; the host-pointer slots hold the poison value. */
static const char *rip_ldr_initialize_thunk( void )
{
    const BYTE *ntdll = find_image( "ntdll.dll" );
    const BYTE *state;
    ULONG_PTR thunk, syscall, unixcall;

    if (boot_ret) return tw_fail( "boot returned %d", boot_ret );
    state = tw_main_state();
    if (!ntdll) return tw_fail( "no MEM_IMAGE view of ntdll.dll" );
    if (!(thunk = find_export( ntdll, "LdrInitializeThunk" ))) return tw_fail( "ntdll exports no LdrInitializeThunk" );
    if (*(const UINT64 *)(state + TW_STATE_RIP) != thunk)
        return tw_fail( "RIP %#llx, LdrInitializeThunk is at %#lx",
                        (unsigned long long)*(const UINT64 *)(state + TW_STATE_RIP), (unsigned long)thunk );
    syscall = find_export( ntdll, "__wine_syscall_dispatcher" );
    unixcall = find_export( ntdll, "__wine_unix_call_dispatcher" );
    TW_EXPECT( syscall && unixcall );
    TW_EXPECT( *(const UINT64 *)syscall == TW_NO_DISPATCHER );
    TW_EXPECT( *(const UINT64 *)unixcall == TW_NO_DISPATCHER );
    TW_EXPECT( *SYSCALL_SLOT == TW_NO_DISPATCHER );
    return NULL;
}

#define GPR(i) (*(const UINT64 *)(state + TW_STATE_GPR(i)))

/* The CONTEXT LdrInitializeThunk gets and the state block (section 8), as
 * init_syscall_frame builds them. */
static const char *context_and_state( void )
{
    const BYTE *ntdll = find_image( "ntdll.dll" );
    const BYTE *state;
    TEB *teb = NtCurrentTeb();
    PEB *peb = teb->Peb;
    const IMAGE_NT_HEADERS64 *exe = nt_headers( (const BYTE *)peb->ImageBaseAddress );
    ULONG_PTR rsp = (ULONG_PTR)teb->Tib.StackBase - 0x28;
    const CONTEXT *ctx = (const CONTEXT *)((rsp & ~(ULONG_PTR)15) - sizeof(CONTEXT));
    int i;

    if (boot_ret) return tw_fail( "boot returned %d", boot_ret );
    state = tw_main_state();
    TW_EXPECT( ntdll && exe );
    /* the CONTEXT */
    TW_EXPECT( ctx->ContextFlags == CONTEXT_FULL );
    TW_EXPECT( ctx->Rip == find_export( ntdll, "RtlUserThreadStart" ) );
    TW_EXPECT( ctx->Rcx == (ULONG_PTR)peb->ImageBaseAddress + exe->OptionalHeader.AddressOfEntryPoint );
    TW_EXPECT( ctx->Rdx == (ULONG_PTR)peb );
    TW_EXPECT( ctx->Rsp == rsp );
    TW_EXPECT( ctx->SegCs == 0x33 );
    TW_EXPECT( ctx->SegDs == 0x2b && ctx->SegEs == 0x2b && ctx->SegGs == 0x2b && ctx->SegSs == 0x2b );
    TW_EXPECT( ctx->SegFs == 0 );
    TW_EXPECT( ctx->EFlags == 0x200 );
    TW_EXPECT( ctx->FltSave.ControlWord == 0x27f );
    TW_EXPECT( ctx->MxCsr == 0x1f80 && ctx->FltSave.MxCsr == 0x1f80 );
    /* the state block */
    TW_EXPECT( ((ULONG_PTR)state & 15) == 0 );
    TW_EXPECT( GPR(1) == (ULONG_PTR)ctx );        /* RCX */
    TW_EXPECT( GPR(2) == (ULONG_PTR)peb );        /* RDX */
    TW_EXPECT( GPR(4) == (ULONG_PTR)ctx - 8 );    /* RSP */
    for (i = 0; i < 16; i++)
        if (i != 1 && i != 2 && i != 4 && GPR(i)) return tw_fail( "GPR %d is %#llx, not 0", i, (unsigned long long)GPR(i) );
    TW_EXPECT( *(const UINT64 *)(state + TW_STATE_RFLAGS) == 0x200 );
    TW_EXPECT( *(const UINT32 *)(state + TW_STATE_MXCSR) == 0x1f80 );
    TW_EXPECT( *(const UINT16 *)(state + TW_STATE_FCW) == 0x27f );
    TW_EXPECT( *(const UINT16 *)(state + TW_STATE_FTW) == 0xffff );  /* all x87 registers empty */
    TW_EXPECT( *(const UINT16 *)(state + TW_STATE_FSW) == 0 );
    TW_EXPECT( *(const UINT64 *)(state + TW_STATE_GS_BASE) == (ULONG_PTR)teb );
    TW_EXPECT( teb->Tib.Self == &teb->Tib );
    return NULL;
}

/* ntdll and hello.exe are MEM_IMAGE views at their preferred bases. */
static const char *images_at_preferred_bases( void )
{
    const BYTE *ntdll = find_image( "ntdll.dll" );
    const BYTE *exe = (const BYTE *)NtCurrentTeb()->Peb->ImageBaseAddress;
    MEMORY_BASIC_INFORMATION info;

    if (!ntdll) return tw_fail( "no MEM_IMAGE view of ntdll.dll" );
    if ((ULONG_PTR)ntdll != nt_headers( ntdll )->OptionalHeader.ImageBase)
        return tw_fail( "ntdll.dll at %p, its preferred base is %#llx", ntdll,
                        (unsigned long long)nt_headers( ntdll )->OptionalHeader.ImageBase );
    TW_EXPECT( exe && nt_headers( exe ) );
    TW_EXPECT( !query( exe, &info ) );
    if (info.Type != MEM_IMAGE || info.AllocationBase != exe)
        return tw_fail( "hello.exe at %p: type %#x, allocation base %p", exe, (unsigned)info.Type, info.AllocationBase );
    if ((ULONG_PTR)exe != nt_headers( exe )->OptionalHeader.ImageBase)
        return tw_fail( "hello.exe at %p, its preferred base is %#llx", exe,
                        (unsigned long long)nt_headers( exe )->OptionalHeader.ImageBase );
    return NULL;
}

/* PEB->ApiSetMap points into apisetschema.dll's view: the .apiset section of
 * a mapped (not image) view of a PE file, a version 6 schema. */
static const char *apiset_map( void )
{
    const API_SET_NAMESPACE *map = NtCurrentTeb()->Peb->ApiSetMap;
    const IMAGE_NT_HEADERS64 *nt;
    const IMAGE_SECTION_HEADER *sec;
    MEMORY_BASIC_INFORMATION info;
    const BYTE *view;
    unsigned int i;

    if (!map) return tw_fail( "PEB->ApiSetMap is NULL" );
    TW_EXPECT( !query( map, &info ) );
    if (info.Type != MEM_MAPPED) return tw_fail( "ApiSetMap %p is in a view of type %#x", map, (unsigned)info.Type );
    view = info.AllocationBase;
    TW_EXPECT( (nt = nt_headers( view )) );
    sec = IMAGE_FIRST_SECTION( nt );
    for (i = 0; i < nt->FileHeader.NumberOfSections; i++, sec++)
        if (!memcmp( sec->Name, ".apiset", 8 )) break;
    if (i == nt->FileHeader.NumberOfSections) return tw_fail( "the view at %p has no .apiset section", view );
    TW_EXPECT( (const BYTE *)map == view + sec->PointerToRawData );
    TW_EXPECT( map->Version == 6 );
    TW_EXPECT( map->Size <= sec->Misc.VirtualSize );
    return NULL;
}

/* A value through NtOpenKey and NtQueryValueKey, compared: a REG_SZ string
 * (with its terminator, as the server stores it) or a REG_DWORD. */
static const char *expect_value( const WCHAR *key, const WCHAR *name, ULONG type, const WCHAR *sz, DWORD dword )
{
    UNICODE_STRING key_str, name_str;
    OBJECT_ATTRIBUTES attr;
    union { KEY_VALUE_PARTIAL_INFORMATION info; BYTE buf[512]; } v;
    const void *expected = type == REG_SZ ? (const void *)sz : (const void *)&dword;
    ULONG size = type == REG_SZ ? (wcslen( sz ) + 1) * sizeof(WCHAR) : sizeof(dword);
    HANDLE handle;
    NTSTATUS status;
    ULONG len;
    char path[256], got[256], want[256];

    init_unicode_string( &key_str, key );
    init_unicode_string( &name_str, name );
    InitializeObjectAttributes( &attr, &key_str, OBJ_CASE_INSENSITIVE, 0, NULL );
    ntdll_wcstoumbs( key, wcslen( key ) + 1, path, sizeof(path), FALSE );
    if ((status = NtOpenKey( &handle, KEY_QUERY_VALUE, &attr ))) return tw_fail( "NtOpenKey %s: %#x", path, (unsigned)status );
    status = NtQueryValueKey( handle, &name_str, KeyValuePartialInformation, &v, sizeof(v) - sizeof(WCHAR), &len );
    NtClose( handle );
    ntdll_wcstoumbs( name, wcslen( name ) + 1, want, sizeof(want), FALSE );
    if (status) return tw_fail( "%s, %s: NtQueryValueKey %#x", path, want, (unsigned)status );
    if (v.info.Type != type) return tw_fail( "%s, %s: type %u, expected %u", path, want, (unsigned)v.info.Type, (unsigned)type );
    if (v.info.DataLength == size && !memcmp( v.info.Data, expected, size )) return NULL;
    if (type == REG_DWORD) return tw_fail( "%s, %s: %#x, expected %#x", path, want, *(const DWORD *)v.info.Data, dword );
    *(WCHAR *)(v.info.Data + v.info.DataLength) = 0;
    ntdll_wcstoumbs( (const WCHAR *)v.info.Data, wcslen( (const WCHAR *)v.info.Data ) + 1, got, sizeof(got), FALSE );
    ntdll_wcstoumbs( sz, wcslen( sz ) + 1, want, sizeof(want), FALSE );
    return tw_fail( "%s: \"%s\", expected \"%s\"", path, got, want );
}

#define HKLM u"\\Registry\\Machine\\"
#define HKCU u"\\Registry\\User\\S-1-5-21-0-0-0-1000\\"
#define CPU0 HKLM u"Hardware\\Description\\System\\CentralProcessor\\0"
#define SZ(key, name, value) expect_value( key, name, REG_SZ, value, 0 )
#define CHECK(expr) do { if ((detail = (expr))) return detail; } while (0)

/* Values of the prefix (system.reg, user.reg) and of the overlay
 * (native/wine/prefix/overlay.reg) through NtQueryValueKey. The overlay's
 * CPU is the one the Unix side reports (SystemCpuInformation), which comes
 * from the translator's profile (section 3). */
static const char *registry_values( void )
{
    SYSTEM_CPU_INFORMATION sci;
    WCHAR id[64];
    char buf[64];
    const char *detail;

    TW_EXPECT( !NtQuerySystemInformation( SystemCpuInformation, &sci, sizeof(sci), NULL ) );
    snprintf( buf, sizeof(buf), "Intel64 Family %u Model %u Stepping %u",
              sci.ProcessorLevel, HIBYTE(sci.ProcessorRevision), LOBYTE(sci.ProcessorRevision) );
    ntdll_umbstowcs( buf, strlen( buf ) + 1, id, ARRAY_SIZE(id) );

    /* system.reg and user.reg */
    CHECK( SZ( HKLM u"Software\\Microsoft\\Windows NT\\CurrentVersion", u"CurrentBuildNumber", u"19045" ) );
    CHECK( SZ( HKCU u"Control Panel\\International", u"iCountry", u"1" ) );
    /* overlay.reg: the computer name, the CPU, the volatile environment */
    CHECK( SZ( HKLM u"System\\CurrentControlSet\\Control\\ComputerName\\ActiveComputerName", u"ComputerName", u"TEMPWIND" ) );
    CHECK( SZ( CPU0, u"Identifier", id ) );
    CHECK( expect_value( CPU0, u"FeatureSet", REG_DWORD, NULL, (DWORD)sci.ProcessorFeatureBits ) );
    CHECK( SZ( CPU0, u"VendorIdentifier", u"TempwindVCPU" ) );
    CHECK( SZ( CPU0, u"ProcessorNameString", u"Tempwind virtual x86-64 CPU" ) );
    CHECK( SZ( HKLM u"System\\CurrentControlSet\\Control\\Session Manager\\Environment", u"NUMBER_OF_PROCESSORS", u"1" ) );
    CHECK( SZ( HKCU u"Volatile Environment", u"USERDOMAIN", u"TEMPWIND" ) );
    return NULL;
}

/* NLS is loaded: the locale (locale.nls) maps LANG=en_US.UTF-8 to 0x409,
 * and the casing tables (l_intl.nls) case. */
static const char *nls_loaded( void )
{
    LCID user = 0, system = 0;

    TW_EXPECT( !NtQueryDefaultLocale( TRUE, &user ) && !NtQueryDefaultLocale( FALSE, &system ) );
    if (user != 0x409 || system != 0x409) return tw_fail( "locales %#x and %#x, expected 0x409", (unsigned)user, (unsigned)system );
    TW_EXPECT( uctable && lctable );
    TW_EXPECT( RtlUpcaseUnicodeChar( 'a' ) == 'A' && RtlUpcaseUnicodeChar( 0xe9 ) == 0xc9 );
    TW_EXPECT( RtlDowncaseUnicodeChar( 'A' ) == 'a' );
    return NULL;
}

/* The fields of KUSER that differ from native Wine 11's (kuser.bin) by
 * design (section 10): those Wine fills from the clock, those that come from
 * the CPU (the translator's profile here, the host's CPU there), and the
 * host's physical memory. */
#define KUSER_FIELD(f) { offsetof( KUSER_SHARED_DATA, f ), sizeof(((KUSER_SHARED_DATA *)0)->f), #f }
static const struct { unsigned int offset, size; const char *name; } kuser_skip[] =
{
    /* the clock */
    KUSER_FIELD(TickCountLowDeprecated),
    KUSER_FIELD(InterruptTime),
    KUSER_FIELD(SystemTime),
    KUSER_FIELD(TickCount),
    /* the CPU */
    KUSER_FIELD(ProcessorFeatures),
    KUSER_FIELD(ActiveProcessorCount),
    KUSER_FIELD(XState),
    /* the host's memory */
    KUSER_FIELD(NumberOfPhysicalPages),
};

static const char *kuser( void )
{
    static BYTE native[0x1000];
    char diff[400];
    size_t len = 0;
    unsigned int i, j, n = 0;
    FILE *f = fopen( KUSER_ORACLE, "rb" );

    if (!f) return tw_fail( "cannot open %s", KUSER_ORACLE );
    i = fread( native, 1, sizeof(native), f );
    fclose( f );
    if (i != sizeof(native)) return tw_fail( "%s: %u bytes, expected 4096", KUSER_ORACLE, i );
    diff[0] = 0;
    for (i = 0; i < sizeof(native); i++)
    {
        if (KUSER_ADDR[i] == native[i]) continue;
        for (j = 0; j < ARRAY_SIZE(kuser_skip); j++)
            if (i >= kuser_skip[j].offset && i < kuser_skip[j].offset + kuser_skip[j].size) break;
        if (j < ARRAY_SIZE(kuser_skip)) continue;
        if (n++ < 24 && len < sizeof(diff) - 32)
            len += snprintf( diff + len, sizeof(diff) - len, " %#x:%02x/%02x", i, KUSER_ADDR[i], native[i] );
    }
    if (n) return tw_fail( "%u bytes differ (offset:ours/native):%s", n, diff );
    return NULL;
}

static const struct tw_check checks[] =
{
    { "rip_ldr_initialize_thunk", rip_ldr_initialize_thunk },
    { "context_and_state", context_and_state },
    { "images_at_preferred_bases", images_at_preferred_bases },
    { "apiset_map", apiset_map },
    { "registry_values", registry_values },
    { "nls_loaded", nls_loaded },
    { "kuser", kuser },
};

int tw_test_main( void )
{
    static char *argv[] = { "/wine/bin/wine", "C:\\tempwind\\hello.exe", NULL };

    boot_ret = boot( 2, argv, BOOT_STOP_AT_ENTRY );
    return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
}

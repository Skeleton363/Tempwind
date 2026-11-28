/*
 * The static table of unix libraries, in place of dlopen
 * (docs/design/wine-unix.md, section 2, "Unix libraries"; patch 0107).
 *
 * A builtin DLL's __wine_init_unix_call asks for its funcs with
 * NtQueryVirtualMemory(MemoryWineUnixFuncs) (dlls/winecrt0/unix_lib.c
 * L92-96), which reaches get_builtin_unix_funcs (unix/virtual.c L837-863).
 * Patch 0107 has it call tw_unixlib_funcs with the module's unix_path, which
 * find_builtin_dll records without checking that the file exists
 * (unix/loader.c L1358-1363). The libraries are linked into this module, so
 * the table maps a file name to its __wine_unix_call_funcs.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <stdio.h>
#include <string.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winternl.h"
#include "wine/unixlib.h"

#include "tempwind_private.h"

/* win32u's table (win32u/syscall.c L108-111), and its count, which patch
 * 0401 adds there as 0301 adds ntdll's tw_unix_call_count. */
extern const unixlib_entry_t __wine_unix_call_funcs[];
extern const unsigned int tw_win32u_unix_call_count;

struct tw_unixlib
{
    const char *name;               /* the unix_path's file name */
    const unixlib_entry_t *funcs;   /* its __wine_unix_call_funcs */
    const unsigned int *count;      /* its entry count */
};

/* In 7zz's closure only ntdll and win32u have Unix sides, and ntdll's own
 * table doesn't come this way (section 8). */
static const struct tw_unixlib unixlibs[] =
{
    { "win32u.so", __wine_unix_call_funcs, &tw_win32u_unix_call_count },
};

const void *tw_unixlib_funcs( const char *unix_path, int wow )
{
    const char *name = strrchr( unix_path, '/' );
    char what[300];
    size_t i;

    name = name ? name + 1 : unix_path;
    if (!wow)
        for (i = 0; i < ARRAY_SIZE(unixlibs); i++)
            if (!strcmp( name, unixlibs[i].name )) return unixlibs[i].funcs;
    snprintf( what, sizeof(what), "unix library %s%s", unix_path, wow ? " (wow64)" : "" );
    tw_unsupported( what );
}

unsigned int tw_unixlib_count( const void *funcs )
{
    size_t i;

    for (i = 0; i < ARRAY_SIZE(unixlibs); i++)
        if (funcs == unixlibs[i].funcs) return *unixlibs[i].count;
    return 0;
}

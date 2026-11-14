/*
 * The boot export and the startup glue (docs/design/wine-unix.md, section 8).
 *
 * boot() runs Wine's __wine_main as the exec'd wine loader would, and comes
 * back here when signal_start_thread (signal_wasm64.c) has built the main
 * thread's start state: BOOT_STOP_AT_ENTRY, M2's only mode.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <limits.h>
#include <setjmp.h>
#include <stdint.h>
#include <stdlib.h>
#include <unistd.h>

#include "tempwind_private.h"

#define BOOT_STOP_AT_ENTRY 1

/* Wine's entry point (dlls/ntdll/unix/loader.c). */
extern void __wine_main( int argc, char *argv[] );

/* The launch directory: the cwd when boot was entered. env.c's
 * get_initial_directory uses it in place of getcwd(), since the in-process
 * server moves the cwd (03xx). */
const char *tw_launch_dir;

static jmp_buf boot_stop;
static int booting;

/* boot(argc, argv, flags), entered through WebAssembly.promising: argv[0] is
 * /wine/bin/wine, argv[1] the program as a DOS path, then its arguments.
 * Returns 0 once stopped at the entry; a Wine error ends in exit(). */
int boot( int argc, char **argv, int flags )
{
    static char cwd[PATH_MAX];

    if (flags != BOOT_STOP_AT_ENTRY) tw_unsupported( "boot: a mode other than BOOT_STOP_AT_ENTRY (M3)" );
    if (booting) tw_unsupported( "boot: a second boot" );
    if (!getcwd( cwd, sizeof(cwd) )) tw_unsupported( "boot: getcwd" );
    tw_launch_dir = cwd;
    /* What the wine loader's exec of itself sets (loader.c, loader_exec):
     * __wine_main then takes argv as the loader's, with no re-exec. */
    if (setenv( "WINELOADERNOEXEC", "1", 1 )) tw_unsupported( "boot: setenv" );

    booting = 1;
    if (!setjmp( boot_stop ))
    {
        __wine_main( argc, argv );
        tw_unsupported( "boot: __wine_main returned" );
    }
    return 0;
}

/* signal_start_thread's end under BOOT_STOP_AT_ENTRY: back to boot, which
 * returns 0. Wine's startup frames below it are dropped, as the native
 * signal_start_thread drops them when it switches to the guest. */
_Noreturn void tw_boot_stopped( void )
{
    if (!booting) tw_unsupported( "tw_boot_stopped outside boot" );
    longjmp( boot_stop, 1 );
}

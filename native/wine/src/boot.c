/*
 * The boot export and the startup glue (docs/design/wine-unix.md, section 8).
 *
 * boot() runs Wine's __wine_main as the exec'd wine loader would. Once
 * signal_start_thread (signal_wasm64.c) has built the main thread's start
 * state, BOOT_STOP_AT_ENTRY comes back here, and run mode (flags 0) enters
 * tw_run_loop (run.c). In run mode the process's exit comes back here too:
 * exit_process, and NtTerminateProcess before its abort_process call, call
 * tw_process_exit (patch 0302), when it ends on the main thread.
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

/* longjmp's values to boot_stop */
#define BOOT_STOPPED 1
#define BOOT_EXITED  2

static jmp_buf boot_stop;
static int booting;
static int exit_status;

/* boot's flags, for signal_start_thread (signal_wasm64.c). */
int tw_boot_flags;

/* boot(argc, argv, flags), entered through WebAssembly.promising: argv[0] is
 * /wine/bin/wine, argv[1] the program as a DOS path, then its arguments.
 * flags is 0 (run) or BOOT_STOP_AT_ENTRY; any other value stops loudly.
 * Returns 0 once stopped at the entry, and in run mode the process's exit
 * status, the full Windows status; a Wine error ends in exit(). */
int boot( int argc, char **argv, int flags )
{
    static char cwd[PATH_MAX];

    if (flags != 0 && flags != BOOT_STOP_AT_ENTRY) tw_unsupported( "boot: flags other than 0 (run) or BOOT_STOP_AT_ENTRY" );
    if (booting) tw_unsupported( "boot: a second boot" );
    if (!getcwd( cwd, sizeof(cwd) )) tw_unsupported( "boot: getcwd" );
    tw_launch_dir = cwd;
    /* What the wine loader's exec of itself sets (loader.c, loader_exec):
     * __wine_main then takes argv as the loader's, with no re-exec. */
    if (setenv( "WINELOADERNOEXEC", "1", 1 )) tw_unsupported( "boot: setenv" );

    tw_boot_flags = flags;
    booting = 1;
    switch (setjmp( boot_stop ))
    {
    case 0:
        __wine_main( argc, argv );
        tw_unsupported( "boot: __wine_main returned" );
    case BOOT_STOPPED:
        return 0;
    default:
        /* the longjmp left the translated frames' depth behind (abi.md v5,
         * "Trampoline") */
        tw_depth_set( 0 );
        return exit_status;
    }
}

/* signal_start_thread's end under BOOT_STOP_AT_ENTRY: back to boot, which
 * returns 0. Wine's startup frames below it are dropped, as the native
 * signal_start_thread drops them when it switches to the guest. */
_Noreturn void tw_boot_stopped( void )
{
    if (!booting) tw_unsupported( "tw_boot_stopped outside boot" );
    longjmp( boot_stop, BOOT_STOPPED );
}

/* Process exit (patch 0302): exit_process, and NtTerminateProcess before its
 * abort_process call, call it with the full Windows status, before
 * get_unix_exit_code cuts it. Back to boot, which returns the status; the
 * longjmp unwinds the translated frames on the wasm stack as a wasm
 * exception. The instance is finished after it. abort_thread's error ends
 * keep abort_process's _exit, which rejects boot's promise.
 * On a thread other than the main one it records the status and
 * longjmps to that thread's tw_thread_main, which returns
 * TW_THREAD_PROCESS_EXIT (green_pthread.c; section 8, "Process exit from
 * another thread"); boot's promise then never settles. */
_Noreturn void tw_process_exit( int status )
{
    if (!green_is_main()) green_thread_process_exit( status );
    if (!booting) tw_unsupported( "tw_process_exit outside boot" );
    exit_status = status;
    longjmp( boot_stop, BOOT_EXITED );
}

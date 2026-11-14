/*
 * The C harness format (docs/design/wine-unix.md, section 10).
 *
 * A harness native/wine/tests/NAME.c defines its checks and
 *     int tw_test_main(void) { return tw_run_checks( checks, ARRAY_SIZE(checks) ); }
 * which the JS host enters through WebAssembly.promising. Output on stdout,
 * one line per check that ran:
 *     ok NAME
 *     not ok NAME: detail
 * and a last line
 *     done PASSED/TOTAL
 * A check marked TW_ALONE expects a loud stop: it is skipped in a normal run
 * and runs only when TW_TEST_ONLY=<its name> is in the environment, in a fresh
 * instance; then the harness runs only that check. A check returns NULL for
 * success or a detail (tw_fail's formatted string) for failure.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#ifndef TEMPWIND_HARNESS_H
#define TEMPWIND_HARNESS_H

#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

enum tw_check_mode { TW_NORMAL, TW_ALONE };

struct tw_check
{
    const char *name;
    const char *(*fn)( void );
    enum tw_check_mode mode;
};

/* A failure's detail, formatted (the buffer lives until the next call). */
static inline const char *tw_fail( const char *fmt, ... )
{
    static char buf[512];
    va_list args;

    va_start( args, fmt );
    vsnprintf( buf, sizeof(buf), fmt, args );
    va_end( args );
    return buf;
}

/* Fails the check with the expression's text unless it holds. */
#define TW_EXPECT(cond) do { if (!(cond)) return tw_fail( "%s:%d: %s", __FILE__, __LINE__, #cond ); } while (0)

/* Runs the checks (only TW_TEST_ONLY's, if set) and prints the lines;
 * returns 0 when every check that ran passed. stdout is not a tty, so libc
 * buffers it fully: each line is flushed, so a loud stop keeps what came
 * before it. */
static inline int tw_run_checks( const struct tw_check *checks, unsigned int count )
{
    const char *only = getenv( "TW_TEST_ONLY" );
    unsigned int i, total = 0, passed = 0;

    for (i = 0; i < count; i++)
    {
        const char *detail;

        if (only ? strcmp( only, checks[i].name ) : checks[i].mode == TW_ALONE) continue;
        total++;
        detail = checks[i].fn();
        if (detail) printf( "not ok %s: %s\n", checks[i].name, detail );
        else
        {
            printf( "ok %s\n", checks[i].name );
            passed++;
        }
        fflush( stdout );
    }
    if (only && !total) printf( "not ok %s: no such check\n", only );
    printf( "done %u/%u\n", passed, total );
    fflush( stdout );
    return passed == total && total ? 0 : 1;
}

#endif /* TEMPWIND_HARNESS_H */

/*
 * CoreMark port for 64-bit Windows (MinGW-w64), used by the Tempwind Phase 1
 * targets. MIT license (see /LICENSE). Written from CoreMark's porting
 * documentation (barebones_porting.md) and the interface that coremark.h and
 * core_main.c expect; no code from the upstream ports is used.
 *
 * Choices: timer from QueryPerformanceCounter, seeds and iteration count from
 * the command line (SEED_ARG), a static data block (MEM_STATIC), single
 * threaded, floating point on.
 */
#ifndef CORE_PORTME_H
#define CORE_PORTME_H

#include <stddef.h>

#define HAS_FLOAT  1
#define HAS_TIME_H 0
#define USE_CLOCK  0
#define HAS_STDIO  1
/* coremark.h then maps ee_printf to printf. */
#define HAS_PRINTF 1

/* FLAGS_STR is the quoted compiler flag string, set by build.sh. */
#ifndef FLAGS_STR
#define FLAGS_STR "unknown"
#endif
#define COMPILER_VERSION "GCC " __VERSION__
#define COMPILER_FLAGS   FLAGS_STR
#define MEM_LOCATION     "STATIC"

/* ee_ptr_int must hold a pointer: 64 bits on Win64 (long is 32 bits there). */
typedef signed short       ee_s16;
typedef unsigned short     ee_u16;
typedef signed int         ee_s32;
typedef double             ee_f32;
typedef unsigned char      ee_u8;
typedef unsigned int       ee_u32;
typedef unsigned long long ee_ptr_int;
typedef size_t             ee_size_t;

/* Round a pointer up to a 4-byte boundary (the matrix code needs it). */
#define align_mem(x) (void *)(4 + (((ee_ptr_int)(x)-1) & ~(ee_ptr_int)3))

/* Ticks are microseconds from the performance counter: ee_u32 holds 71 minutes. */
#define CORETIMETYPE ee_u32
typedef ee_u32 CORE_TICKS;
#define EE_TICKS_PER_SEC 1000000

#define SEED_METHOD SEED_ARG
#define MEM_METHOD  MEM_STATIC

#define MULTITHREAD 1
#define USE_PTHREAD 0
#define USE_FORK    0
#define USE_SOCKET  0

#define MAIN_HAS_NOARGC   0
#define MAIN_HAS_NORETURN 0

extern ee_u32 default_num_contexts;

typedef struct CORE_PORTABLE_S
{
    ee_u8 portable_id;
} core_portable;

void portable_init(core_portable *p, int *argc, char *argv[]);
void portable_fini(core_portable *p);

/* Which run the data size selects when the build does not say. */
#if !defined(PROFILE_RUN) && !defined(PERFORMANCE_RUN) \
    && !defined(VALIDATION_RUN)
#if (TOTAL_DATA_SIZE == 1200)
#define PROFILE_RUN 1
#elif (TOTAL_DATA_SIZE == 2000)
#define PERFORMANCE_RUN 1
#else
#define VALIDATION_RUN 1
#endif
#endif

#endif /* CORE_PORTME_H */

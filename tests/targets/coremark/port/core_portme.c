/*
 * CoreMark port for 64-bit Windows (MinGW-w64): timer and start-up hooks.
 * MIT license (see /LICENSE); see core_portme.h.
 */
#include "coremark.h"
#include "core_portme.h"

#define WIN32_LEAN_AND_MEAN
#include <windows.h>

static LARGE_INTEGER counter_frequency;
static LARGE_INTEGER start_counter, stop_counter;

ee_u32 default_num_contexts = 1;

/* Called right before the timed part of the benchmark. */
void
start_time(void)
{
    QueryPerformanceCounter(&start_counter);
}

/* Called right after the timed part. */
void
stop_time(void)
{
    QueryPerformanceCounter(&stop_counter);
}

/* Elapsed microseconds between start_time and stop_time. */
CORE_TICKS
get_time(void)
{
    unsigned long long delta
        = (unsigned long long)(stop_counter.QuadPart - start_counter.QuadPart);
    /* Split the multiplication so that long runs cannot overflow 64 bits. */
    unsigned long long freq = (unsigned long long)counter_frequency.QuadPart;
    unsigned long long whole = delta / freq;
    unsigned long long rest  = delta % freq;
    return (CORE_TICKS)(whole * EE_TICKS_PER_SEC
                        + rest * EE_TICKS_PER_SEC / freq);
}

secs_ret
time_in_secs(CORE_TICKS ticks)
{
    return ((secs_ret)ticks) / (secs_ret)EE_TICKS_PER_SEC;
}

void
portable_init(core_portable *p, int *argc, char *argv[])
{
    (void)argc;
    (void)argv;
    if (!QueryPerformanceFrequency(&counter_frequency)
        || counter_frequency.QuadPart <= 0)
    {
        ee_printf("ERROR! QueryPerformanceFrequency failed\n");
        counter_frequency.QuadPart = 1;
    }
    if (sizeof(ee_ptr_int) != sizeof(ee_u8 *))
    {
        ee_printf("ERROR! ee_ptr_int does not hold a pointer\n");
    }
    if (sizeof(ee_u32) != 4)
    {
        ee_printf("ERROR! ee_u32 is not 32 bits\n");
    }
    p->portable_id = 1;
}

void
portable_fini(core_portable *p)
{
    p->portable_id = 0;
}

/*
 * The `link` harness (docs/design/wine-unix.md, section 10): the module's
 * own layer works once linked: the host heap, the heap cap and the one
 * memory grower (section 5), the green pthread layer (section 9) and green
 * switches. tests/wine/link.test.mjs runs it.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <errno.h>
#include <pthread.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "harness.h"
#include "tempwind_private.h"

#define HOST_REGION_START 0x80000000ull
#define HOST_REGION_END   0x100000000ull
#define WASM_PAGE         0x10000ull

static uint64_t memory_bytes( void )
{
    return (uint64_t)__builtin_wasm_memory_size( 0 ) * WASM_PAGE;
}

/* The heap is in the host region (abi.md v5, "The host region"). */
static const char *malloc_host_region( void )
{
    char *p = malloc( 1 << 20 );
    uint64_t at = (uintptr_t)p;

    TW_EXPECT( p );
    TW_EXPECT( at >= HOST_REGION_START && at + (1 << 20) <= HOST_REGION_END );
    memset( p, 0x5a, 1 << 20 );
    TW_EXPECT( p[0] == 0x5a && p[(1 << 20) - 1] == 0x5a );
    free( p );
    return NULL;
}

/* A malloc that would move the break past the host region returns NULL and
 * leaves the memory's size alone (section 5). */
static const char *heap_cap( void )
{
    uint64_t before = memory_bytes();
    void *volatile p;  /* volatile: the compiler may elide an unused malloc */

    TW_EXPECT( before == HOST_REGION_END );  /* INITIAL_MEMORY = 4 GiB */
    errno = 0;
    p = malloc( (size_t)3 << 30 );
    TW_EXPECT( !p );
    TW_EXPECT( errno == ENOMEM );
    TW_EXPECT( memory_bytes() == before );
    return NULL;
}

/* grow_to grows the memory past 4 GiB (the JS test sees memory_grown); the
 * heap still stops at the host region's end. */
static const char *grow_to_then_heap_cap( void )
{
    volatile uint8_t *past = (volatile uint8_t *)(uintptr_t)(HOST_REGION_END + 0x1000);
    uint64_t grown;
    void *volatile big;

    TW_EXPECT( grow_to( HOST_REGION_END + 0x1001 ) == 0 );
    grown = memory_bytes();
    TW_EXPECT( grown == HOST_REGION_END + WASM_PAGE );
    TW_EXPECT( *past == 0 );
    *past = 0xa5;
    TW_EXPECT( *past == 0xa5 );
    TW_EXPECT( grow_to( HOST_REGION_END ) == 0 );   /* already covered: no grow */
    TW_EXPECT( memory_bytes() == grown );
    big = malloc( (size_t)3 << 30 );
    TW_EXPECT( !big );
    TW_EXPECT( memory_bytes() == grown );
    errno = 0;
    TW_EXPECT( grow_to( 0x400000001ull ) == -1 && errno == ENOMEM );  /* past 16 GiB */
    return NULL;
}

static const char *mutex_recursive( void )
{
    pthread_mutexattr_t attr;
    pthread_mutex_t m;

    TW_EXPECT( !pthread_mutexattr_init( &attr ) );
    TW_EXPECT( !pthread_mutexattr_settype( &attr, PTHREAD_MUTEX_RECURSIVE ) );
    TW_EXPECT( !pthread_mutex_init( &m, &attr ) );
    TW_EXPECT( !pthread_mutex_lock( &m ) );
    TW_EXPECT( !pthread_mutex_lock( &m ) );
    TW_EXPECT( !pthread_mutex_trylock( &m ) );
    TW_EXPECT( green_current->mutexes_held == 1 );
    TW_EXPECT( pthread_mutex_destroy( &m ) == EBUSY );
    TW_EXPECT( !pthread_mutex_unlock( &m ) );
    TW_EXPECT( !pthread_mutex_unlock( &m ) );
    TW_EXPECT( green_current->mutexes_held == 1 );
    TW_EXPECT( !pthread_mutex_unlock( &m ) );
    TW_EXPECT( green_current->mutexes_held == 0 );
    TW_EXPECT( pthread_mutex_unlock( &m ) == EPERM );
    TW_EXPECT( !pthread_mutex_destroy( &m ) );
    return NULL;
}

static const char *mutex_normal_errorcheck( void )
{
    pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
    pthread_mutexattr_t attr;
    pthread_mutex_t e;

    TW_EXPECT( !pthread_mutex_lock( &m ) );
    TW_EXPECT( pthread_mutex_trylock( &m ) == EBUSY );
    TW_EXPECT( !pthread_mutex_unlock( &m ) );
    TW_EXPECT( !pthread_mutex_trylock( &m ) );
    TW_EXPECT( !pthread_mutex_unlock( &m ) );

    TW_EXPECT( !pthread_mutexattr_init( &attr ) );
    TW_EXPECT( !pthread_mutexattr_settype( &attr, PTHREAD_MUTEX_ERRORCHECK ) );
    TW_EXPECT( !pthread_mutex_init( &e, &attr ) );
    TW_EXPECT( !pthread_mutex_lock( &e ) );
    TW_EXPECT( pthread_mutex_lock( &e ) == EDEADLK );
    TW_EXPECT( !pthread_mutex_unlock( &e ) );
    TW_EXPECT( pthread_mutex_unlock( &e ) == EPERM );
    TW_EXPECT( green_current->mutexes_held == 0 );
    return NULL;
}

static int once_count;
static void once_init( void ) { once_count++; }

static const char *once_keys_self( void )
{
    static pthread_once_t once = PTHREAD_ONCE_INIT;
    pthread_key_t k1, k2;
    int v = 0;

    TW_EXPECT( !pthread_once( &once, once_init ) );
    TW_EXPECT( !pthread_once( &once, once_init ) );
    TW_EXPECT( once_count == 1 );

    TW_EXPECT( !pthread_key_create( &k1, NULL ) );
    TW_EXPECT( !pthread_key_create( &k2, NULL ) );
    TW_EXPECT( k1 != 0 && k1 != k2 );
    TW_EXPECT( pthread_getspecific( k1 ) == NULL );
    TW_EXPECT( !pthread_setspecific( k1, &v ) );
    TW_EXPECT( pthread_getspecific( k1 ) == &v );
    TW_EXPECT( pthread_getspecific( k2 ) == NULL );
    TW_EXPECT( !pthread_key_delete( k1 ) );
    TW_EXPECT( pthread_setspecific( k1, &v ) == EINVAL );
    TW_EXPECT( !pthread_key_delete( k2 ) );

    TW_EXPECT( pthread_equal( pthread_self(), pthread_self() ) );
    TW_EXPECT( pthread_self() == (pthread_t)green_current );
    return NULL;
}

/* Signalling a condition nobody waits on is a no-op; a wait goes through
 * green.c (WP3). */
static const char *cond_without_waiters( void )
{
    pthread_condattr_t attr;
    pthread_cond_t c;

    TW_EXPECT( !pthread_condattr_init( &attr ) );
    TW_EXPECT( !pthread_condattr_setclock( &attr, CLOCK_MONOTONIC ) );
    TW_EXPECT( !pthread_cond_init( &c, &attr ) );
    TW_EXPECT( !pthread_cond_signal( &c ) );
    TW_EXPECT( !pthread_cond_broadcast( &c ) );
    TW_EXPECT( !pthread_cond_destroy( &c ) );
    return NULL;
}

/* A suspension through green_block keeps the save set; the JS test resumes
 * the thread with its id. */
static const char *green_switch_resumes( void )
{
    volatile int local = 1234;
    uintptr_t sp_before;
    int64_t v;

    errno = 77;
    sp_before = (uintptr_t)&local;
    v = green_switch();
    TW_EXPECT( v == green_current->id );
    TW_EXPECT( errno == 77 );
    TW_EXPECT( local == 1234 && (uintptr_t)&local == sp_before );
    return NULL;
}

/* Alone: pthread_create stops loudly until M4. */
static const char *pthread_create_stops( void )
{
    pthread_t t;

    pthread_create( &t, NULL, NULL, NULL );
    return tw_fail( "pthread_create returned" );
}

/* Alone: suspending while holding a mutex is a bug, and green.c asserts. */
static const char *suspend_holding_mutex( void )
{
    static pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;

    pthread_mutex_lock( &m );
    green_switch();
    return tw_fail( "green_switch returned while a mutex was held" );
}

static const struct tw_check checks[] =
{
    { "malloc_host_region", malloc_host_region },
    { "heap_cap", heap_cap },
    { "grow_to_then_heap_cap", grow_to_then_heap_cap },
    { "mutex_recursive", mutex_recursive },
    { "mutex_normal_errorcheck", mutex_normal_errorcheck },
    { "once_keys_self", once_keys_self },
    { "cond_without_waiters", cond_without_waiters },
    { "green_switch_resumes", green_switch_resumes },
    { "pthread_create_stops", pthread_create_stops, TW_ALONE },
    { "suspend_holding_mutex", suspend_holding_mutex, TW_ALONE },
};

int tw_test_main( void )
{
    return tw_run_checks( checks, sizeof(checks) / sizeof(checks[0]) );
}

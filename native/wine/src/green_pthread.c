/*
 * The green pthread layer, single-thread part (docs/design/wine-unix.md,
 * section 9; abi.md v5, "Green threads").
 *
 * The -ww libc that -sSHARED_MEMORY links without -pthread has no pthread
 * functions at all, so these are the ones Wine calls. M2 has one green
 * thread, the main thread:
 * - mutexes (normal, recursive, error-checking), condition variables,
 *   pthread_once, keys (per green thread), pthread_self and the attribute
 *   functions (libc's own pthread_sigmask serves: it is sigprocmask);
 * - a condition wait goes through green.c;
 * - a thread that suspends while holding a mutex is a bug: green.c asserts on
 *   mutexes_held;
 * - pthread_create stops loudly until M4.
 * Our state lives inside libc's opaque types (checked by the static asserts).
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
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "tempwind_private.h"

static struct green_thread main_thread = { .id = 1 };
struct green_thread *green_current = &main_thread;

/* All-zero is PTHREAD_MUTEX_INITIALIZER: a normal, unlocked mutex. */
struct green_mutex
{
    int type;
    int count;                   /* > 0: locked */
    struct green_thread *owner;
};
struct green_mutexattr { unsigned type; };
struct green_cond
{
    struct green_queue queue;
    int clock;                   /* 0 = CLOCK_REALTIME */
};
struct green_condattr { unsigned clock; };
struct green_attr
{
    size_t stack_size;
    void *stack_addr;
    size_t guard_size;
    int detach_state;
    int initialized;
};

_Static_assert( sizeof(struct green_mutex) <= sizeof(pthread_mutex_t), "pthread_mutex_t" );
_Static_assert( sizeof(struct green_mutexattr) <= sizeof(pthread_mutexattr_t), "pthread_mutexattr_t" );
_Static_assert( sizeof(struct green_cond) <= sizeof(pthread_cond_t), "pthread_cond_t" );
_Static_assert( sizeof(struct green_condattr) <= sizeof(pthread_condattr_t), "pthread_condattr_t" );
_Static_assert( sizeof(struct green_attr) <= sizeof(pthread_attr_t), "pthread_attr_t" );

#define MUTEX(m) ((struct green_mutex *)(m))
#define COND(c) ((struct green_cond *)(c))
#define ATTR(a) ((struct green_attr *)(a))

static _Noreturn void green_bug( const char *what )
{
    fprintf( stderr, "tempwind: green_pthread: %s (thread %d)\n", what, green_current->id );
    abort();
}

/* ---- threads ---- */

pthread_t pthread_self( void )
{
    return (pthread_t)green_current;
}

int (pthread_equal)( pthread_t a, pthread_t b )  /* pthread.h also defines it as a macro */
{
    return a == b;
}

int pthread_create( pthread_t *thread, const pthread_attr_t *attr, void *(*start)( void * ), void *arg )
{
    tw_unsupported( "pthread_create" );
}

/* ---- mutexes ---- */

int pthread_mutexattr_init( pthread_mutexattr_t *attr )
{
    memset( attr, 0, sizeof(*attr) );
    return 0;
}

int pthread_mutexattr_destroy( pthread_mutexattr_t *attr )
{
    return 0;
}

int pthread_mutexattr_settype( pthread_mutexattr_t *attr, int type )
{
    if (type != PTHREAD_MUTEX_NORMAL && type != PTHREAD_MUTEX_RECURSIVE && type != PTHREAD_MUTEX_ERRORCHECK)
        return EINVAL;
    ((struct green_mutexattr *)attr)->type = type;
    return 0;
}

int pthread_mutexattr_gettype( const pthread_mutexattr_t *attr, int *type )
{
    *type = ((const struct green_mutexattr *)attr)->type;
    return 0;
}

int pthread_mutex_init( pthread_mutex_t *mutex, const pthread_mutexattr_t *attr )
{
    memset( mutex, 0, sizeof(*mutex) );
    if (attr) MUTEX(mutex)->type = ((const struct green_mutexattr *)attr)->type;
    return 0;
}

int pthread_mutex_destroy( pthread_mutex_t *mutex )
{
    return MUTEX(mutex)->count ? EBUSY : 0;
}

int pthread_mutex_trylock( pthread_mutex_t *mutex )
{
    struct green_mutex *m = MUTEX(mutex);

    if (!m->count)
    {
        m->count = 1;
        m->owner = green_current;
        green_current->mutexes_held++;
        return 0;
    }
    if (m->owner == green_current && m->type == PTHREAD_MUTEX_RECURSIVE)
    {
        m->count++;
        return 0;
    }
    return EBUSY;
}

int pthread_mutex_lock( pthread_mutex_t *mutex )
{
    struct green_mutex *m = MUTEX(mutex);

    if (!pthread_mutex_trylock( mutex )) return 0;
    if (m->owner == green_current)
    {
        if (m->type == PTHREAD_MUTEX_ERRORCHECK) return EDEADLK;
        green_bug( "deadlock: a normal mutex locked again by its owner" );
    }
    /* Held by another green thread: waiting for it needs the scheduler (M4). */
    tw_unsupported( "pthread_mutex_lock of a mutex another green thread holds" );
}

int pthread_mutex_unlock( pthread_mutex_t *mutex )
{
    struct green_mutex *m = MUTEX(mutex);

    if (!m->count || m->owner != green_current)
    {
        if (m->type != PTHREAD_MUTEX_NORMAL) return EPERM;
        green_bug( "a mutex unlocked by a thread that doesn't hold it" );
    }
    if (--m->count) return 0;
    m->owner = NULL;
    green_current->mutexes_held--;
    return 0;
}

/* ---- condition variables ---- */

int pthread_condattr_init( pthread_condattr_t *attr )
{
    memset( attr, 0, sizeof(*attr) );
    return 0;
}

int pthread_condattr_destroy( pthread_condattr_t *attr )
{
    return 0;
}

int pthread_condattr_setclock( pthread_condattr_t *attr, clockid_t clock )
{
    if (clock != CLOCK_REALTIME && clock != CLOCK_MONOTONIC) return EINVAL;
    ((struct green_condattr *)attr)->clock = clock;
    return 0;
}

int pthread_cond_init( pthread_cond_t *cond, const pthread_condattr_t *attr )
{
    memset( cond, 0, sizeof(*cond) );
    if (attr) COND(cond)->clock = ((const struct green_condattr *)attr)->clock;
    return 0;
}

int pthread_cond_destroy( pthread_cond_t *cond )
{
    return COND(cond)->queue.waiters ? EBUSY : 0;
}

/* Releases the mutex completely (a recursive one too), waits in green.c,
 * then takes it back with its count. */
static int cond_wait( pthread_cond_t *cond, pthread_mutex_t *mutex, const struct timespec *deadline )
{
    struct green_mutex *m = MUTEX(mutex);
    int count = m->count, ret;

    if (!count || m->owner != green_current) return EPERM;
    m->count = 0;
    m->owner = NULL;
    green_current->mutexes_held--;
    ret = green_wait( &COND(cond)->queue, deadline, COND(cond)->clock );
    pthread_mutex_lock( mutex );
    m->count = count;
    return ret;
}

int pthread_cond_wait( pthread_cond_t *cond, pthread_mutex_t *mutex )
{
    return cond_wait( cond, mutex, NULL );
}

int pthread_cond_timedwait( pthread_cond_t *cond, pthread_mutex_t *mutex, const struct timespec *deadline )
{
    return cond_wait( cond, mutex, deadline );
}

int pthread_cond_signal( pthread_cond_t *cond )
{
    if (COND(cond)->queue.waiters) green_wake_queue( &COND(cond)->queue, 0 );
    return 0;
}

int pthread_cond_broadcast( pthread_cond_t *cond )
{
    if (COND(cond)->queue.waiters) green_wake_queue( &COND(cond)->queue, 1 );
    return 0;
}

/* ---- once and keys ---- */

int pthread_once( pthread_once_t *once, void (*init)( void ) )
{
    /* 0: not run, 1: running, 2: done. With one thread, "running" seen again
     * means init called pthread_once on itself. */
    if (*once == 2) return 0;
    if (*once == 1) green_bug( "pthread_once re-entered from its own init function" );
    *once = 1;
    init();
    *once = 2;
    return 0;
}

static struct { int used; void (*dtor)( void * ); } keys[GREEN_KEYS_MAX];

int pthread_key_create( pthread_key_t *key, void (*dtor)( void * ) )
{
    unsigned int i;

    /* Key 0 stays unused, so a zero-initialized pthread_key_t (thread.c's
     * teb_key before virtual_alloc_first_teb) never names a live key. */
    for (i = 1; i < GREEN_KEYS_MAX; i++)
    {
        if (keys[i].used) continue;
        keys[i].used = 1;
        keys[i].dtor = dtor;
        green_current->keys[i] = NULL;
        *key = i;
        return 0;
    }
    return EAGAIN;
}

int pthread_key_delete( pthread_key_t key )
{
    if (key >= GREEN_KEYS_MAX || !keys[key].used) return EINVAL;
    keys[key].used = 0;
    green_current->keys[key] = NULL;
    return 0;
}

void *pthread_getspecific( pthread_key_t key )
{
    return key < GREEN_KEYS_MAX ? green_current->keys[key] : NULL;
}

int pthread_setspecific( pthread_key_t key, const void *value )
{
    if (key >= GREEN_KEYS_MAX || !keys[key].used) return EINVAL;
    green_current->keys[key] = (void *)value;
    return 0;
}

/* ---- thread attributes ---- */

int pthread_attr_init( pthread_attr_t *attr )
{
    memset( attr, 0, sizeof(*attr) );
    ATTR(attr)->initialized = 1;
    return 0;
}

int pthread_attr_destroy( pthread_attr_t *attr )
{
    return 0;
}

int pthread_attr_setstacksize( pthread_attr_t *attr, size_t size )
{
    ATTR(attr)->stack_size = size;
    return 0;
}

int pthread_attr_getstacksize( const pthread_attr_t *attr, size_t *size )
{
    *size = ATTR(attr)->stack_size;
    return 0;
}

int pthread_attr_setstack( pthread_attr_t *attr, void *addr, size_t size )
{
    ATTR(attr)->stack_addr = addr;
    ATTR(attr)->stack_size = size;
    return 0;
}

int pthread_attr_getstack( const pthread_attr_t *attr, void **addr, size_t *size )
{
    *addr = ATTR(attr)->stack_addr;
    *size = ATTR(attr)->stack_size;
    return 0;
}

int pthread_attr_setguardsize( pthread_attr_t *attr, size_t size )
{
    ATTR(attr)->guard_size = size;
    return 0;
}

int pthread_attr_setdetachstate( pthread_attr_t *attr, int state )
{
    if (state != PTHREAD_CREATE_JOINABLE && state != PTHREAD_CREATE_DETACHED) return EINVAL;
    ATTR(attr)->detach_state = state;
    return 0;
}

int pthread_attr_getdetachstate( const pthread_attr_t *attr, int *state )
{
    *state = ATTR(attr)->detach_state;
    return 0;
}

int pthread_attr_setscope( pthread_attr_t *attr, int scope )
{
    return scope == PTHREAD_SCOPE_SYSTEM ? 0 : ENOTSUP;
}

/* ---- the rest of a thread's life: M4 ---- */

_Noreturn void pthread_exit( void *ret )
{
    tw_unsupported( "pthread_exit" );
}

int pthread_join( pthread_t thread, void **ret )
{
    tw_unsupported( "pthread_join" );
}

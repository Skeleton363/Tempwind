/*
 * The green pthread layer (docs/design/wine-unix.md, section 9; abi.md v5,
 * "Green threads").
 *
 * The -ww libc that -sSHARED_MEMORY links without -pthread has no pthread
 * functions at all, so these are the ones Wine calls:
 * - mutexes (normal, recursive, error-checking), condition variables,
 *   pthread_once, keys (per green thread), pthread_self and the attribute
 *   functions;
 * - a condition wait goes through green.c;
 * - a thread that suspends while holding a mutex is a bug: green.c asserts on
 *   mutexes_held;
 * - from M4, threads: pthread_create makes a green thread on the stack
 *   pthread_attr_setstack gave (Wine's kernel stack) and calls the import
 *   green_spawn; JS starts it with the raw export tw_thread_main.
 *   pthread_exit longjmps back there (on the main thread it parks);
 *   pthread_join waits for a thread's end and frees it. Process exit on a
 *   thread other than the main one longjmps there too.
 * - from M4 (WP4), the signal mask per green thread: tw_pthread_sigmask,
 *   which config.h makes Wine's pthread_sigmask; a new thread inherits its
 *   creator's mask.
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
#include <setjmp.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#include "tempwind_private.h"

/* A green thread's life (M4); green_thread is what green.c sees. */
enum gthread_state { GT_CREATED, GT_RUNNING, GT_ENDED };
struct gthread
{
    struct green_thread g;          /* first: a green_thread is its gthread */
    struct gthread *next;           /* in the list of green threads not yet freed */
    enum gthread_state state;
    void *(*start)( void * );
    void *arg;
    void *retval;                   /* pthread_exit's */
    jmp_buf *exit_jmp;              /* tw_thread_main's */
    void *own_stack;                /* malloc'd when attr gave none */
    struct green_queue joiners;
};

/* tw_thread_main's setjmp values */
#define JMP_ENDED        1
#define JMP_PROCESS_EXIT 2

static struct gthread main_thread = { .g.id = 1, .state = GT_RUNNING };
static struct gthread *threads = &main_thread;
static int32_t next_id = 2;
struct green_thread *green_current = &main_thread.g;

/* Process exit on a thread other than the main one (boot.c's
 * tw_process_exit): the status, for tw_exit_status. */
static int thread_exit_status, thread_exited;

#define GTHREAD(t) ((struct gthread *)(t))

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

struct green_thread *green_thread_by_id( int32_t id )
{
    struct gthread *t;

    for (t = threads; t; t = t->next)
        if (t->g.id == id) return t->state == GT_ENDED ? NULL : &t->g;
    return NULL;
}

int green_is_main( void )
{
    return green_current == &main_thread.g;
}

/* get_unix_tid's (unix/server.c L1527-1549, with HAVE_PTHREAD_GETTHREADID_NP
 * from config.h): the green thread's id is the thread's unix_tid, by which
 * the in-process server finds it (wine-unix.md, section 7). */
int pthread_getthreadid_np( void )
{
    return green_current->id;
}

/* Wine's only call is NtCreateThreadEx's (unix/thread.c L1445-1450), with
 * the kernel stack from pthread_attr_setstack and start_thread. The thread
 * counts as runnable until tw_thread_main starts it. */
int pthread_create( pthread_t *thread, const pthread_attr_t *attr, void *(*start)( void * ), void *arg )
{
    struct gthread *t;
    char *bottom;
    size_t size;

    if (attr && ATTR(attr)->detach_state == PTHREAD_CREATE_DETACHED)
        tw_unsupported( "pthread_create of a detached thread" );
    if (!(t = calloc( 1, sizeof(*t) ))) return EAGAIN;
    if (attr && ATTR(attr)->stack_addr)
    {
        bottom = ATTR(attr)->stack_addr;
        size = ATTR(attr)->stack_size;
    }
    else
    {
        size = 1 << 20;
        if (!(bottom = t->own_stack = aligned_alloc( 16, size )))
        {
            free( t );
            return EAGAIN;
        }
    }
    t->g.id = next_id++;
    /* a pthread inherits its creator's mask: NtCreateThreadEx blocks
     * server_block_set around this call (unix/thread.c L1419-1459), so the
     * thread starts with SIGUSR1 blocked until signal_start_thread unblocks
     * it (wine-unix.md, section 9, "The mask") */
    t->g.sig_blocked = green_current->sig_blocked;
    t->state = GT_CREATED;
    t->start = start;
    t->arg = arg;
    t->joiners.what = "a thread's end";
    t->next = threads;
    threads = t;
    green_runnable_add( 1 );
    green_spawn( t->g.id, (uint64_t)(uintptr_t)(bottom + size), (uint64_t)(uintptr_t)bottom );
    *thread = (pthread_t)t;
    return 0;
}

/* A spawned thread's entry (raw export), entered through
 * WebAssembly.promising on the thread's own stack, which JS set first. */
int32_t tw_thread_main( int32_t id )
{
    struct gthread *t = (struct gthread *)green_thread_by_id( id );
    jmp_buf exit_jmp;

    if (!t || t->state != GT_CREATED) tw_unsupported( "tw_thread_main: no spawned thread with that id to start" );
    green_current = &t->g;
    tw_depth_set( 0 );
    green_runnable_add( -1 );
    errno = 0;
    t->state = GT_RUNNING;
    t->exit_jmp = &exit_jmp;
    switch (setjmp( exit_jmp ))
    {
    case 0:
        /* Wine's only routine is start_thread, a void (TEB *) cast to the
         * pthread type (unix/thread.c L1119, L1450). A call through a
         * pointer of another signature traps in wasm (as section 11's
         * syscall wrappers note), so it is called as what it is; it never
         * returns (signal_start_thread doesn't), and a return would be a
         * pthread_exit( NULL ). */
        ((void (*)( void * ))t->start)( t->arg );
        pthread_exit( NULL );
    case JMP_ENDED:
        tw_depth_set( 0 );  /* the frames the longjmp dropped leave nothing behind */
        return 0;
    default:
        tw_depth_set( 0 );
        return TW_THREAD_PROCESS_EXIT;
    }
}

_Noreturn void green_thread_process_exit( int status )
{
    struct gthread *t = GTHREAD(green_current);

    if (t == &main_thread) green_bug( "green_thread_process_exit on the main thread" );
    thread_exit_status = status;
    thread_exited = 1;
    longjmp( *t->exit_jmp, JMP_PROCESS_EXIT );
}

int32_t tw_exit_status( void )
{
    if (!thread_exited) tw_unsupported( "tw_exit_status: no process exit on a thread other than the main one" );
    return thread_exit_status;
}

/* Wine's ntdll, server and win32u never call these (section 9). */
int pthread_detach( pthread_t thread )
{
    tw_unsupported( "pthread_detach" );
}

int pthread_kill( pthread_t thread, int sig )
{
    tw_unsupported( "pthread_kill" );
}

int pthread_cancel( pthread_t thread )
{
    tw_unsupported( "pthread_cancel" );
}

/* ---- the signal mask (M4 WP4; wine-unix.md, section 9, "The mask") ---- */

#define SIG_BIT(sig) ((uint64_t)1 << ((sig) - 1))

/* pthread_sigmask on the running green thread's mask (bit sig - 1 of
 * sig_blocked, signals 1-64), in place of libc's process-wide one. SIGKILL
 * and SIGSTOP are never blocked, as on Linux. Unblocking a pending signal is
 * delivery point 2: it is handled before this returns, as the kernel
 * delivers it before sigprocmask returns. */
int tw_pthread_sigmask( int how, const sigset_t *set, sigset_t *old )
{
    struct green_thread *self = green_current;
    uint64_t prev = self->sig_blocked, mask = 0;
    int sig;

    if (set && how != SIG_BLOCK && how != SIG_UNBLOCK && how != SIG_SETMASK) return EINVAL;
    if (old)
    {
        sigemptyset( old );
        for (sig = 1; sig <= 64; sig++)
            if (prev & SIG_BIT( sig )) sigaddset( old, sig );
    }
    if (!set) return 0;
    for (sig = 1; sig <= 64; sig++)
        if (sigismember( set, sig ) == 1) mask |= SIG_BIT( sig );
    switch (how)
    {
    case SIG_BLOCK:   self->sig_blocked = prev | mask; break;
    case SIG_UNBLOCK: self->sig_blocked = prev & ~mask; break;
    default:          self->sig_blocked = mask; break;
    }
    self->sig_blocked &= ~(SIG_BIT( SIGKILL ) | SIG_BIT( SIGSTOP ));
    if (prev & ~self->sig_blocked & self->sig_pending) green_deliver_signals();
    return 0;
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

/* ---- a thread's end (M4) ---- */

/* Wine's only caller is pthread_exit_wrapper (unix/thread.c L1103-1111),
 * after it closed the thread's fds. Runs the key destructors, makes the
 * server's hang-up (section 7, "A thread's end"), then wakes the joiners and
 * longjmps to tw_thread_main; the main thread parks instead: a green_block
 * that nothing wakes. */
_Noreturn void pthread_exit( void *ret )
{
    struct gthread *t = GTHREAD(green_current);
    unsigned int i;

    if (t->g.mutexes_held) green_bug( "pthread_exit while holding a mutex" );
    for (i = 1; i < GREEN_KEYS_MAX; i++)
    {
        void *value = t->g.keys[i];

        if (!keys[i].used || !keys[i].dtor || !value) continue;
        t->g.keys[i] = NULL;
        keys[i].dtor( value );
    }
    inproc_thread_hangup( t->g.id );
    t->retval = ret;
    t->state = GT_ENDED;
    if (t == &main_thread)
    {
        green_parked = &t->g;
        for (;;) green_switch();
    }
    green_wake_queue( &t->joiners, 1 );
    longjmp( *t->exit_jmp, JMP_ENDED );
}

/* Wine joins only in exit_thread (unix/thread.c L1503-1510), a thread whose
 * pthread_id is set, so never the main thread (L1128, L1507). */
int pthread_join( pthread_t thread, void **ret )
{
    struct gthread *t = GTHREAD(thread), **p;

    if (t == &main_thread) tw_unsupported( "pthread_join of the main thread" );
    if (&t->g == green_current) return EDEADLK;
    while (t->state != GT_ENDED) green_wait( &t->joiners, NULL, CLOCK_MONOTONIC );
    if (ret) *ret = t->retval;
    for (p = &threads; *p; p = &(*p)->next)
    {
        if (*p != t) continue;
        *p = t->next;
        break;
    }
    free( t->own_stack );
    free( t );
    return 0;
}

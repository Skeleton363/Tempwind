/*
 * Declarations shared by Tempwind's own files in native/wine/src/
 * (docs/design/wine-unix.md, section 1).
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#ifndef TEMPWIND_PRIVATE_H
#define TEMPWIND_PRIVATE_H

#include <stdint.h>
#include <time.h>
#include <sys/types.h>

/* stubs.c: stop loudly. Prints "tempwind: unsupported: <what>" on stderr and
 * calls abort() (section 2). */
_Noreturn void tw_unsupported( const char *what );

/* Imports of the module (abi.md v5, "Host entries of the Unix side"),
 * declared by library.js. */
int64_t green_block( int32_t thread );  /* WebAssembly.Suspending */
void green_wake( int32_t thread );
void memory_grown( void );
int32_t tw_run( uint64_t state );

/* green_pthread.c: a green thread as the pthread layer sees it (section 9).
 * M2 has one, the main thread. */
#define GREEN_KEYS_MAX 128
struct green_thread
{
    int32_t id;               /* the id green_block and green_wake carry */
    int mutexes_held;         /* suspending while > 0 is a bug */
    void *keys[GREEN_KEYS_MAX];
};
extern struct green_thread *green_current;

/* green.c (section 9). */
int64_t green_switch( void );  /* the only switch point, around green_block */
struct green_waiter;
struct green_queue             /* all zero is an empty queue */
{
    int waiters;
    struct green_waiter *head, *tail;
    const char *what;          /* for the deadlock report; NULL: a condition variable */
};
int green_wait( struct green_queue *queue, const struct timespec *abs_deadline, int clock );
void green_wake_queue( struct green_queue *queue, int all );

/* virtual_wasm.c (section 5): grows the memory to cover end; 0 or -1 with
 * errno = ENOMEM. The module's only memory.grow. */
int grow_to( uint64_t end );

/* virtual_wasm.c (sections 4 and 6): the backend under virtual.c, which the
 * 01xx patches declare there too. The guest regions are tw_reserve_list,
 * ended by a zero size (virtual.c's struct preload_info). */
struct tw_range
{
    void  *addr;
    size_t size;
};
extern const struct tw_range tw_reserve_list[];
void *tw_mmap( void *addr, size_t len, int prot, int flags, int fd, off_t offset );
int tw_munmap( void *addr, size_t len );
int tw_mprotect( void *addr, size_t len, int prot );
int tw_madvise( void *addr, size_t len, int advice );
int tw_msync( void *addr, size_t len, int flags );
void *tw_page_flags( void );
void tw_page_watch( const void *addr, size_t size, int on );

#endif /* TEMPWIND_PRIVATE_H */

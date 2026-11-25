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
#include <signal.h>
#include <sys/types.h>

/* ---- Stopping loudly ---- */

/* stubs.c: stop loudly. Prints "tempwind: unsupported: <what>" on stderr and
 * calls abort() (section 2). */
_Noreturn void tw_unsupported( const char *what );

/* ---- The bridge and the JS host's imports ---- */

/* Imports of the module (abi.md v5, "Host entries of the Unix side"),
 * declared by library.js. */
int64_t green_block( int32_t thread );  /* WebAssembly.Suspending */
void green_wake( int32_t thread );
void memory_grown( void );
/* The bridge's exports (wasm to wasm), then the JS host's. */
int32_t tw_run( uint64_t state );
int32_t tw_depth_get( void );
void tw_depth_set( int32_t depth );
void tw_flags_materialize( uint64_t state );
int32_t tw_translate( uint64_t rip, uint64_t base, uint64_t size );
void tw_image_mapped( int32_t fd, uint64_t base, uint64_t size );
void tw_invalidate( uint64_t base, uint64_t size );

/* Import (library.js): C made the green thread `thread`, whose shadow stack
 * is [bottom, top); JS starts it after the current call into the module
 * returns (section 9). */
void green_spawn( int32_t thread, uint64_t top, uint64_t bottom );

/* ---- The virtual backend ---- */

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

/* virtual.c (patch 0106): the protection byte of addr's page, and in
 * *base, *size the run of committed executable pages around it inside its
 * view (size 0: none), which the run loop passes to tw_translate. */
unsigned int tw_exec_range( uint64_t addr, uint64_t *base, uint64_t *size );

/* ---- The in-process server ---- */

/* patch 0201 (server/ptrace.c): the server's side of a thread's end, the
 * hang-up thread_poll_event sees natively (section 7, "A thread's end"). */
void inproc_thread_hangup( int tid );

/* ---- Green threads ---- */

/* green_pthread.c: a green thread as the pthread layer sees it (section 9).
 * The main thread is one; the spawned ones add their signal state (green.c
 * delivers it). */
#define GREEN_KEYS_MAX 128
struct green_waiter;
struct green_thread
{
    int32_t id;               /* the id green_block and green_wake carry */
    int mutexes_held;         /* suspending while > 0 is a bug */
    void *keys[GREEN_KEYS_MAX];
    /* (section 9, "Signals"): bit sig - 1 of each */
    uint64_t sig_pending;
    uint64_t sig_blocked;     /* the thread's mask; SIGQUIT is never blocked */
    struct green_waiter *waiter;  /* the wait it is in, which a signal ends */
};
extern struct green_thread *green_current;

/* green.c (section 9). */
int64_t green_switch( void );  /* the only switch point, around green_block */
struct green_queue             /* all zero is an empty queue */
{
    int waiters;
    struct green_waiter *head, *tail;
    const char *what;          /* for the deadlock report; NULL: a condition variable */
};
int green_wait( struct green_queue *queue, const struct timespec *abs_deadline, int clock );
void green_wake_queue( struct green_queue *queue, int all );

/* green.c: green_runnable_add counts spawned threads that have yet to start
 * as runnable, for sched_yield. green_parked is the parked main thread, for
 * the deadlock report. */
void green_runnable_add( int count );
extern struct green_thread *green_parked;

/* green_pthread.c (wine-unix.md v3, sections 7 and 9): the live green thread
 * with that id, or NULL; whether the running thread is the main one; process
 * exit on a thread other than the main one (tw_process_exit): records the
 * status and longjmps to the thread's tw_thread_main, which returns
 * TW_THREAD_PROCESS_EXIT. */
struct green_thread *green_thread_by_id( int32_t id );
int green_is_main( void );
_Noreturn void green_thread_process_exit( int status );

/* green_pthread.c (section 9): raw exports. tw_thread_main runs a spawned
 * thread on its own stack, entered through WebAssembly.promising; it returns
 * 0 when the thread ended, TW_THREAD_PROCESS_EXIT when the process exited on
 * it (section 8). tw_exit_status gives that exit's full Windows status. */
#define TW_THREAD_PROCESS_EXIT 1
int32_t tw_thread_main( int32_t thread );
int32_t tw_exit_status( void );

/* ---- Signals and the mask ---- */

/* green.c: signals. green_signal marks sig pending on the green thread with
 * that id and wakes it if it waits; 1, or 0 when no such green thread is
 * alive (send_thread_signal, section 7). green_deliver_signals handles the
 * running thread's pending signals that its mask doesn't block, at the
 * delivery points; it returns 1 when a handler ran and returned. */
int inproc_thread_signal( int tid, int sig );
int green_deliver_signals( void );

/* signal_wasm64.c: the handler of a delivered signal (SIGQUIT: abort_thread,
 * as quit_handler); it returns only for a handler that returns. */
void tw_handle_signal( int sig );

/* signal_wasm64.c: NULL in the real module. A test harness sets it to run a
 * C routine in place of the guest entry of a thread signal_start_thread
 * starts (entry and arg as NtCreateThreadEx got them, and the thread's state
 * block); the routine ends the thread itself, through Wine. */
extern void (*tw_thread_start_hook)( void *entry, void *arg, void *state );

/* green_pthread.c: the running green thread's mask, pthread_sigmask's
 * contract; config.h renames Wine's calls to it. An unblock of a pending
 * signal is delivery point 2. */
int tw_pthread_sigmask( int how, const sigset_t *set, sigset_t *old );

/* ---- The boot and process exit ---- */

/* boot.c (section 8): the end of signal_start_thread under
 * BOOT_STOP_AT_ENTRY, back to boot. */
_Noreturn void tw_boot_stopped( void );

/* boot.c (section 8): boot's flags, 0 (run) or BOOT_STOP_AT_ENTRY (1). */
extern int tw_boot_flags;

/* boot.c (section 8): process exit, from exit_process and from
 * NtTerminateProcess's abort_process call (patch 0302); back to boot with
 * the full Windows status. */
_Noreturn void tw_process_exit( int status );

/* ---- The run loop ---- */

/* run.c (section 12): runs translated code from the state block until the
 * thread ends. It returns only on EXIT_HOST; the host function that stopped
 * recorded why. Every other end stops loudly, or is process exit's longjmp. */
void tw_run_loop( uint64_t state );

/* run.c (section 12): set by a host function that stops with
 * EXIT_HOST, before it returns STATUS_EXIT; the run loop needs a reason. */
extern const char *tw_host_stop_reason;

/* ---- Syscalls and unix calls ---- */

/* unixlibs.c (section 2, patch 0107): the static unix library table, in
 * place of dlopen. tw_unixlib_funcs returns the __wine_unix_call_funcs of
 * the library named by unix_path's file name; a wow request or a library not
 * in the table stops loudly. tw_unixlib_count returns the entry count of a
 * table tw_unixlib_funcs returned, and 0 for any other handle (the unix-call
 * dispatcher, section 11). */
const void *tw_unixlib_funcs( const char *unix_path, int wow );
unsigned int tw_unixlib_count( const void *funcs );

/* ---- The session view ---- */

/* server/mapping.c (section 7, patch 0203): the session's shared memory, one
 * reservation of TW_SESSION_CAP bytes in the host heap. tw_session_view
 * returns the address of offset in it and sets *size to the bytes from there
 * to the reservation's end; past the cap, or before the server has set the
 * session up, it stops loudly. win32u's map_shared_session_block uses it
 * (04xx). */
#define TW_SESSION_CAP ((size_t)16 << 20)
void *tw_session_view( size_t offset, size_t *size );

#endif /* TEMPWIND_PRIVATE_H */

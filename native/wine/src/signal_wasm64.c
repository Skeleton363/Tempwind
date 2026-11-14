/*
 * The machine layer of the wasm64 Unix side, in place of signal_x86_64.c
 * (docs/design/wine-unix.md, sections 1 and 8; abi.md v5, "Wine boundary").
 *
 * M2: the process and thread setup of section 8 and the start state of
 * BOOT_STOP_AT_ENTRY. The rest stops loudly until M3-M5 own it (the run
 * loop, syscalls, contexts, exceptions). NtCurrentTeb() needs no entry here:
 * thread.c's reads teb_key on every machine.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include "config.h"

#include <pthread.h>
#include <stdarg.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winternl.h"
#include "unix_private.h"

#include "tempwind/abi.h"
#include "tempwind_private.h"

/* The x86-64 user selectors (section 3). The thread's FS selector is 0: no
 * WoW64. */
static const WORD cs64_sel = 0x33;
static const WORD ds64_sel = 0x2b;

/* The main thread's CPU state block (abi.md, "CPU state block"), in the host
 * heap; signal_start_thread fills it. */
static BYTE *main_state;

/* ---- process and thread setup (section 8) ---- */

/* signal_x86_64.c reads the host's selectors here; ours are constants. */
void signal_init_threading( void )
{
}

/* No signal handlers: nothing faults, and the server's signals are handled
 * in-process (section 7). What stays of signal_x86_64.c's is the page at
 * 0x7ffe1000, which holds the syscall dispatcher's address: it gets the
 * poison value until M3 writes the dispatcher's (section 8). */
void signal_init_process( void )
{
    void *ptr = (char *)user_shared_data + page_size;

    if (anon_mmap_fixed( ptr, page_size, PROT_READ | PROT_WRITE, 0 ) != ptr)
        tw_unsupported( "signal_init_process: committing the page at 0x7ffe1000" );
    *(UINT64 *)ptr = TW_NO_DISPATCHER;
}

/* No LDT entry to allocate: no WoW64 (section 3). */
NTSTATUS signal_alloc_thread( TEB *teb )
{
    if (teb->WowTebOffset) tw_unsupported( "signal_alloc_thread: a WoW64 thread" );
    return STATUS_SUCCESS;
}

void signal_free_thread( TEB *teb )
{
}

/* The main thread's state block, once boot has stopped at the entry. */
void *tw_main_state( void )
{
    if (!main_state) tw_unsupported( "tw_main_state: boot has not stopped at the entry" );
    return main_state;
}

/* The thread's start as signal_x86_64.c's init_syscall_frame builds it
 * (section 8): the CONTEXT that LdrInitializeThunk gets, below the stack's
 * top, and the state block that enters LdrInitializeThunk with RCX = that
 * CONTEXT. BOOT_STOP_AT_ENTRY is M2's only mode, so it then returns to boot. */
void signal_start_thread( PRTL_THREAD_START_ROUTINE entry, void *arg, BOOL suspend, TEB *teb )
{
    CONTEXT *ctx, context = { 0 };
    BYTE *state;

    if (suspend) tw_unsupported( "signal_start_thread: a suspended start (M4)" );
    if (teb != NtCurrentTeb()) tw_unsupported( "signal_start_thread: a thread other than the running one (M4)" );

    context.ContextFlags = CONTEXT_ALL | CONTEXT_EXCEPTION_REPORTING | CONTEXT_EXCEPTION_ACTIVE;
    context.Rcx    = (ULONG_PTR)entry;
    context.Rdx    = (ULONG_PTR)arg;
    context.Rsp    = (ULONG_PTR)teb->Tib.StackBase - 0x28;
    context.Rip    = (ULONG_PTR)pRtlUserThreadStart;
    context.SegCs  = cs64_sel;
    context.SegDs  = ds64_sel;
    context.SegEs  = ds64_sel;
    context.SegFs  = 0;
    context.SegGs  = ds64_sel;
    context.SegSs  = ds64_sel;
    context.EFlags = 0x200;
    context.FltSave.ControlWord = 0x27f;
    context.FltSave.MxCsr = context.MxCsr = 0x1f80;

    ctx = (CONTEXT *)((ULONG_PTR)context.Rsp & ~15) - 1;
    *ctx = context;
    ctx->ContextFlags = CONTEXT_FULL;

    if (!(state = aligned_alloc( 64, TW_STATE_SIZE )))
        tw_unsupported( "signal_start_thread: no host memory for the state block" );
    memset( state, 0, TW_STATE_SIZE );
    *(UINT64 *)(state + TW_STATE_GPR(1)) = (ULONG_PTR)ctx;      /* RCX */
    *(UINT64 *)(state + TW_STATE_GPR(2)) = (ULONG_PTR)arg;      /* RDX: the PEB */
    *(UINT64 *)(state + TW_STATE_GPR(4)) = (ULONG_PTR)ctx - 8;  /* RSP */
    *(UINT64 *)(state + TW_STATE_RIP) = (ULONG_PTR)pLdrInitializeThunk;
    *(UINT64 *)(state + TW_STATE_RFLAGS) = 0x200;
    *(UINT64 *)(state + TW_STATE_GS_BASE) = (ULONG_PTR)teb;
    *(UINT32 *)(state + TW_STATE_MXCSR) = 0x1f80;
    *(UINT16 *)(state + TW_STATE_FCW) = 0x27f;
    main_state = state;

    tw_boot_stopped();
}

/* ---- guest entries into the Unix side (M3) ---- */

void __wine_syscall_dispatcher( void )
{
    tw_unsupported( "__wine_syscall_dispatcher" );
}

void __wine_unix_call_dispatcher( void )
{
    tw_unsupported( "__wine_unix_call_dispatcher" );
}

/* ---- contexts (M3) ---- */

NTSTATUS WINAPI NtGetContextThread( HANDLE handle, CONTEXT *context )
{
    tw_unsupported( "NtGetContextThread" );
}

NTSTATUS WINAPI NtSetContextThread( HANDLE handle, const CONTEXT *context )
{
    tw_unsupported( "NtSetContextThread" );
}

NTSTATUS signal_set_full_context( CONTEXT *context )
{
    tw_unsupported( "signal_set_full_context" );
}

void *get_native_context( CONTEXT *context )
{
    tw_unsupported( "get_native_context" );
}

void *get_wow_context( CONTEXT *context )
{
    tw_unsupported( "get_wow_context" );
}

NTSTATUS get_thread_wow64_context( HANDLE handle, void *ctx, ULONG size )
{
    tw_unsupported( "get_thread_wow64_context" );
}

NTSTATUS set_thread_wow64_context( HANDLE handle, const void *ctx, ULONG size )
{
    tw_unsupported( "set_thread_wow64_context" );
}

NTSTATUS get_thread_ldt_entry( HANDLE handle, THREAD_DESCRIPTOR_INFORMATION *info, ULONG len )
{
    tw_unsupported( "get_thread_ldt_entry" );
}

void set_process_instrumentation_callback( void *callback )
{
    tw_unsupported( "set_process_instrumentation_callback" );
}

/* ---- calls into the guest's dispatchers (M3-M5) ---- */

NTSTATUS call_user_apc_dispatcher( CONTEXT *context_ptr, unsigned int flags, ULONG_PTR arg1, ULONG_PTR arg2,
                                   ULONG_PTR arg3, PNTAPCFUNC func, NTSTATUS status )
{
    tw_unsupported( "call_user_apc_dispatcher" );
}

NTSTATUS call_user_exception_dispatcher( EXCEPTION_RECORD *rec, CONTEXT *context )
{
    tw_unsupported( "call_user_exception_dispatcher" );
}

void call_raise_user_exception_dispatcher( void )
{
    tw_unsupported( "call_raise_user_exception_dispatcher" );
}

NTSTATUS WINAPI NtCallbackReturn( void *ret_ptr, ULONG ret_len, NTSTATUS status )
{
    tw_unsupported( "NtCallbackReturn" );
}

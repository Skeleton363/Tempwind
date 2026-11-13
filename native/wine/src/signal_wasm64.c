/*
 * The machine layer of the wasm64 Unix side, in place of signal_x86_64.c
 * (docs/design/wine-unix.md, sections 1 and 8; abi.md v5, "Wine boundary").
 *
 * M2 WP1 skeleton: the functions ntdll's other files expect from a
 * signal_*.c, each stopping loudly until WP4 (startup) and M3-M5 (the run
 * loop, syscalls, contexts, exceptions) own them. NtCurrentTeb() needs no
 * entry here: thread.c's reads teb_key on every machine.
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

#include "ntstatus.h"
#define WIN32_NO_STATUS
#include "windef.h"
#include "winternl.h"
#include "unix_private.h"

#include "tempwind_private.h"

/* ---- process and thread setup (section 8, WP4) ---- */

void signal_init_threading( void )
{
    tw_unsupported( "signal_init_threading" );
}

void signal_init_process( void )
{
    tw_unsupported( "signal_init_process" );
}

NTSTATUS signal_alloc_thread( TEB *teb )
{
    tw_unsupported( "signal_alloc_thread" );
}

void signal_free_thread( TEB *teb )
{
    tw_unsupported( "signal_free_thread" );
}

void signal_start_thread( PRTL_THREAD_START_ROUTINE entry, void *arg, BOOL suspend, TEB *teb )
{
    tw_unsupported( "signal_start_thread" );
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

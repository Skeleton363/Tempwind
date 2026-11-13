/*
 * The version strings Wine's build would generate (dlls/ntdll/unix/version.c,
 * made from `git describe`), and the layout checks of
 * docs/design/wine-unix.md, section 3: the module is wasm64, the Windows it
 * describes is AMD64.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <stdarg.h>
#include <stddef.h>

#include "windef.h"
#include "winnt.h"

const char wine_build[] = "wine-11.0";

C_ASSERT( sizeof(ULONG_PTR) == 8 );
C_ASSERT( sizeof(INT_PTR) == 8 );
C_ASSERT( sizeof(void *) == 8 );
C_ASSERT( sizeof(CONTEXT) == 0x4d0 );

/* The CONTEXT fields the boot test reads (section 8), at AMD64's offsets. */
C_ASSERT( offsetof(CONTEXT, ContextFlags) == 0x30 );
C_ASSERT( offsetof(CONTEXT, MxCsr) == 0x34 );
C_ASSERT( offsetof(CONTEXT, SegCs) == 0x38 );
C_ASSERT( offsetof(CONTEXT, SegDs) == 0x3a );
C_ASSERT( offsetof(CONTEXT, SegEs) == 0x3c );
C_ASSERT( offsetof(CONTEXT, SegFs) == 0x3e );
C_ASSERT( offsetof(CONTEXT, SegGs) == 0x40 );
C_ASSERT( offsetof(CONTEXT, SegSs) == 0x42 );
C_ASSERT( offsetof(CONTEXT, EFlags) == 0x44 );
C_ASSERT( offsetof(CONTEXT, Rcx) == 0x80 );
C_ASSERT( offsetof(CONTEXT, Rdx) == 0x88 );
C_ASSERT( offsetof(CONTEXT, Rsp) == 0x98 );
C_ASSERT( offsetof(CONTEXT, Rip) == 0xf8 );
C_ASSERT( offsetof(CONTEXT, FltSave) == 0x100 );
C_ASSERT( offsetof(CONTEXT, FltSave.ControlWord) == 0x100 );
C_ASSERT( offsetof(CONTEXT, FltSave.MxCsr) == 0x118 );

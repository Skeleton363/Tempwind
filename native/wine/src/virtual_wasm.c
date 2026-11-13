/*
 * The memory backend under virtual.c (docs/design/wine-unix.md, sections 4,
 * 5 and 6).
 *
 * There is no host mmap: the patched virtual.c calls tw_mmap, tw_munmap,
 * tw_mprotect, tw_madvise and tw_msync (01xx patches), and they work on the
 * one wasm memory. The backend keeps its own record of what is mapped, one
 * byte per 4 KiB page up to 16 GiB; Wine keeps the protections itself, and
 * nothing faults.
 *
 * Copyright 2026 the Tempwind authors
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 */
#include <errno.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

#include "tempwind/abi.h"
#include "tempwind_private.h"

#define WASM_PAGE 0x10000ull
#define MAX_PAGES TW_MAX_MEMORY_PAGES          /* 16 GiB, the memory's maximum */
#define MEMORY_MAX (MAX_PAGES * WASM_PAGE)
#define HOST_REGION_END 0x100000000ull         /* the heap's cap (section 5) */
#define PAGE_SHIFT TW_PAGE_FLAGS_SHIFT         /* 4 KiB guest pages */
#define PAGE_SIZE (1ull << PAGE_SHIFT)
#define PAGE_COUNT (MEMORY_MAX >> PAGE_SHIFT)

/* The only memory.grow in the module: grows the memory to cover end, rounded
 * up to 64 KiB, then tells JS (memory_grown). */
int grow_to( uint64_t end )
{
    uint64_t pages = (end + WASM_PAGE - 1) / WASM_PAGE;
    uint64_t cur = __builtin_wasm_memory_size( 0 );

    if (end > MAX_PAGES * WASM_PAGE)
    {
        errno = ENOMEM;
        return -1;
    }
    if (pages <= cur) return 0;
    if (__builtin_wasm_memory_grow( 0, pages - cur ) == (size_t)-1)
    {
        errno = ENOMEM;
        return -1;
    }
    memory_grown();
    return 0;
}

/* -Wl,--wrap=sbrk: the heap never grows the memory. The break stays below
 * the end of the host region, which INITIAL_MEMORY (4 GiB) covers, so
 * emscripten_resize_heap is never reached; past it malloc returns NULL. */
void *__real_sbrk( intptr_t increment );

void *__wrap_sbrk( intptr_t increment )
{
    uintptr_t brk = (uintptr_t)__real_sbrk( 0 );

    if (increment > 0 && (uint64_t)increment > HOST_REGION_END - brk)
    {
        errno = ENOMEM;
        return (void *)-1;
    }
    return __real_sbrk( increment );
}

/* ---- The guest regions (section 4) ----------------------------------------
 * The preloader's reserve list, as a static list. virtual.c adds it to its
 * reserved areas (virtual_init); here it is where maps may go. The NULL
 * region, the gap, the host region and everything above 16 GiB are outside
 * it. The layout is virtual.c's struct preload_info, ended by a zero size. */
const struct tw_range tw_reserve_list[] =
{
    { (void *)0x10000, 0x7ffe0000 - 0x10000 },        /* the low region */
    { (void *)0x7ffe0000, 0x10000 },                 /* KUSER and the syscall slot */
    { (void *)0x100000000, 0x400000000 - 0x100000000 },  /* the high region */
    { 0, 0 }
};

/* Whether [start, end) lies in the guest regions (they may be adjacent). */
static int in_guest_regions( uint64_t start, uint64_t end )
{
    unsigned int i = 0;

    if (end <= start) return 0;
    while (tw_reserve_list[i].size)
    {
        uint64_t base = (uintptr_t)tw_reserve_list[i].addr, limit = base + tw_reserve_list[i].size;

        if (start < base || start >= limit)
        {
            i++;
            continue;
        }
        if (end <= limit) return 1;
        start = limit;  /* the rest, from the first region on */
        i = 0;
    }
    return 0;
}

/* ---- The record of maps -----------------------------------------------------
 * One byte per page, in the host heap. 0 is the initial state: mapped and clean. The whole
 * guest regions start mapped with no access, as the preloader's reservation
 * would (virtual.c sees them as reserved areas), so a try-fixed map there
 * fails with EEXIST until Wine unmaps part of them. */
#define VM_FREE  1   /* unmapped (munmap) */
#define VM_DIRTY 2   /* may hold nonzero bytes: a fixed map zeroes it first */
static uint8_t *vm_state;  /* PAGE_COUNT bytes from the host heap */

static void vm_init( void )
{
    if (!vm_state && !(vm_state = calloc( PAGE_COUNT, 1 )))
        tw_unsupported( "virtual_wasm: no host memory for the record of maps" );
}

static int range_has( uint64_t start, uint64_t end, uint8_t bit )
{
    uint64_t i;

    for (i = start >> PAGE_SHIFT; i < end >> PAGE_SHIFT; i++) if (vm_state[i] & bit) return 1;
    return 0;
}

/* Zeroes the dirty pages of [start, end) and marks them clean; the memory
 * covers them (a dirty page was mapped, so it was grown to). */
static void zero_dirty( uint64_t start, uint64_t end )
{
    uint64_t i = start >> PAGE_SHIFT, last = end >> PAGE_SHIFT;

    while (i < last)
    {
        uint64_t run;

        if (!(vm_state[i] & VM_DIRTY)) { i++; continue; }
        for (run = i; run < last && (vm_state[run] & VM_DIRTY); run++) vm_state[run] &= ~VM_DIRTY;
        memset( (void *)(uintptr_t)(i << PAGE_SHIFT), 0, (run - i) << PAGE_SHIFT );
        i = run;
    }
}

static void set_state( uint64_t start, uint64_t end, uint8_t set, uint8_t clear )
{
    uint64_t i;

    for (i = start >> PAGE_SHIFT; i < end >> PAGE_SHIFT; i++)
        vm_state[i] = (vm_state[i] & ~clear) | set;
}

/* ---- File views (section 6) ---------------------------------------------------
 * A view is a copy of the file's bytes. A writable shared view keeps a dup of
 * the fd, the offset and the size, and writes the bytes back on msync and
 * when the range is unmapped or mapped over. */
struct file_record
{
    uint64_t start;   /* address of the view's first byte */
    uint64_t size;    /* bytes that belong to the file (not past its end) */
    off_t    offset;  /* in the file */
    int      fd;      /* our dup */
    dev_t    dev;
    ino_t    ino;
};
static struct file_record *records;
static unsigned int record_count, record_capacity;

static int write_back( const struct file_record *rec, uint64_t from, uint64_t to )
{
    while (from < to)
    {
        ssize_t n = pwrite( rec->fd, (void *)(uintptr_t)(rec->start + from), to - from,
                            rec->offset + (off_t)from );
        if (n < 0)
        {
            if (errno == EINTR) continue;
            return -1;
        }
        if (!n)
        {
            errno = EIO;
            return -1;
        }
        from += n;
    }
    return 0;
}

static int add_record( const struct file_record *rec )
{
    if (record_count == record_capacity)
    {
        unsigned int cap = record_capacity ? 2 * record_capacity : 16;
        struct file_record *r = realloc( records, cap * sizeof(*r) );

        if (!r) return -1;
        records = r;
        record_capacity = cap;
    }
    records[record_count++] = *rec;
    return 0;
}

/* The range [start, end) is unmapped or mapped over: the records' bytes in it
 * are written back, and the records shrink to what lies outside it. */
static int drop_records( uint64_t start, uint64_t end )
{
    unsigned int i = 0;
    int ret = 0;

    while (i < record_count)
    {
        struct file_record *rec = &records[i];
        uint64_t rs = rec->start, re = rec->start + rec->size;
        uint64_t from = rs > start ? rs : start, to = re < end ? re : end;

        if (from >= to) { i++; continue; }
        if (write_back( rec, from - rs, to - rs )) ret = -1;
        if (from > rs && to < re)  /* a hole in the middle: the tail is a new record */
        {
            struct file_record tail = *rec;

            tail.start = to;
            tail.size = re - to;
            tail.offset = rec->offset + (off_t)(to - rs);
            tail.fd = dup( rec->fd );
            rec->size = from - rs;
            if (tail.fd < 0 || add_record( &tail )) ret = -1;
            i++;
        }
        else if (from > rs)
        {
            rec->size = from - rs;
            i++;
        }
        else if (to < re)
        {
            rec->offset += (off_t)(to - rs);
            rec->start = to;
            rec->size = re - to;
            i++;
        }
        else
        {
            close( rec->fd );
            records[i] = records[--record_count];
        }
    }
    return ret;
}

/* Copies the file into [start, start + size) and, for a writable shared view,
 * records it. Stops loudly on a second writable view of a file range. */
static int map_file( uint64_t start, uint64_t size, int prot, int flags, int fd, off_t offset )
{
    struct file_record rec;
    struct stat st;
    uint64_t done = 0;
    unsigned int i;

    if (fstat( fd, &st )) return -1;
    if ((flags & MAP_SHARED) && (prot & PROT_WRITE) && offset < st.st_size)
    {
        rec.start = start;
        rec.size = (uint64_t)(st.st_size - offset);
        if (rec.size > size) rec.size = size;
        rec.offset = offset;
        rec.dev = st.st_dev;
        rec.ino = st.st_ino;
        for (i = 0; i < record_count; i++)
        {
            const struct file_record *r = &records[i];

            if (r->dev == rec.dev && r->ino == rec.ino &&
                r->offset < rec.offset + (off_t)rec.size && rec.offset < r->offset + (off_t)r->size)
                tw_unsupported( "second writable view" );
        }
        if ((rec.fd = dup( fd )) < 0) return -1;
        if (add_record( &rec ))
        {
            close( rec.fd );
            errno = ENOMEM;
            return -1;
        }
    }
    while (done < size)
    {
        ssize_t n = pread( fd, (void *)(uintptr_t)(start + done), size - done, offset + (off_t)done );
        if (n < 0)
        {
            if (errno == EINTR) continue;
            return -1;
        }
        if (!n) break;  /* past the end of the file: the rest stays zero */
        done += n;
    }
    return 0;
}

/* ---- The calls virtual.c makes -------------------------------------------------- */

/* Maps [start, start + size), which lies in the guest regions: grows the
 * memory to cover it, zeroes it, then copies the file for a file view. */
static void *map_range( uint64_t start, uint64_t size, int prot, int flags, int fd, off_t offset )
{
    int file = !(flags & MAP_ANON) && fd != -1;
    uint64_t end = start + size;

    if (grow_to( end )) return MAP_FAILED;
    if (drop_records( start, end )) return MAP_FAILED;
    zero_dirty( start, end );
    set_state( start, end, (prot != PROT_NONE || file) ? VM_DIRTY : 0, VM_FREE );
    if (file && map_file( start, size, prot, flags, fd, offset )) return MAP_FAILED;
    return (void *)(uintptr_t)start;
}

static int range_free( uint64_t start, uint64_t end )
{
    uint64_t i;

    for (i = start >> PAGE_SHIFT; i < end >> PAGE_SHIFT; i++) if (!(vm_state[i] & VM_FREE)) return 0;
    return 1;
}

/* A free range of size bytes in the guest regions: at the hint if it is free
 * there, else the lowest one 64 KiB aligned; 0 if there is none. */
static uint64_t find_free( uint64_t hint, uint64_t size )
{
    const struct tw_range *r;

    if (hint && !(hint & (PAGE_SIZE - 1)) && in_guest_regions( hint, hint + size ) &&
        range_free( hint, hint + size ))
        return hint;
    for (r = tw_reserve_list; r->size; r++)
    {
        uint64_t base = (uintptr_t)r->addr, limit = base + r->size, start;

        for (start = (base + WASM_PAGE - 1) & ~(WASM_PAGE - 1); start + size <= limit; start += WASM_PAGE)
            if (range_free( start, start + size )) return start;
    }
    return 0;
}

void *tw_mmap( void *addr, size_t len, int prot, int flags, int fd, off_t offset )
{
    uint64_t start = (uintptr_t)addr;
    uint64_t size = (len + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1);

    vm_init();
    if (!len || (offset & (PAGE_SIZE - 1)) || (!(flags & MAP_ANON) && fd != -1 && offset < 0))
    {
        errno = EINVAL;
        return MAP_FAILED;
    }
    if (flags & (MAP_FIXED | MAP_FIXED_NOREPLACE))
    {
        if (start & (PAGE_SIZE - 1))
        {
            errno = EINVAL;
            return MAP_FAILED;
        }
        if (!in_guest_regions( start, start + size ))
        {
            /* not ours to map: the host region is the allocator's */
            errno = (flags & MAP_FIXED) ? ENOMEM : EEXIST;
            return MAP_FAILED;
        }
        if (!(flags & MAP_FIXED) && !range_free( start, start + size ))
        {
            errno = EEXIST;
            return MAP_FAILED;
        }
    }
    else if (!(start = find_free( start, size )))
    {
        errno = ENOMEM;
        return MAP_FAILED;
    }
    return map_range( start, size, prot, flags, fd, offset );
}

int tw_munmap( void *addr, size_t len )
{
    uint64_t start = (uintptr_t)addr;
    uint64_t end = start + ((len + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1));

    vm_init();
    if (!len || (start & (PAGE_SIZE - 1)) || !in_guest_regions( start, end ))
    {
        errno = EINVAL;
        return -1;
    }
    if (drop_records( start, end )) return -1;
    set_state( start, end, VM_FREE, 0 );  /* the memory never shrinks */
    return 0;
}

/* Bookkeeping only: Wine keeps the protections. An accessible page may be
 * written, so a later fixed map zeroes it. */
int tw_mprotect( void *addr, size_t len, int prot )
{
    uint64_t start = (uintptr_t)addr;
    uint64_t end = start + ((len + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1));

    vm_init();
    if (start & (PAGE_SIZE - 1))
    {
        errno = EINVAL;
        return -1;
    }
    if (!len) return 0;
    if (!in_guest_regions( start, end ) || range_has( start, end, VM_FREE ))
    {
        errno = ENOMEM;
        return -1;
    }
    if (prot != PROT_NONE)
    {
        if (grow_to( end )) return -1;  /* a page of the initial reservation */
        set_state( start, end, VM_DIRTY, 0 );
    }
    return 0;
}

int tw_madvise( void *addr, size_t len, int advice )
{
    return 0;
}

/* Writes back the writable shared views in the range. */
int tw_msync( void *addr, size_t len, int flags )
{
    uint64_t start = (uintptr_t)addr;
    uint64_t end = start + ((len + PAGE_SIZE - 1) & ~(PAGE_SIZE - 1));
    unsigned int i;
    int ret = 0;

    vm_init();
    if (start & (PAGE_SIZE - 1))
    {
        errno = EINVAL;
        return -1;
    }
    if (len && (!in_guest_regions( start, end ) || range_has( start, end, VM_FREE )))
    {
        errno = ENOMEM;
        return -1;
    }
    for (i = 0; i < record_count; i++)
    {
        const struct file_record *rec = &records[i];
        uint64_t rs = rec->start, re = rec->start + rec->size;
        uint64_t from = rs > start ? rs : start, to = re < end ? re : end;

        if (from < to && write_back( rec, from - rs, to - rs )) ret = -1;
    }
    return ret;
}

/* ---- The page-state table (abi.md, "Page-state table") ------------------------
 * One byte per 4 KiB page up to 16 GiB, from the host heap. */
static uint8_t *page_flags;

void *tw_page_flags( void )
{
    if (!page_flags && !(page_flags = calloc( PAGE_COUNT, 1 )))
        tw_unsupported( "tw_page_flags: no host memory for the page-state table" );
    return page_flags;
}

/* PAGE_WATCH follows Wine's VPROT_WRITEWATCH on [addr, addr + size), rounded
 * out to pages as virtual.c does. */
void tw_page_watch( const void *addr, size_t size, int on )
{
    uint8_t *flags = tw_page_flags();
    uint64_t i = (uintptr_t)addr >> PAGE_SHIFT;
    uint64_t end = ((uintptr_t)addr + size + PAGE_SIZE - 1) >> PAGE_SHIFT;

    if (end > PAGE_COUNT) end = PAGE_COUNT;
    for (; i < end; i++)
        flags[i] = on ? (flags[i] | TW_PAGE_WATCH) : (flags[i] & ~TW_PAGE_WATCH);
}

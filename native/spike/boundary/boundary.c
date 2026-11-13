// The Unix side of the M1 boundary spike (docs/design/abi.md, "Wine boundary").
// No Wine code: this stands in for Wine's Unix side to test the contract.
//
// - It owns the run loop: it imports a translated module's `run` (wasm to
//   wasm) and handles EXIT_LEFT_CODE through the plain import `bnd_translate`.
// - `host_dispatch` is a raw export of the guest-function type that the driver
//   puts in env.table; the entry argument says which host function it is.
// - `green_block` (a WebAssembly.Suspending import) is the only switch point;
//   `green_switch` saves and restores the save set around it.
// - Guest memory is this module's own linear memory: a guest address is a
//   pointer.
//
// The ABI's constants come from tempwind/abi.h (x64wasm-cli gen-header), so the
// spike checks the header against real translated modules. The layout
// constants match guest_a.S, guest_b.S and driver.mjs.

#include <emscripten/stack.h>
#include <errno.h>
#include <setjmp.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "tempwind/abi.h"

// State-block registers as uint64_t indexes (abi.h's byte offsets / 8).
enum {
  RAX = TW_STATE_GPR(0) / 8,
  RCX = TW_STATE_GPR(1) / 8,
  RDX = TW_STATE_GPR(2) / 8,
  RSP = TW_STATE_GPR(4) / 8,
  RIP = TW_STATE_RIP / 8,
};
// The Unix side's own reason for a callback's return (spike-local; abi.md
// leaves the value to M3).
#define EXIT_CB_RETURN 0x100
// The end of the host region (abi.md, "The host region"): the heap's cap.
#define HOST_REGION_END 0x100000000ull

// Host function ids (the lookup slot's entry) and their guest addresses.
enum { HOST_ADD = 1, HOST_BLOCK = 2, HOST_CALLBACK = 3, HOST_CB_RETURN = 4, HOST_EXIT = 5, HOST_GROW = 6 };
#define THUNK_BASE 0x7ff00000ull
#define THUNK(id) (THUNK_BASE + 16ull * (id))

// Imports. The JS library (library.js) declares them; the driver replaces
// bnd_run, bnd_translate, green_block and bnd_depth in Emscripten's
// instantiateWasm hook.
int32_t bnd_run(uint64_t state);           // the translated module's raw `run`
int32_t bnd_translate(uint64_t rip);       // plain: link code for rip; 0 = linked
int64_t green_block(int32_t tid);          // WebAssembly.Suspending
int32_t bnd_depth(void);                   // plain: env.depth's value; the driver records it
void memory_grown(void);                   // plain: JS refreshes its views
void bnd_glue_string(const char *s);       // plain: the glue reads s through its views

// compiler-rt's stack_ops.S; the driver calls the same functions as raw exports.
void _emscripten_stack_restore(uintptr_t sp);
// The linker's start of the heap.
extern unsigned char __heap_base;

struct thread {
  uint64_t *state;
  int32_t tid;
  int32_t exit_code;
};

// The current thread: its state-block pointer is part of the save set.
static struct thread *cur;
// Address-taken locals escape here, so they must live on the shadow stack.
static volatile uint64_t *escaped[8];

static uint64_t *gpr(uint64_t state) { return (uint64_t *)(uintptr_t)state; }
static uint64_t load64(uint64_t addr) { return *(volatile uint64_t *)(uintptr_t)addr; }

// The guest's `ret` (abi.md, "Host functions").
static int32_t guest_ret(uint64_t *g, uint64_t ret) {
  uint64_t t = load64(g[RSP]);
  g[RSP] += 8;
  if (t == ret) return TW_STATUS_RETURNED;
  g[RIP] = t;
  return TW_STATUS_UNWIND;
}

static int32_t stop(uint64_t *g, uint32_t id, uint32_t reason) {
  g[RIP] = THUNK(id);
  *(uint32_t *)((char *)g + TW_STATE_EXIT) = reason;
  return TW_STATUS_EXIT;
}

// The heap never grows the memory (wine-unix.md, section 5): every sbrk
// (-Wl,--wrap=sbrk) refuses a break past the host region's end, so malloc
// returns NULL there and emscripten_resize_heap is never reached.
void *__real_sbrk(intptr_t increment);
void *__wrap_sbrk(intptr_t increment) {
  uintptr_t brk = (uintptr_t)__real_sbrk(0);
  if (increment > 0 && (uintptr_t)increment > HOST_REGION_END - brk) {
    errno = ENOMEM;
    return (void *)-1;
  }
  return __real_sbrk(increment);
}

// The only switch point. Saves the save set (abi.md, "Green threads"), blocks,
// restores it. Another green thread may run on its own stack while we wait.
static int64_t green_switch(int32_t tid) {
  uintptr_t sp = emscripten_stack_get_current();
  uintptr_t base = emscripten_stack_get_base();
  uintptr_t end = emscripten_stack_get_end();
  int saved_errno = errno;
  struct thread *saved_cur = cur;
  int64_t v = green_block(tid);
  emscripten_stack_set_limits((void *)base, (void *)end);
  _emscripten_stack_restore(sp);
  errno = saved_errno;
  cur = saved_cur;
  return v;
}

// The C run loop (abi.md, "Who calls whom", 2). Returns the exit reason it
// does not handle itself.
static uint32_t run_loop(uint64_t *g) {
  for (;;) {
    uint32_t r = (uint32_t)bnd_run((uint64_t)(uintptr_t)g);
    if (r == TW_EXIT_LEFT_CODE && bnd_translate(g[RIP]) == 0) continue;
    return r;
  }
}

static int32_t host_block(uint64_t *g, uint64_t ret) {
  int32_t tid = (int32_t)g[RCX];
  volatile uint64_t local = 0xA0000ull + (uint64_t)tid;
  escaped[tid & 7] = &local;
  errno = 1000 + tid;
  uintptr_t sp0 = emscripten_stack_get_current();
  uintptr_t base0 = emscripten_stack_get_base(), end0 = emscripten_stack_get_end();
  struct thread *cur0 = cur;

  int64_t v = green_switch(tid);

  uint32_t bad = 0;
  if (*escaped[tid & 7] != 0xA0000ull + (uint64_t)tid || local != 0xA0000ull + (uint64_t)tid) bad |= 1;
  if (errno != 1000 + tid) bad |= 2;
  if (emscripten_stack_get_current() != sp0) bad |= 4;
  if (emscripten_stack_get_base() != base0 || emscripten_stack_get_end() != end0) bad |= 8;
  if (cur != cur0 || cur->state != g || cur->tid != tid) bad |= 16;
  // The local lies in this thread's own stack region.
  uintptr_t at = (uintptr_t)&local;
  if (at >= base0 || at < end0) bad |= 32;
  g[RAX] = bad ? 0xBAD00000ull | bad : (uint64_t)v;
  return guest_ret(g, ret);
}

// A nested guest call (abi.md, "Who calls whom", 5): save the caller's state,
// run the loop on fn(arg) until it returns to the cb_return address, restore.
// `run` keeps the caller's depth (abi.md v5): env.depth before and after the
// nested loop must be equal and nonzero (the outer guest frames are live);
// else the callback's result is 0xBADD0000. The driver also records both.
static int32_t host_callback(uint64_t *g, uint64_t ret) {
  uint64_t fn = g[RCX], arg = g[RDX];
  uint64_t saved[TW_STATE_SIZE / 8];
  memcpy(saved, g, TW_STATE_SIZE);
  uint64_t rsp = ((g[RSP] - 0x100) & ~15ull) - 0x28;  // [rsp] = return address, 32 bytes of home space
  *(uint64_t *)(uintptr_t)rsp = THUNK(HOST_CB_RETURN);
  g[RSP] = rsp;
  g[RCX] = arg;
  g[RIP] = fn;
  int32_t depth0 = bnd_depth();
  uint32_t r = run_loop(g);
  int32_t depth1 = bnd_depth();
  uint64_t result = g[RAX];
  if (depth0 == 0 || depth1 != depth0) {
    fprintf(stderr, "boundary: env.depth %d before the nested loop, %d after\n", depth0, depth1);
    result = 0xBADD0000ull;
  }
  memcpy(g, saved, TW_STATE_SIZE);
  if (r != EXIT_CB_RETURN) {
    fprintf(stderr, "boundary: nested loop ended with exit %u, expected the callback's return\n", r);
    cur->exit_code = -(int32_t)r;
    return stop(g, HOST_CALLBACK, TW_EXIT_HOST);
  }
  g[RAX] = result;
  return guest_ret(g, ret);
}

// The cap check's result (cap_check): -1 not run, 0 passed, else the failed
// checks' bits.
static int32_t cap_bad = -1;

// Grows memory past its current size with memory.grow (the virtual backend's
// job in Wine), writes past the old end, then tells JS. Then the heap's cap:
// a malloc that would move the break past the host region's end returns NULL
// with ENOMEM and leaves the memory's size alone (wine-unix.md, section 5).
static int32_t host_grow(uint64_t *g, uint64_t ret) {
  uint64_t tid = g[RCX];
  size_t old_pages = __builtin_wasm_memory_size(0);
  if (__builtin_wasm_memory_grow(0, 1) == (size_t)-1) {
    fprintf(stderr, "boundary: memory.grow failed at %zu pages\n", old_pages);
    cur->exit_code = -1;
    return stop(g, HOST_GROW, TW_EXIT_HOST);
  }
  uint64_t addr = (uint64_t)old_pages * 65536 + 0x100;
  *(volatile uint64_t *)(uintptr_t)addr = 0x600DF00D00000000ull | tid;
  strcpy((char *)(uintptr_t)(addr + 0x100), "past the old end");
  memory_grown();
  bnd_glue_string((const char *)(uintptr_t)(addr + 0x100));

  size_t pages = __builtin_wasm_memory_size(0);
  uintptr_t brk = (uintptr_t)sbrk(0);
  errno = 0;
  // More than the whole heap below the cap, so it crosses the cap whatever is free.
  void *p = malloc(HOST_REGION_END - (uintptr_t)&__heap_base + 1);
  int err = errno;
  size_t pages_after = __builtin_wasm_memory_size(0);
  cap_bad = (pages * 65536 <= HOST_REGION_END) | (p != NULL) << 1 | (err != ENOMEM) << 2 |
            (pages_after != pages) << 3 | ((uintptr_t)sbrk(0) != brk) << 4;
  if (cap_bad)
    fprintf(stderr, "boundary: cap check %#x: brk %#zx, malloc %p, errno %d, pages %zu -> %zu\n",
            (unsigned)cap_bad, (size_t)brk, p, err, pages, pages_after);
  free(p);

  g[RAX] = addr;
  return guest_ret(g, ret);
}

__attribute__((export_name("cap_check")))
int32_t cap_check(void) { return cap_bad; }

// Probes the lookup table at `header` for `addr` with abi.h's layout and
// tw_lookup_hash, as translated code does; returns the slot's entry (at +12:
// key u64, func u32, entry u32), or -1 on a miss.
__attribute__((export_name("lookup_entry")))
int32_t lookup_entry(uint64_t header, uint64_t addr) {
  uint64_t slots = load64(header + TW_LOOKUP_SLOTS), mask = load64(header + TW_LOOKUP_MASK);
  for (uint64_t i = tw_lookup_hash(addr, mask);; i = (i + 1) & mask) {
    uint64_t at = slots + i * TW_LOOKUP_SLOT_SIZE;
    uint64_t key = load64(at);
    if (key == 0) return -1;
    if (key == addr) return *(volatile int32_t *)(uintptr_t)(at + 12);
  }
}

// Every host function: the raw export in env.table (abi.md, "Host functions").
__attribute__((export_name("host_dispatch")))
int32_t host_dispatch(uint64_t state, uint32_t entry, uint64_t ret) {
  uint64_t *g = gpr(state);
  switch (entry) {
  case HOST_ADD:
    g[RAX] = g[RCX] + g[RDX];
    return guest_ret(g, ret);
  case HOST_BLOCK:
    return host_block(g, ret);
  case HOST_CALLBACK:
    return host_callback(g, ret);
  case HOST_CB_RETURN:
    return stop(g, HOST_CB_RETURN, EXIT_CB_RETURN);
  case HOST_EXIT:
    cur->exit_code = (int32_t)g[RCX];
    return stop(g, HOST_EXIT, TW_EXIT_HOST);
  case HOST_GROW:
    return host_grow(g, ret);
  default:
    fprintf(stderr, "boundary: unknown host function %u\n", entry);
    cur->exit_code = -2;
    return stop(g, entry, TW_EXIT_HOST);
  }
}

// A green thread (abi.md, "Green threads"). The driver sets the stack pointer
// and limits through the raw stack exports first, then calls this through
// WebAssembly.promising.
__attribute__((export_name("thread_main")))
int32_t thread_main(uint64_t state, int32_t tid) {
  struct thread t = {gpr(state), tid, 0};
  cur = &t;
  uint32_t r = run_loop(t.state);
  if (r != TW_EXIT_HOST) {
    fprintf(stderr, "boundary: thread %d stopped with exit %u at rip %#llx\n", tid, r,
            (unsigned long long)t.state[RIP]);
    return -(int32_t)r;
  }
  return t.exit_code;
}

// The suspend/resume round trip, n times.
__attribute__((export_name("bench_main")))
int32_t bench_main(int32_t n) {
  int32_t done = 0;
  for (int32_t i = 0; i < n; i++) done += green_switch(-1) == 0;
  return done;
}

// setjmp/longjmp through wasm exception handling (-sSUPPORT_LONGJMP=wasm).
static jmp_buf jb;
__attribute__((noinline)) static void jump(int v) { longjmp(jb, v); }
__attribute__((export_name("longjmp_check")))
int32_t longjmp_check(void) {
  volatile int32_t steps = 0;
  int v = setjmp(jb);
  steps++;
  if (v == 0) jump(42);
  return v == 42 && steps == 2 ? 1 : 0;
}

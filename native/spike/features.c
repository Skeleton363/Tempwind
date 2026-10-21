/* Compile-time feature probes for the Emscripten spike (Phase 0, item 6).
 *
 * run.sh compiles this file once per PROBE_* macro with -Werror, for both
 * x86_64-linux (gcc) and wasm64-emscripten (emcc), and records whether it
 * compiles (and, for PROBE_LINK_*, links). Each probe is a language or
 * library feature that Wine's x86-64 Unix side uses.
 */
#define _GNU_SOURCE
#include <stddef.h>

#if defined(PROBE_MS_ABI)
/* include/minwindef.h: __stdcall is __attribute__((ms_abi)) on x86_64 PE builds. */
__attribute__((ms_abi)) int probe(int a) { return a; }

#elif defined(PROBE_MS_VA_LIST)
/* include/minwindef.h: __ms_va_list is __builtin_ms_va_list on x86_64. */
int probe(int n, ...)
{
    __builtin_ms_va_list ap;
    __builtin_ms_va_start(ap, n);
    __builtin_ms_va_end(ap);
    return n;
}

#elif defined(PROBE_X86_INLINE_ASM)
/* dlls/ntdll/unix/signal_x86_64.c, system.c, thread.c: x86 inline asm. */
int probe(void) { int r; __asm__ volatile("movl $1, %0" : "=r"(r)); return r; }

#elif defined(PROBE_TOPLEVEL_ASM)
/* include/wine/asm.h __ASM_GLOBAL_FUNC: hand-written functions in top-level asm. */
__asm__(".globl probe_asm\nprobe_asm:\n\tret\n");
int probe(void) { return 0; }

#elif defined(PROBE_NAKED)
__attribute__((naked)) void probe(void) { __asm__("ret"); }

#elif defined(PROBE_THREAD_LOCAL)
static __thread int tls;
int probe(void) { return ++tls; }

#elif defined(PROBE_UCONTEXT)
/* signal_x86_64.c reads and writes registers through ucontext_t (RIP_sig etc.). */
#include <ucontext.h>
int probe(ucontext_t *uc) { return (int)sizeof(uc->uc_mcontext); }

#elif defined(PROBE_UCONTEXT_GREGS)
#include <ucontext.h>
long probe(ucontext_t *uc) { return (long)uc->uc_mcontext.gregs[0]; }

#elif defined(PROBE_LINUX_FUTEX)
/* dlls/ntdll/unix/sync.c: syscall(__NR_futex, ...) from <linux/futex.h>. */
#include <linux/futex.h>
#include <sys/syscall.h>
#include <unistd.h>
long probe(int *a) { return syscall(__NR_futex, a, FUTEX_WAIT_PRIVATE, 0, NULL, 0, 0); }

#elif defined(PROBE_EPOLL)
/* server/fd.c uses epoll on Linux. */
#include <sys/epoll.h>
int probe(void) { return epoll_create1(0); }

#elif defined(PROBE_PTRACE)
/* server/ptrace.c: thread contexts and debug registers via ptrace. */
#include <sys/ptrace.h>
long probe(int pid) { return ptrace(PTRACE_ATTACH, pid, 0, 0); }

#elif defined(PROBE_INOTIFY)
/* server/change.c: directory change notifications. */
#include <sys/inotify.h>
int probe(void) { return inotify_init(); }

#elif defined(PROBE_SCM_RIGHTS)
/* server/request.c, dlls/ntdll/unix/server.c: fd passing. */
#include <sys/socket.h>
int probe(void) { return SCM_RIGHTS + (int)CMSG_SPACE(sizeof(int)); }

#elif defined(PROBE_DLOPEN)
/* dlls/ntdll/unix/loader.c: dlopen of *.so Unix libraries. */
#include <dlfcn.h>
void *probe(const char *n) { return dlopen(n, RTLD_NOW); }

#elif defined(PROBE_MAP_FIXED_NOREPLACE)
#include <sys/mman.h>
int probe(void) { return MAP_FIXED_NOREPLACE | MAP_NORESERVE; }

#elif defined(PROBE_INT128)
unsigned __int128 probe(unsigned long a, unsigned long b) { return (unsigned __int128)a * b; }

#elif defined(PROBE_BUILTIN_FRAME_ADDRESS)
void *probe(void) { return __builtin_frame_address(0); }

#elif defined(PROBE_BUILTIN_RETURN_ADDRESS)
void *probe(void) { return __builtin_return_address(0); }

#elif defined(PROBE_LINK_FORK)
#include <unistd.h>
int main(void) { return fork() < 0; }

#elif defined(PROBE_LINK_POSIX_SPAWN)
#include <spawn.h>
extern char **environ;
int main(void) { pid_t p; char *a[] = { "true", NULL }; return posix_spawn(&p, "/bin/true", NULL, NULL, a, environ); }

#elif defined(PROBE_LINK_GETCONTEXT)
#include <ucontext.h>
int main(void) { ucontext_t uc; return getcontext(&uc); }

#elif defined(PROBE_LINK_SIGALTSTACK)
#include <signal.h>
int main(void) { stack_t ss = { 0 }; return sigaltstack(NULL, &ss); }

#elif defined(PROBE_LINK_SETJMP)
#include <setjmp.h>
static jmp_buf jb;
int main(void) { if (setjmp(jb)) return 0; longjmp(jb, 1); }

#else
#error no probe selected
#endif

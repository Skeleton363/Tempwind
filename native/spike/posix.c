/* POSIX behaviour probe for the Emscripten spike (Phase 0, item 6).
 *
 * Wine's x86-64 Unix side relies on POSIX behaviour, not just type sizes:
 * faulting on bad pointers, mmap at fixed addresses, page protection, shared
 * file mappings, fork, fd passing over Unix sockets, asynchronous signals to
 * other threads. This probe checks each one at run time and prints one JSON
 * object, so run.sh can show native x86_64-linux next to wasm64-emscripten.
 * Nothing here is a pass/fail gate; it documents what we must replace.
 */
#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <poll.h>
#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <sys/uio.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

static int first = 1;

static void result(const char *name, int ok, const char *fmt, ...) __attribute__((format(printf, 3, 4)));
static void result(const char *name, int ok, const char *fmt, ...)
{
    char detail[256];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(detail, sizeof(detail), fmt, ap);
    va_end(ap);
    for (char *p = detail; *p; p++) if (*p == '"' || *p == '\\') *p = '\'';
    printf("%s\n  \"%s\": {\"ok\": %s, \"detail\": \"%s\"}", first ? "" : ",", name, ok ? "true" : "false", detail);
    first = 0;
    fflush(stdout);
}

static double now_ms(void)
{
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return ts.tv_sec * 1e3 + ts.tv_nsec / 1e6;
}

/* ---- fault detection: does touching a bad address raise SIGSEGV/SIGBUS? ---- */

static sigjmp_buf fault_jmp;
static volatile sig_atomic_t fault_armed;

static void fault_handler(int sig)
{
    if (fault_armed) siglongjmp(fault_jmp, sig);
    _exit(99);
}

static int touch_faults(volatile char *p, int write)
{
    int sig;
    fault_armed = 1;
    if ((sig = sigsetjmp(fault_jmp, 1)) == 0) {
        if (write) *p = 1; else (void)*p;
        fault_armed = 0;
        return 0;
    }
    fault_armed = 0;
    return sig;
}

/* ---- signals ---- */

static atomic_int usr1_count;
static void usr1_handler(int sig) { (void)sig; atomic_fetch_add(&usr1_count, 1); }

static atomic_int busy_stop;
static void *busy_thread(void *arg)
{
    (void)arg;
    while (!atomic_load(&busy_stop)) { /* spin, never yield */ }
    return NULL;
}

static __thread int tls_value;
static void *tls_thread(void *arg)
{
    tls_value = 42;
    *(int *)arg = tls_value;
    return NULL;
}

int main(void)
{
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = fault_handler;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGSEGV, &sa, NULL);
    sigaction(SIGBUS, &sa, NULL);

    printf("{");

    {
        long ps = sysconf(_SC_PAGESIZE);
        result("page_size_4k", ps == 4096, "sysconf(_SC_PAGESIZE) = %ld", ps);
        long n = sysconf(_SC_NPROCESSORS_ONLN);
        result("nprocessors", n > 0, "sysconf(_SC_NPROCESSORS_ONLN) = %ld", n);
    }

    {
        int sig = touch_faults((volatile char *)(uintptr_t)0, 0);
        result("null_read_faults", sig != 0, "%s (signal %d)", sig ? "faulted" : "read of address 0 succeeded", sig);
    }

    {
        long ps = sysconf(_SC_PAGESIZE);
        char *p = mmap(NULL, ps, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        if (p == MAP_FAILED) result("mprotect_none_faults", 0, "mmap failed: %s", strerror(errno));
        else {
            int r = mprotect(p, ps, PROT_NONE);
            int sig = touch_faults(p, 1);
            result("mprotect_none_faults", sig != 0, "mprotect=%d, write after PROT_NONE: %s", r,
                   sig ? "faulted" : "succeeded");
            r = mprotect(p, ps, PROT_READ);
            sig = touch_faults(p, 1);
            result("mprotect_readonly_faults", sig != 0, "mprotect=%d, write after PROT_READ: %s", r,
                   sig ? "faulted" : "succeeded");
            mprotect(p, ps, PROT_READ | PROT_WRITE);
            munmap(p, ps);
        }
    }

    {
        void *want = (void *)(uintptr_t)0x10000000; /* 256 MiB, inside a 16 MiB+ wasm memory only if grown */
        void *p = mmap(want, 0x10000, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED, -1, 0);
        result("mmap_fixed", p == want, "MAP_FIXED at %p -> %p (%s)", want, p == MAP_FAILED ? NULL : p,
               p == MAP_FAILED ? strerror(errno) : "ok");
        if (p != MAP_FAILED) munmap(p, 0x10000);
        p = mmap(want, 0x10000, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
        result("mmap_hint_honoured", p == want, "hint %p -> %p (%s)", want, p == MAP_FAILED ? NULL : p,
               p == MAP_FAILED ? strerror(errno) : "ok");
        if (p != MAP_FAILED) munmap(p, 0x10000);
    }

    {
        size_t sz = (size_t)1 << 30;
        double t0 = now_ms();
        char *p = mmap(NULL, sz, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
        double t1 = now_ms();
        result("reserve_1g_cheap", p != MAP_FAILED && t1 - t0 < 5.0, "PROT_NONE NORESERVE 1 GiB: %s in %.2f ms",
               p == MAP_FAILED ? strerror(errno) : "ok", t1 - t0);
        if (p != MAP_FAILED) munmap(p, sz);
        sz = (size_t)64 << 30;
        p = mmap(NULL, sz, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
        result("reserve_64g", p != MAP_FAILED, "PROT_NONE NORESERVE 64 GiB: %s",
               p == MAP_FAILED ? strerror(errno) : "ok");
        if (p != MAP_FAILED) munmap(p, sz);
    }

    {
        char path[] = "/tmp/tw-posix-XXXXXX";
        int fd = mkstemp(path);
        long ps = sysconf(_SC_PAGESIZE);
        if (fd < 0 || ftruncate(fd, ps) != 0) result("mmap_shared_alias", 0, "temp file: %s", strerror(errno));
        else {
            char *a = mmap(NULL, ps, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
            char *b = mmap(NULL, ps, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
            if (a == MAP_FAILED || b == MAP_FAILED) result("mmap_shared_alias", 0, "mmap MAP_SHARED: %s", strerror(errno));
            else {
                char c = 0;
                strcpy(a, "hello");
                int alias = strcmp(b, "hello") == 0;
                ssize_t n = pread(fd, &c, 1, 0);
                result("mmap_shared_alias", alias, "write via view A seen in view B: %s", alias ? "yes" : "no");
                result("mmap_shared_writes_file", n == 1 && c == 'h', "pread after store, before msync: %s",
                       n == 1 && c == 'h' ? "sees store" : "stale");
                munmap(a, ps);
                munmap(b, ps);
            }
            close(fd);
            unlink(path);
        }
    }

    {
        fflush(stdout);
        pid_t pid = fork();
        if (pid == 0) _exit(7);
        if (pid < 0) result("fork", 0, "fork: %s", strerror(errno));
        else {
            int st = 0;
            waitpid(pid, &st, 0);
            result("fork", WIFEXITED(st) && WEXITSTATUS(st) == 7, "child exit status %d", WEXITSTATUS(st));
        }
    }

    {
        int sv[2];
        if (socketpair(AF_UNIX, SOCK_STREAM, 0, sv) != 0) result("socketpair_scm_rights", 0, "socketpair: %s", strerror(errno));
        else {
            int pfd[2];
            char byte = 'x';
            char cbuf[CMSG_SPACE(sizeof(int))];
            struct iovec iov = { &byte, 1 };
            struct msghdr msg;
            memset(&msg, 0, sizeof(msg));
            memset(cbuf, 0, sizeof(cbuf));
            msg.msg_iov = &iov;
            msg.msg_iovlen = 1;
            msg.msg_control = cbuf;
            msg.msg_controllen = sizeof(cbuf);
            if (pipe(pfd) != 0) pfd[0] = pfd[1] = -1;
            struct cmsghdr *cm = CMSG_FIRSTHDR(&msg);
            cm->cmsg_level = SOL_SOCKET;
            cm->cmsg_type = SCM_RIGHTS;
            cm->cmsg_len = CMSG_LEN(sizeof(int));
            memcpy(CMSG_DATA(cm), &pfd[0], sizeof(int));
            ssize_t s = sendmsg(sv[0], &msg, 0);
            int sent_errno = errno;
            int got = -1;
            if (s == 1) {
                memset(cbuf, 0, sizeof(cbuf));
                msg.msg_controllen = sizeof(cbuf);
                if (recvmsg(sv[1], &msg, 0) == 1 && (cm = CMSG_FIRSTHDR(&msg)) && cm->cmsg_type == SCM_RIGHTS)
                    memcpy(&got, CMSG_DATA(cm), sizeof(int));
            }
            result("socketpair_scm_rights", got >= 0, "sendmsg=%zd (%s), received fd %d", s,
                   s == 1 ? "ok" : strerror(sent_errno), got);
            close(sv[0]);
            close(sv[1]);
            if (pfd[0] >= 0) { close(pfd[0]); close(pfd[1]); }
            if (got >= 0) close(got);
        }
    }

    {
        int pfd[2];
        if (pipe(pfd) != 0) result("pipe_poll", 0, "pipe: %s", strerror(errno));
        else {
            struct pollfd p = { pfd[0], POLLIN, 0 };
            int before = poll(&p, 1, 0);
            ssize_t w = write(pfd[1], "x", 1);
            int after = poll(&p, 1, 100);
            result("pipe_poll", before == 0 && w == 1 && after == 1 && (p.revents & POLLIN),
                   "poll before write=%d, after=%d", before, after);
            close(pfd[0]);
            close(pfd[1]);
        }
    }

    {
        struct sigaction u;
        memset(&u, 0, sizeof(u));
        u.sa_handler = usr1_handler;
        sigemptyset(&u.sa_mask);
        sigaction(SIGUSR1, &u, NULL);
        raise(SIGUSR1);
        result("raise_same_thread", atomic_load(&usr1_count) == 1, "handler ran %d time(s)", atomic_load(&usr1_count));

        pthread_t th;
        atomic_store(&usr1_count, 0);
        if (pthread_create(&th, NULL, busy_thread, NULL) != 0) result("pthread_kill_async", 0, "pthread_create failed");
        else {
            int r = pthread_kill(th, SIGUSR1);
            double t0 = now_ms();
            while (atomic_load(&usr1_count) == 0 && now_ms() - t0 < 1000) { }
            int n = atomic_load(&usr1_count);
            atomic_store(&busy_stop, 1);
            pthread_join(th, NULL);
            result("pthread_kill_async", r == 0 && n == 1,
                   "pthread_kill=%d; handler on a spinning thread within 1 s: %s", r, n ? "yes" : "no");
        }
    }

    {
        stack_t ss;
        memset(&ss, 0, sizeof(ss));
        ss.ss_sp = malloc(65536);
        ss.ss_size = 65536;
        int r = sigaltstack(&ss, NULL);
        result("sigaltstack", r == 0, "sigaltstack=%d (%s)", r, r ? strerror(errno) : "ok");
    }

    {
        int v = 0;
        pthread_t th;
        tls_value = 1;
        pthread_create(&th, NULL, tls_thread, &v);
        pthread_join(th, NULL);
        result("thread_local", v == 42 && tls_value == 1, "thread saw %d, main keeps %d", v, tls_value);
    }

    printf("\n}\n");
    fflush(stdout);
    return 0;
}

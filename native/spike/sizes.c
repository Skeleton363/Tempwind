/* Type-size and ABI probe for the Emscripten spike (Phase 0, item 6).
 *
 * Built twice by native/spike/run.sh: natively with gcc for x86_64-linux-gnu
 * (the target Wine's x86-64 Unix side is normally built for) and with
 * emcc -sMEMORY64=1 -pthread for wasm64-emscripten. Prints one JSON object on
 * stdout; compare.mjs diffs the two outputs.
 *
 * It also starts NTHREADS pthreads that bump an atomic counter and a
 * mutex-protected counter, joins them and checks the totals, to show that
 * pthreads work under MEMORY64.
 */
#define _GNU_SOURCE
#include <float.h>
#include <limits.h>
#include <pthread.h>
#include <setjmp.h>
#include <signal.h>
#include <stdalign.h>
#include <stdarg.h>
#include <stdatomic.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/time.h>
#include <sys/types.h>
#include <time.h>
#include <wchar.h>

#define NTHREADS 4
#define ITERS 100000

static int first = 1;

static void key(const char *k)
{
    printf("%s\n  \"%s\": ", first ? "" : ",", k);
    first = 0;
}

#define TYPE(name, T) do { \
    key(name); \
    printf("{\"size\": %zu, \"align\": %zu}", sizeof(T), (size_t)alignof(T)); \
} while (0)

#define TYPE_EXTRA(name, T, fmt, ...) do { \
    key(name); \
    printf("{\"size\": %zu, \"align\": %zu, " fmt "}", sizeof(T), (size_t)alignof(T), __VA_ARGS__); \
} while (0)

#define IS_SIGNED(T) ((T)-1 < (T)1)

#define BOOL(name, v) do { key(name); printf("%s", (v) ? "true" : "false"); } while (0)
#define INT(name, v) do { key(name); printf("%lld", (long long)(v)); } while (0)
#define STR(name, v) do { key(name); printf("\"%s\"", (v)); } while (0)

/* A va_list parameter decays to a pointer when va_list is an array type
 * (x86-64 SysV: __va_list_tag[1]); otherwise it keeps its full size. */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wsizeof-array-argument"
static int va_list_param_size(va_list ap)
{
    return (int)sizeof(ap); /* intentionally the parameter's (decayed) size */
}
#pragma GCC diagnostic pop

static int va_list_probe(int n, ...)
{
    va_list ap;
    int r;
    va_start(ap, n);
    r = va_list_param_size(ap);
    va_end(ap);
    return r;
}

static atomic_long atomic_counter;
static long mutex_counter;
static pthread_mutex_t mutex = PTHREAD_MUTEX_INITIALIZER;
static atomic_int threads_started;

static void *worker(void *arg)
{
    (void)arg;
    atomic_fetch_add(&threads_started, 1);
    for (int i = 0; i < ITERS; i++) {
        atomic_fetch_add_explicit(&atomic_counter, 1, memory_order_relaxed);
        pthread_mutex_lock(&mutex);
        mutex_counter++;
        pthread_mutex_unlock(&mutex);
    }
    return NULL;
}

int main(void)
{
    printf("{");

#if defined(__x86_64__)
    STR("target", "x86_64-linux");
#elif defined(__wasm64__)
    STR("target", "wasm64-emscripten");
#elif defined(__wasm32__)
    STR("target", "wasm32-emscripten");
#else
    STR("target", "unknown");
#endif
#if defined(__clang__)
    STR("compiler", "clang " __clang_version__);
#elif defined(__GNUC__)
    STR("compiler", "gcc " __VERSION__);
#endif

    /* Integers and pointers. */
    BOOL("char.signed", CHAR_MIN < 0);
    TYPE("char", char);
    TYPE("short", short);
    TYPE("int", int);
    TYPE("long", long);
    TYPE("long long", long long);
    TYPE("pointer", void *);
    TYPE("function pointer", void (*)(void));
    TYPE("size_t", size_t);
    TYPE("ssize_t", ssize_t);
    TYPE("ptrdiff_t", ptrdiff_t);
    TYPE("intptr_t", intptr_t);
    TYPE("intmax_t", intmax_t);
    TYPE("off_t", off_t);
    TYPE("time_t", time_t);
    TYPE("clock_t", clock_t);
    TYPE_EXTRA("wchar_t", wchar_t, "\"signed\": %s", IS_SIGNED(wchar_t) ? "true" : "false");
    TYPE_EXTRA("wint_t", wint_t, "\"signed\": %s", IS_SIGNED(wint_t) ? "true" : "false");
#ifdef __SIZEOF_INT128__
    BOOL("__SIZEOF_INT128__", 1);
    TYPE("__int128", __int128);
#else
    BOOL("__SIZEOF_INT128__", 0);
#endif

    /* Floating point. */
    TYPE_EXTRA("float", float, "\"mant_dig\": %d", FLT_MANT_DIG);
    TYPE_EXTRA("double", double, "\"mant_dig\": %d", DBL_MANT_DIG);
    TYPE_EXTRA("long double", long double, "\"mant_dig\": %d, \"max_exp\": %d", LDBL_MANT_DIG, LDBL_MAX_EXP);
    INT("FLT_EVAL_METHOD", FLT_EVAL_METHOD);

    /* Alignment and varargs. */
    TYPE("max_align_t", max_align_t);
    {
        int psize = va_list_probe(1, 2);
        TYPE_EXTRA("va_list", va_list, "\"is_array\": %s, \"param_size\": %d",
                   psize != (int)sizeof(va_list) ? "true" : "false", psize);
    }

    /* Threads, signals, non-local jumps. */
    TYPE("pthread_t", pthread_t);
    TYPE("pthread_mutex_t", pthread_mutex_t);
    TYPE("pthread_cond_t", pthread_cond_t);
    TYPE("pthread_attr_t", pthread_attr_t);
    TYPE("pthread_key_t", pthread_key_t);
    TYPE("sig_atomic_t", sig_atomic_t);
    TYPE("sigset_t", sigset_t);
    TYPE("struct sigaction", struct sigaction);
    TYPE("jmp_buf", jmp_buf);
    TYPE("sigjmp_buf", sigjmp_buf);

    /* System structures. */
    TYPE("struct stat", struct stat);
    TYPE("struct timespec", struct timespec);
    TYPE("struct timeval", struct timeval);
    TYPE("struct sockaddr_storage", struct sockaddr_storage);
    TYPE("socklen_t", socklen_t);
    TYPE("fd_set", fd_set);
    INT("FD_SETSIZE", FD_SETSIZE);

    /* Byte order. */
    {
        union { uint32_t u; unsigned char b[4]; } e = { 0x01020304u };
        STR("endianness", e.b[0] == 4 ? "little" : e.b[0] == 1 ? "big" : "mixed");
    }

    /* Lock-free atomics, as the compiler sees them (no libatomic needed). */
    BOOL("atomic.always_lock_free.1", __atomic_always_lock_free(1, 0));
    BOOL("atomic.always_lock_free.2", __atomic_always_lock_free(2, 0));
    BOOL("atomic.always_lock_free.4", __atomic_always_lock_free(4, 0));
    BOOL("atomic.always_lock_free.8", __atomic_always_lock_free(8, 0));
    BOOL("atomic.always_lock_free.16", __atomic_always_lock_free(16, 0));
    INT("ATOMIC_LLONG_LOCK_FREE", ATOMIC_LLONG_LOCK_FREE);
    INT("ATOMIC_POINTER_LOCK_FREE", ATOMIC_POINTER_LOCK_FREE);

    /* Predefined macros. */
#define MACRO(m, defined_) BOOL("macro." m, defined_)
#ifdef __x86_64__
    MACRO("__x86_64__", 1);
#else
    MACRO("__x86_64__", 0);
#endif
#ifdef __wasm__
    MACRO("__wasm__", 1);
#else
    MACRO("__wasm__", 0);
#endif
#ifdef __wasm64__
    MACRO("__wasm64__", 1);
#else
    MACRO("__wasm64__", 0);
#endif
#ifdef __wasm32__
    MACRO("__wasm32__", 1);
#else
    MACRO("__wasm32__", 0);
#endif
#ifdef __LP64__
    MACRO("__LP64__", 1);
#else
    MACRO("__LP64__", 0);
#endif
#ifdef _LP64
    MACRO("_LP64", 1);
#else
    MACRO("_LP64", 0);
#endif
#ifdef __EMSCRIPTEN__
    MACRO("__EMSCRIPTEN__", 1);
#else
    MACRO("__EMSCRIPTEN__", 0);
#endif
#ifdef __linux__
    MACRO("__linux__", 1);
#else
    MACRO("__linux__", 0);
#endif
#ifdef __unix__
    MACRO("__unix__", 1);
#else
    MACRO("__unix__", 0);
#endif
#ifdef __ELF__
    MACRO("__ELF__", 1);
#else
    MACRO("__ELF__", 0);
#endif
#ifdef __GNUC__
    MACRO("__GNUC__", 1);
#else
    MACRO("__GNUC__", 0);
#endif
#ifdef __SIZEOF_POINTER__
    INT("macro.__SIZEOF_POINTER__", __SIZEOF_POINTER__);
#endif
#ifdef __BIGGEST_ALIGNMENT__
    INT("macro.__BIGGEST_ALIGNMENT__", __BIGGEST_ALIGNMENT__);
#endif

    /* Pthreads. */
    {
        pthread_t th[NTHREADS];
        int created = 0, joined = 0, ok;
        for (int i = 0; i < NTHREADS; i++)
            if (pthread_create(&th[i], NULL, worker, NULL) == 0) created++;
        for (int i = 0; i < created; i++)
            if (pthread_join(th[i], NULL) == 0) joined++;
        ok = created == NTHREADS && joined == NTHREADS &&
             atomic_load(&atomic_counter) == (long)NTHREADS * ITERS &&
             mutex_counter == (long)NTHREADS * ITERS;
        key("pthread");
        printf("{\"threads\": %d, \"created\": %d, \"joined\": %d, \"started\": %d, "
               "\"expected\": %ld, \"atomic_counter\": %ld, \"mutex_counter\": %ld, \"ok\": %s}",
               NTHREADS, created, joined, atomic_load(&threads_started), (long)NTHREADS * ITERS,
               (long)atomic_load(&atomic_counter), mutex_counter, ok ? "true" : "false");
    }

    printf("\n}\n");
    fflush(stdout);
    return 0;
}

/* L2 "translated vs native" program: threads and synchronization. Every thread
 * blocks in Windows calls (waits, Sleep, SwitchToThread): our green threads are
 * cooperative and switch only inside them, so no loop spins on a flag without a
 * call. Only main prints, after the joins and in a fixed order, so the output
 * does not depend on scheduling. No addresses, no ids, no times in the output. */
#include <process.h>
#include <stdint.h>
#include <stdio.h>
#include <windows.h>

#define LIMIT 20000 /* ms: a hang fails the run instead of stalling it */

static void must(int ok, const char *what) {
    if (!ok) {
        printf("FATAL: %s (error %lu)\n", what, GetLastError());
        fflush(stdout);
        ExitProcess(99);
    }
}

static HANDLE spawn(LPTHREAD_START_ROUTINE fn, void *arg) {
    HANDLE h = CreateThread(NULL, 0, fn, arg, 0, NULL);
    must(h != NULL, "CreateThread");
    return h;
}

static DWORD join(HANDLE h) {
    DWORD w = WaitForSingleObject(h, LIMIT);
    must(w == WAIT_OBJECT_0, "join");
    DWORD code = 0xdeadbeef;
    must(GetExitCodeThread(h, &code), "GetExitCodeThread");
    CloseHandle(h);
    return code;
}

/* 1. return, ExitThread, _beginthreadex, STILL_ACTIVE */
static DWORD WINAPI ret7(void *p) {
    (void)p;
    return 7;
}
static DWORD WINAPI exit9(void *p) {
    (void)p;
    ExitThread(9);
    return 1;
}
static unsigned __stdcall ret11(void *p) { return (unsigned)(uintptr_t)p + 5; }
static HANDLE g_hold;
static DWORD WINAPI hold_then_13(void *p) {
    (void)p;
    WaitForSingleObject(g_hold, LIMIT);
    return 13;
}

static void basics(void) {
    printf("return: %lu\n", join(spawn(ret7, NULL)));
    printf("ExitThread: %lu\n", join(spawn(exit9, NULL)));
    HANDLE h = (HANDLE)_beginthreadex(NULL, 0, ret11, (void *)(uintptr_t)6, 0, NULL);
    must(h != NULL, "_beginthreadex");
    printf("_beginthreadex: %lu\n", join(h));
    g_hold = CreateEventA(NULL, TRUE, FALSE, NULL);
    h = spawn(hold_then_13, NULL);
    DWORD code = 0;
    GetExitCodeThread(h, &code);
    printf("running: %lu\n", code); /* STILL_ACTIVE */
    SetEvent(g_hold);
    printf("held: %lu\n", join(h));
    CloseHandle(g_hold);
}

/* 2. WaitForMultipleObjects */
static DWORD WINAPI ret_plus_100(void *p) { return 100 + (DWORD)(uintptr_t)p; }
static void multi(void) {
    HANDLE t[3];
    for (int i = 0; i < 3; i++) t[i] = spawn(ret_plus_100, (void *)(uintptr_t)i);
    printf("wait all threads: %lu\n", WaitForMultipleObjects(3, t, TRUE, LIMIT));
    DWORD sum = 0;
    for (int i = 0; i < 3; i++) {
        DWORD c = 0;
        GetExitCodeThread(t[i], &c);
        sum += c;
        CloseHandle(t[i]);
    }
    printf("exit code sum: %lu\n", sum);
    HANDLE e[3];
    for (int i = 0; i < 3; i++) e[i] = CreateEventA(NULL, FALSE, FALSE, NULL);
    printf("wait any, none set: %lu\n", WaitForMultipleObjects(3, e, FALSE, 0));
    SetEvent(e[1]);
    printf("wait any, #1 set: %lu\n", WaitForMultipleObjects(3, e, FALSE, LIMIT));
    printf("wait any again (auto-reset): %lu\n", WaitForMultipleObjects(3, e, FALSE, 0));
    SetEvent(e[0]);
    SetEvent(e[2]);
    printf("wait all, 2 of 3: %lu\n", WaitForMultipleObjects(3, e, TRUE, 0));
    SetEvent(e[1]);
    printf("wait all, 3 of 3: %lu\n", WaitForMultipleObjects(3, e, TRUE, LIMIT));
    printf("wait all consumed them: %lu\n", WaitForMultipleObjects(3, e, FALSE, 0));
    for (int i = 0; i < 3; i++) CloseHandle(e[i]);
}

/* 3. auto-reset events: a ping-pong between main and a worker */
static HANDLE g_ping, g_pong;
static int g_pings;
static DWORD WINAPI ponger(void *p) {
    int n = (int)(uintptr_t)p;
    for (int i = 0; i < n; i++) {
        must(WaitForSingleObject(g_ping, LIMIT) == WAIT_OBJECT_0, "ping");
        g_pings++;
        SetEvent(g_pong);
    }
    return 0;
}
static void autoreset(void) {
    g_ping = CreateEventA(NULL, FALSE, FALSE, NULL);
    g_pong = CreateEventA(NULL, FALSE, FALSE, NULL);
    HANDLE h = spawn(ponger, (void *)(uintptr_t)5);
    for (int i = 0; i < 5; i++) {
        SetEvent(g_ping);
        must(WaitForSingleObject(g_pong, LIMIT) == WAIT_OBJECT_0, "pong");
    }
    join(h);
    printf("pings: %d\n", g_pings);
    printf("ping after the wait: %lu\n", WaitForSingleObject(g_ping, 0));
    CloseHandle(g_ping);
    CloseHandle(g_pong);
}

/* 4. a manual-reset event wakes every waiter and stays set until reset */
static HANDLE g_gate;
static volatile LONG g_ready, g_through;
static DWORD WINAPI gate_waiter(void *p) {
    (void)p;
    InterlockedIncrement(&g_ready);
    must(WaitForSingleObject(g_gate, LIMIT) == WAIT_OBJECT_0, "gate");
    InterlockedIncrement(&g_through);
    return 0;
}
static void manualreset(void) {
    g_gate = CreateEventA(NULL, TRUE, FALSE, NULL);
    HANDLE t[3];
    for (int i = 0; i < 3; i++) t[i] = spawn(gate_waiter, NULL);
    while (g_ready != 3) Sleep(1);
    printf("through before set: %ld\n", g_through);
    SetEvent(g_gate);
    printf("join: %lu\n", WaitForMultipleObjects(3, t, TRUE, LIMIT));
    for (int i = 0; i < 3; i++) CloseHandle(t[i]);
    printf("through: %ld\n", g_through);
    printf("still set: %lu\n", WaitForSingleObject(g_gate, 0));
    ResetEvent(g_gate);
    printf("after reset: %lu\n", WaitForSingleObject(g_gate, 0));
    CloseHandle(g_gate);
}

/* 5. critical section: the counter is read, a Sleep(0) lets another thread run
 * (and meet the lock), and it is written back. */
static CRITICAL_SECTION g_cs;
static int g_cs_count;
static DWORD WINAPI cs_worker(void *p) {
    (void)p;
    for (int i = 0; i < 100; i++) {
        EnterCriticalSection(&g_cs);
        int v = g_cs_count;
        if (i % 7 == 0) Sleep(0);
        g_cs_count = v + 1;
        LeaveCriticalSection(&g_cs);
        if (i % 11 == 0) Sleep(0);
    }
    return 0;
}
static volatile LONG g_try_result = -1;
static DWORD WINAPI cs_try(void *p) {
    (void)p;
    g_try_result = TryEnterCriticalSection(&g_cs) ? 1 : 0;
    if (g_try_result) LeaveCriticalSection(&g_cs);
    return 0;
}
static void critsec(void) {
    InitializeCriticalSection(&g_cs);
    HANDLE t[4];
    for (int i = 0; i < 4; i++) t[i] = spawn(cs_worker, NULL);
    printf("join: %lu\n", WaitForMultipleObjects(4, t, TRUE, LIMIT));
    for (int i = 0; i < 4; i++) CloseHandle(t[i]);
    printf("count: %d\n", g_cs_count);
    EnterCriticalSection(&g_cs);
    EnterCriticalSection(&g_cs); /* recursive */
    join(spawn(cs_try, NULL));
    printf("try from another thread, held twice: %ld\n", g_try_result);
    LeaveCriticalSection(&g_cs);
    join(spawn(cs_try, NULL));
    printf("try, held once: %ld\n", g_try_result);
    LeaveCriticalSection(&g_cs);
    join(spawn(cs_try, NULL));
    printf("try, free: %ld\n", g_try_result);
    DeleteCriticalSection(&g_cs);
}

/* 6. mutex: mutual exclusion, recursion, ownership */
static HANDLE g_mutex;
static int g_mx_count;
static volatile LONG g_mx_result;
static DWORD WINAPI mx_worker(void *p) {
    (void)p;
    for (int i = 0; i < 50; i++) {
        must(WaitForSingleObject(g_mutex, LIMIT) == WAIT_OBJECT_0, "mutex");
        int v = g_mx_count;
        if (i % 5 == 0) Sleep(0);
        g_mx_count = v + 1;
        ReleaseMutex(g_mutex);
    }
    return 0;
}
static DWORD WINAPI mx_probe(void *p) {
    (void)p;
    g_mx_result = (LONG)WaitForSingleObject(g_mutex, 0);
    if (g_mx_result == WAIT_OBJECT_0) ReleaseMutex(g_mutex);
    return 0;
}
static DWORD WINAPI mx_block(void *p) {
    (void)p;
    g_mx_result = (LONG)WaitForSingleObject(g_mutex, LIMIT);
    if (g_mx_result == WAIT_OBJECT_0) ReleaseMutex(g_mutex);
    return 0;
}
static void mutexes(void) {
    g_mutex = CreateMutexA(NULL, FALSE, NULL);
    HANDLE t[3];
    for (int i = 0; i < 3; i++) t[i] = spawn(mx_worker, NULL);
    printf("join: %lu\n", WaitForMultipleObjects(3, t, TRUE, LIMIT));
    for (int i = 0; i < 3; i++) CloseHandle(t[i]);
    printf("count: %d\n", g_mx_count);
    WaitForSingleObject(g_mutex, 0);
    WaitForSingleObject(g_mutex, 0); /* recursive */
    join(spawn(mx_probe, NULL));
    printf("probe, owned twice: %ld\n", g_mx_result);
    printf("release 1: %d\n", ReleaseMutex(g_mutex));
    join(spawn(mx_probe, NULL));
    printf("probe, owned once: %ld\n", g_mx_result);
    /* a thread blocked on it gets it once main lets go */
    HANDLE h = spawn(mx_block, NULL);
    Sleep(0);
    printf("release 2: %d\n", ReleaseMutex(g_mutex));
    join(h);
    printf("blocked thread got it: %ld\n", g_mx_result);
    SetLastError(0);
    int r = ReleaseMutex(g_mutex);
    printf("release unowned: %d error %lu\n", r, GetLastError()); /* ERROR_NOT_OWNER */
    CloseHandle(g_mutex);
}

/* 7. semaphore */
static HANDLE g_sem;
static volatile LONG g_sem_got;
static DWORD WINAPI sem_waiter(void *p) {
    (void)p;
    must(WaitForSingleObject(g_sem, LIMIT) == WAIT_OBJECT_0, "semaphore");
    InterlockedIncrement(&g_sem_got);
    return 0;
}
static void semaphores(void) {
    g_sem = CreateSemaphoreA(NULL, 0, 3, NULL);
    HANDLE t[3];
    for (int i = 0; i < 3; i++) t[i] = spawn(sem_waiter, NULL);
    Sleep(0);
    LONG prev = -1;
    int r = ReleaseSemaphore(g_sem, 3, &prev);
    printf("release 3: %d previous %ld\n", r, prev);
    printf("join: %lu\n", WaitForMultipleObjects(3, t, TRUE, LIMIT));
    for (int i = 0; i < 3; i++) CloseHandle(t[i]);
    printf("got: %ld\n", g_sem_got);
    printf("empty: %lu\n", WaitForSingleObject(g_sem, 0));
    prev = -1;
    r = ReleaseSemaphore(g_sem, 1, &prev);
    printf("release 1: %d previous %ld\n", r, prev);
    SetLastError(0);
    r = ReleaseSemaphore(g_sem, 3, NULL);
    printf("release 3 over the maximum: %d error %lu\n", r, GetLastError()); /* ERROR_TOO_MANY_POSTS */
    printf("take the one: %lu\n", WaitForSingleObject(g_sem, 0));
    CloseHandle(g_sem);
}

/* 8. TLS */
static DWORD g_tls;
static volatile LONG g_tls_ok;
static DWORD WINAPI tls_worker(void *p) {
    uintptr_t mine = (uintptr_t)p * 10 + 1;
    int ok = TlsGetValue(g_tls) == NULL; /* a new thread starts with NULL */
    TlsSetValue(g_tls, (void *)mine);
    for (int i = 0; i < 4; i++) {
        Sleep(0);
        if ((uintptr_t)TlsGetValue(g_tls) != mine) ok = 0;
    }
    if (ok) InterlockedIncrement(&g_tls_ok);
    return (DWORD)mine;
}
static void tls(void) {
    g_tls = TlsAlloc();
    must(g_tls != TLS_OUT_OF_INDEXES, "TlsAlloc");
    TlsSetValue(g_tls, (void *)(uintptr_t)12345);
    HANDLE t[4];
    for (int i = 0; i < 4; i++) t[i] = spawn(tls_worker, (void *)(uintptr_t)i);
    printf("join: %lu\n", WaitForMultipleObjects(4, t, TRUE, LIMIT));
    DWORD sum = 0;
    for (int i = 0; i < 4; i++) {
        DWORD c = 0;
        GetExitCodeThread(t[i], &c);
        sum += c;
        CloseHandle(t[i]);
    }
    printf("workers that kept their own value: %ld, values %lu\n", g_tls_ok, sum);
    printf("main's value: %lu\n", (unsigned long)(uintptr_t)TlsGetValue(g_tls));
    printf("TlsFree: %d\n", TlsFree(g_tls));
}

/* 9. Interlocked operations from several threads */
static volatile LONG g_ilong;
static volatile LONG64 g_i64;
static volatile LONG g_max;
static DWORD WINAPI il_worker(void *p) {
    LONG id = (LONG)(uintptr_t)p;
    for (int i = 0; i < 250; i++) {
        InterlockedIncrement(&g_ilong);
        InterlockedExchangeAdd(&g_ilong, 2);
        InterlockedAdd64(&g_i64, 3);
        LONG seen = g_max, want = id * 1000 + i;
        while (want > seen) {
            LONG old = InterlockedCompareExchange(&g_max, want, seen);
            if (old == seen) break;
            seen = old;
        }
        if (i % 17 == 0) Sleep(0);
    }
    return 0;
}
static void interlocked(void) {
    HANDLE t[4];
    for (int i = 0; i < 4; i++) t[i] = spawn(il_worker, (void *)(uintptr_t)i);
    printf("join: %lu\n", WaitForMultipleObjects(4, t, TRUE, LIMIT));
    for (int i = 0; i < 4; i++) CloseHandle(t[i]);
    printf("long: %ld  int64: %lld  max: %ld\n", g_ilong, (long long)g_i64, g_max);
    volatile LONG v = 5;
    LONG old;
    old = InterlockedExchange(&v, 9);
    printf("exchange: %ld now %ld\n", old, v);
    old = InterlockedCompareExchange(&v, 11, 9);
    printf("cas hit: %ld now %ld\n", old, v);
    old = InterlockedCompareExchange(&v, 13, 9);
    printf("cas miss: %ld now %ld\n", old, v);
    old = InterlockedExchangeAdd(&v, -4);
    printf("exchange add: %ld now %ld\n", old, v);
    old = InterlockedOr(&v, 0x30);
    printf("or: %ld now %ld\n", old, v);
    old = InterlockedAnd(&v, 0x3c);
    printf("and: %ld now %ld\n", old, v);
    old = InterlockedDecrement(&v);
    printf("decrement: %ld now %ld\n", old, v);
    volatile LONG64 w = 1ll << 40;
    LONG64 old64 = InterlockedExchange64(&w, 77);
    printf("64-bit exchange: %lld now %lld\n", (long long)old64, (long long)w);
    old64 = InterlockedCompareExchange64(&w, 1ll << 33, 77);
    printf("64-bit cas: %lld now %lld\n", (long long)old64, (long long)w);
    void *volatile ptr = NULL;
    void *was = InterlockedCompareExchangePointer(&ptr, (void *)&v, NULL);
    printf("pointer cas: %d\n", was == NULL && ptr == (void *)&v);
}

/* 10. Sleep(0) and SwitchToThread: A waits on a flag only through SwitchToThread, and B
 * sets it after some Sleep(0)s. */
static volatile LONG g_flag, g_spins_done;
static DWORD WINAPI yielder(void *p) {
    (void)p;
    while (!g_flag) SwitchToThread();
    InterlockedIncrement(&g_spins_done);
    return 0;
}
static DWORD WINAPI flagger(void *p) {
    (void)p;
    for (int i = 0; i < 5; i++) Sleep(0);
    g_flag = 1;
    return 0;
}
static void yields(void) {
    HANDLE a = spawn(yielder, NULL), b = spawn(flagger, NULL);
    join(b);
    join(a);
    printf("yielder finished: %ld\n", g_spins_done);
    Sleep(0);
    SwitchToThread();
    printf("main yielded\n");
}

/* 11. TerminateThread of a thread in Sleep(INFINITE) */
static HANDLE g_started;
static DWORD WINAPI sleeper(void *p) {
    (void)p;
    SetEvent(g_started);
    Sleep(INFINITE);
    return 1;
}
static void terminate(void) {
    g_started = CreateEventA(NULL, TRUE, FALSE, NULL);
    HANDLE h = spawn(sleeper, NULL);
    must(WaitForSingleObject(g_started, LIMIT) == WAIT_OBJECT_0, "started");
    DWORD code = 0;
    GetExitCodeThread(h, &code);
    printf("sleeping: %lu\n", code); /* STILL_ACTIVE */
    printf("TerminateThread: %d\n", TerminateThread(h, 77));
    printf("join: %lu\n", WaitForSingleObject(h, LIMIT));
    GetExitCodeThread(h, &code);
    printf("exit code: %lu\n", code);
    CloseHandle(h);
    CloseHandle(g_started);
}

/* 12. main returns while a worker waits on an event nobody sets */
static HANDLE g_never, g_waiting;
static DWORD WINAPI orphan(void *p) {
    (void)p;
    SetEvent(g_waiting);
    WaitForSingleObject(g_never, INFINITE);
    printf("FATAL: the orphan woke up\n");
    return 5;
}

int main(void) {
    basics();
    multi();
    autoreset();
    manualreset();
    critsec();
    mutexes();
    semaphores();
    tls();
    interlocked();
    yields();
    terminate();
    g_never = CreateEventA(NULL, TRUE, FALSE, NULL);
    g_waiting = CreateEventA(NULL, TRUE, FALSE, NULL);
    spawn(orphan, NULL);
    must(WaitForSingleObject(g_waiting, LIMIT) == WAIT_OBJECT_0, "orphan started");
    printf("main returns\n");
    return 3;
}

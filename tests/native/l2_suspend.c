/* L2 "translated vs native" program: suspension, thread contexts and APCs.
 * Every thread blocks in Windows calls (our green threads are cooperative and
 * switch only inside them), and only main prints, after the joins, so the
 * output does not depend on scheduling. No addresses, ids or times in it. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>
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
    must(WaitForSingleObject(h, LIMIT) == WAIT_OBJECT_0, "join");
    DWORD code = 0xdeadbeef;
    must(GetExitCodeThread(h, &code), "GetExitCodeThread");
    CloseHandle(h);
    return code;
}

/* A worker that tells main it is about to wait, waits for `go`, and returns 21. */
static HANDLE g_started, g_go;
static DWORD WINAPI waiter(void *p) {
    (void)p;
    SetEvent(g_started);
    must(WaitForSingleObject(g_go, LIMIT) == WAIT_OBJECT_0, "go");
    return 21;
}

/* 1. Suspend and resume counts, and a suspended thread whose wait is satisfied
 * stays stopped until the count is back to zero. */
static void counts(void) {
    g_started = CreateEventA(NULL, TRUE, FALSE, NULL);
    g_go = CreateEventA(NULL, TRUE, FALSE, NULL);
    HANDLE h = spawn(waiter, NULL);
    must(WaitForSingleObject(g_started, LIMIT) == WAIT_OBJECT_0, "started");
    printf("suspend: %lu\n", SuspendThread(h));
    printf("suspend: %lu\n", SuspendThread(h));
    SetEvent(g_go);
    printf("suspended thread finished: %lu\n", WaitForSingleObject(h, 100)); /* WAIT_TIMEOUT */
    printf("resume: %lu\n", ResumeThread(h));
    printf("still one left: %lu\n", WaitForSingleObject(h, 100));
    printf("resume: %lu\n", ResumeThread(h));
    printf("exit code: %lu\n", join(h));
    CloseHandle(g_started);
    CloseHandle(g_go);
}

/* 2. A thread suspends itself and main resumes it. Main cannot know whether the
 * worker has reached its SuspendThread, so it resumes until the thread ends:
 * a resume of a thread with count 0 does nothing. The first resume that finds
 * a count to take down returns 1. */
static volatile LONG g_before_suspend;
static DWORD WINAPI self_suspender(void *p) {
    (void)p;
    g_before_suspend = 1;
    SuspendThread(GetCurrentThread());
    return 31;
}
static void self_suspend(void) {
    HANDLE h = spawn(self_suspender, NULL);
    DWORD first = 0;
    int tries = 0;
    while (WaitForSingleObject(h, 20) == WAIT_TIMEOUT) {
        DWORD r = ResumeThread(h);
        if (r != 0 && first == 0) first = r;
        must(++tries < 1000, "self-suspended thread never ended");
    }
    printf("first resume that took a count down returned: %lu\n", first);
    printf("exit code: %lu\n", join(h));
    printf("worker ran: %ld\n", g_before_suspend);
}

/* 3. GetThreadContext and SetThreadContext on a suspended worker that is blocked in
 * a wait. Only facts the same on every system are printed. */
static void contexts(void) {
    g_started = CreateEventA(NULL, TRUE, FALSE, NULL);
    g_go = CreateEventA(NULL, TRUE, FALSE, NULL);
    HANDLE h = spawn(waiter, NULL);
    must(WaitForSingleObject(g_started, LIMIT) == WAIT_OBJECT_0, "started");
    SuspendThread(h);
    CONTEXT a;
    memset(&a, 0, sizeof a);
    a.ContextFlags = CONTEXT_CONTROL | CONTEXT_INTEGER;
    printf("GetThreadContext: %d\n", GetThreadContext(h, &a));
    printf("flags kept: %d\n", (a.ContextFlags & (CONTEXT_CONTROL | CONTEXT_INTEGER)) == (CONTEXT_CONTROL | CONTEXT_INTEGER));
    printf("rip and rsp set: %d\n", a.Rip != 0 && a.Rsp != 0);
    CONTEXT b = a;
    b.R14 = 0x1122334455667788ull; /* a register the worker's own code keeps across the wait */
    printf("SetThreadContext: %d\n", SetThreadContext(h, &b));
    CONTEXT c;
    memset(&c, 0, sizeof c);
    c.ContextFlags = CONTEXT_CONTROL | CONTEXT_INTEGER;
    printf("GetThreadContext again: %d\n", GetThreadContext(h, &c));
    printf("r14 round trip: %d\n", c.R14 == 0x1122334455667788ull);
    printf("rip and rsp unchanged: %d\n", c.Rip == a.Rip && c.Rsp == a.Rsp);
    printf("SetThreadContext, original: %d\n", SetThreadContext(h, &a));
    memset(&c, 0, sizeof c);
    c.ContextFlags = CONTEXT_CONTROL | CONTEXT_INTEGER;
    GetThreadContext(h, &c);
    printf("r14 restored: %d\n", c.R14 == a.R14);
    SetEvent(g_go);
    printf("resume: %lu\n", ResumeThread(h));
    printf("exit code: %lu\n", join(h));
    CloseHandle(g_started);
    CloseHandle(g_go);
}

/* 4. QueueUserAPC to a thread in an alertable SleepEx, in an alertable
 * WaitForSingleObjectEx, and to one that is in a plain wait. */
static volatile LONG g_apc_count;
static volatile LONG g_apc_order;
static VOID CALLBACK apc(ULONG_PTR param) {
    InterlockedIncrement(&g_apc_count);
    g_apc_order = g_apc_order * 10 + (LONG)param;
}

static HANDLE g_ev;
static volatile LONG g_result, g_count_at_wake;

static DWORD WINAPI alert_sleep(void *p) {
    (void)p;
    SetEvent(g_started);
    /* main queues its APCs one at a time, so a wake-up may carry one APC or several */
    LONG r;
    do r = (LONG)SleepEx(INFINITE, TRUE);
    while (g_apc_count < 3);
    g_result = r;
    return 41;
}
static DWORD WINAPI alert_wait(void *p) {
    (void)p;
    SetEvent(g_started);
    g_result = (LONG)WaitForSingleObjectEx(g_ev, INFINITE, TRUE);
    return 42;
}
static DWORD WINAPI plain_wait(void *p) {
    (void)p;
    SetEvent(g_started);
    LONG w = (LONG)WaitForSingleObject(g_ev, LIMIT); /* not alertable */
    g_count_at_wake = g_apc_count;
    LONG s = (LONG)SleepEx(0, TRUE); /* now it may run */
    g_result = w * 1000 + s;
    return 43;
}

static void apcs(void) {
    g_started = CreateEventA(NULL, TRUE, FALSE, NULL);
    g_ev = CreateEventA(NULL, FALSE, FALSE, NULL);

    g_apc_count = 0;
    g_apc_order = 0;
    HANDLE h = spawn(alert_sleep, NULL);
    must(WaitForSingleObject(g_started, LIMIT) == WAIT_OBJECT_0, "started");
    printf("queue 1: %d\n", QueueUserAPC(apc, h, 1) != 0);
    printf("queue 2: %d\n", QueueUserAPC(apc, h, 2) != 0);
    printf("queue 3: %d\n", QueueUserAPC(apc, h, 3) != 0);
    printf("exit code: %lu\n", join(h));
    printf("SleepEx returned: %ld (WAIT_IO_COMPLETION is %lu)\n", g_result, (unsigned long)WAIT_IO_COMPLETION);
    printf("APCs run: %ld in order %ld\n", g_apc_count, g_apc_order);

    ResetEvent(g_started);
    g_apc_count = 0;
    g_apc_order = 0;
    h = spawn(alert_wait, NULL);
    must(WaitForSingleObject(g_started, LIMIT) == WAIT_OBJECT_0, "started");
    printf("queue 4: %d\n", QueueUserAPC(apc, h, 4) != 0);
    printf("exit code: %lu\n", join(h));
    printf("WaitForSingleObjectEx returned: %ld\n", g_result);
    printf("APCs run: %ld in order %ld\n", g_apc_count, g_apc_order);
    printf("the event was not taken: %lu\n", WaitForSingleObject(g_ev, 0));

    ResetEvent(g_started);
    g_apc_count = 0;
    g_apc_order = 0;
    h = spawn(plain_wait, NULL);
    must(WaitForSingleObject(g_started, LIMIT) == WAIT_OBJECT_0, "started");
    printf("queue 5: %d\n", QueueUserAPC(apc, h, 5) != 0);
    SetEvent(g_ev);
    printf("exit code: %lu\n", join(h));
    printf("APCs run before the alertable call: %ld\n", g_count_at_wake);
    printf("wait * 1000 + SleepEx: %ld\n", g_result);
    printf("APCs run: %ld in order %ld\n", g_apc_count, g_apc_order);
    CloseHandle(g_started);
    CloseHandle(g_ev);
}

int main(void) {
    counts();
    self_suspend();
    contexts();
    apcs();
    printf("done\n");
    return 0;
}

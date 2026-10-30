/* L2 "translated vs native" program: CRT startup and runtime. No floating
 * point, deterministic, no addresses or environment in the output. */
#include <errno.h>
#include <limits.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static void ctor_a(void) __attribute__((constructor(101)));
static void ctor_b(void) __attribute__((constructor(102)));
static void ctor_c(void) __attribute__((constructor));
static int ctor_count;

static void ctor_a(void) { printf("ctor a (%d)\n", ctor_count++); }
static void ctor_b(void) { printf("ctor b (%d)\n", ctor_count++); }
static void ctor_c(void) { printf("ctor c (%d)\n", ctor_count++); }

static void exit_1(void) { puts("atexit 1"); }
static void exit_2(void) { puts("atexit 2"); }
static void exit_3(void) { printf("atexit 3 (ctors=%d)\n", ctor_count); }

static uint32_t checksum(const unsigned char *p, size_t n) {
    uint32_t h = 2166136261u;
    for (size_t i = 0; i < n; i++) h = (h ^ p[i]) * 16777619u;
    return h;
}

static void heap(void) {
    unsigned char *a = malloc(100);
    unsigned char *b = calloc(50, 4);
    for (int i = 0; i < 100; i++) a[i] = (unsigned char)(i * 3 + 1);
    uint32_t zero = 0;
    for (int i = 0; i < 200; i++) zero |= b[i];
    printf("malloc sum=%08x calloc zero=%u\n", checksum(a, 100), zero);
    a = realloc(a, 5000);
    for (int i = 100; i < 5000; i++) a[i] = (unsigned char)(i ^ (i >> 8));
    printf("realloc grow sum=%08x first100=%08x\n", checksum(a, 5000), checksum(a, 100));
    a = realloc(a, 40);
    printf("realloc shrink sum=%08x\n", checksum(a, 40));
    char *blocks[20];
    uint32_t all = 0;
    for (int i = 0; i < 20; i++) {
        size_t n = (size_t)(i * 37 + 1);
        blocks[i] = malloc(n);
        memset(blocks[i], i + 1, n);
        all = all * 31u + checksum((unsigned char *)blocks[i], n);
    }
    for (int i = 0; i < 20; i += 2) free(blocks[i]);
    for (int i = 0; i < 20; i += 2) {
        size_t n = (size_t)(i * 11 + 5);
        blocks[i] = malloc(n);
        memset(blocks[i], 0xA0 + i, n);
    }
    for (int i = 0; i < 20; i++) {
        size_t n = (i & 1) ? (size_t)(i * 37 + 1) : (size_t)(i * 11 + 5);
        all = all * 31u + checksum((unsigned char *)blocks[i], n);
        free(blocks[i]);
    }
    printf("blocks sum=%08x\n", all);
    printf("aligned16: %d\n", (int)(((uintptr_t)a & 15) == 0 && ((uintptr_t)b & 15) == 0));
    free(a);
    free(b);
    free(NULL);
}

struct rec {
    int key;
    char name[8];
};

static int cmp_int(const void *x, const void *y) {
    int a = *(const int *)x, b = *(const int *)y;
    return (a > b) - (a < b);
}

static int cmp_rec_desc(const void *x, const void *y) {
    const struct rec *a = x, *b = y;
    return (b->key > a->key) - (b->key < a->key);
}

static void sorting(void) {
    int v[64];
    uint32_t s = 12345;
    for (int i = 0; i < 64; i++) {
        s = s * 1103515245u + 12345u;
        v[i] = (int)((s >> 8) % 100000) * 64 + i; /* distinct keys */
    }
    qsort(v, 64, sizeof v[0], cmp_int);
    int ok = 1;
    for (int i = 1; i < 64; i++) ok &= v[i - 1] < v[i];
    printf("qsort ints ok=%d first=%d mid=%d last=%d sum=%08x\n", ok, v[0], v[32], v[63],
           checksum((unsigned char *)v, sizeof v));
    struct rec r[10];
    for (int i = 0; i < 10; i++) {
        r[i].key = (i * 7 + 3) % 10 * 100 + i;
        snprintf(r[i].name, sizeof r[i].name, "n%d", i);
    }
    qsort(r, 10, sizeof r[0], cmp_rec_desc);
    for (int i = 0; i < 10; i++) printf("%s:%d%c", r[i].name, r[i].key, i == 9 ? '\n' : ' ');
}

static void strings(void) {
    char buf[64];
    printf("strlen: %d %d\n", (int)strlen(""), (int)strlen("hello, world"));
    printf("strcmp: %d %d %d\n", strcmp("abc", "abc") == 0, strcmp("abc", "abd") < 0, strcmp("b", "a") > 0);
    printf("strncmp: %d %d\n", strncmp("abcdef", "abcxyz", 3), strncmp("abcdef", "abcxyz", 4) < 0);
    const char *t = "find the needle in the haystack";
    printf("strchr: %d %d %d\n", (int)(strchr(t, 'n') - t), (int)(strrchr(t, 'n') - t), strchr(t, 'Z') == NULL);
    memset(buf, 'x', sizeof buf);
    memcpy(buf, "0123456789abcdef", 16);
    buf[63] = 0;
    memmove(buf + 4, buf, 12); /* overlap, forward */
    buf[16] = 0;
    printf("memmove fwd: %s\n", buf);
    memcpy(buf, "0123456789abcdef", 16);
    memmove(buf, buf + 5, 11); /* overlap, backward */
    buf[11] = 0;
    printf("memmove back: %s\n", buf);
    memset(buf, 0, sizeof buf);
    memset(buf + 1, 'q', 5);
    printf("memset: %d %s %d\n", buf[0], buf + 1, buf[6]);
    strcpy(buf, "abc");
    strcat(buf, "def");
    printf("strcpy/strcat: %s %d\n", buf, memcmp(buf, "abcdeg", 6) < 0);
}

static void formats(void) {
    printf("[%d] [%i] [%u] [%x] [%X] [%o]\n", -42, 42, 4000000000u, 48879, 48879, 511);
    printf("[%c] [%s] [%5d] [%-5d] [%05d] [%+d]\n", 'Z', "str", 42, 42, 42, 42);
    printf("[%8s] [%-8s] [%.3s] [%8.3s]\n", "right", "left", "truncate", "abcdef");
    printf("[%#x] [%#X] [%#o] [%08x] [%-8x]\n", 255, 255, 8, 0xBEEF, 0xBEEF);
    printf("[%lld] [%llu] [%llx] [%lld]\n", -9000000000000000000ll, 18000000000000000000ull, 0xDEADBEEFCAFEull,
           (long long)INT64_MIN);
    printf("[%ld] [%lu] [%hd] [%5.3d]\n", -123456l, 4000000000ul, (short)-5, 7);
    printf("[%*d] [%-*d] [%%] [100%%]\n", 6, 12, 6, 12);
    printf("[%d%%] [%c%c%c]\n", 50, 'a', 'b', 'c');
    puts("puts line");
    putchar('p');
    putchar('c');
    putchar('\n');
    fputs("fputs no newline, ", stdout);
    fputc('!', stdout);
    fputc('\n', stdout);
    char out[40];
    int n = snprintf(out, sizeof out, "%s-%04d-%X", "snp", 77, 0xABC);
    printf("snprintf: %d %s\n", n, out);
}

static void errors(void) {
    char *end;
    errno = 0;
    long v = strtol("99999999999999999999", &end, 10);
    printf("strtol ovf: %ld errno=%d end=%d\n", v, errno, (int)*end);
    errno = 0;
    v = strtol("-0x7f xyz", &end, 16);
    printf("strtol hex: %ld errno=%d rest=[%s]\n", v, errno, end);
    errno = 0;
    unsigned long long u = strtoull("18446744073709551616", &end, 10);
    printf("strtoull ovf: %llu errno=%d\n", u, errno);
    printf("atoi: %d %d %d\n", atoi("  123abc"), atoi("-77"), atoi("zzz"));
    errno = 0;
    /* ERANGE and EDOM texts differ between Wine and msvcrt.dll, so not printed */
    printf("strerror EEXIST: %s\n", strerror(EEXIST));
    printf("strerror EACCES: %s\n", strerror(EACCES));
    printf("strerror ENOENT: %s\n", strerror(ENOENT));
    printf("strerror EINVAL: %s\n", strerror(EINVAL));
    errno = ENOMEM;
    printf("errno=%d abs=%d div=%d,%d\n", errno, abs(-9), div(-17, 5).quot, div(-17, 5).rem);
    FILE *f = fopen("this\\file\\does\\not\\exist.txt", "rb");
    printf("fopen missing: null=%d errno=%d\n", f == NULL, errno);
}

int main(void) {
    printf("main start (ctors=%d)\n", ctor_count);
    atexit(exit_1);
    atexit(exit_2);
    atexit(exit_3);
    heap();
    sorting();
    strings();
    formats();
    errors();
    puts("main end");
    return 37;
}

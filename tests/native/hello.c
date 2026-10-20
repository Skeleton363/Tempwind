/* Seed of the L2 "translated vs native" tests. Deterministic, no input. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static uint32_t crc32_of(const unsigned char *p, size_t n) {
    uint32_t crc = 0xFFFFFFFFu;
    for (size_t i = 0; i < n; i++) {
        crc ^= p[i];
        for (int k = 0; k < 8; k++)
            crc = (crc >> 1) ^ (0xEDB88320u & (0u - (crc & 1u)));
    }
    return ~crc;
}

static int cmp_i32(const void *a, const void *b) {
    int x = *(const int *)a, y = *(const int *)b;
    return (x > y) - (x < y);
}

/* volatile defeats constant folding so the code really executes */
static volatile int64_t vs64 = INT64_MIN;
static volatile int64_t vm1 = -1;
static volatile uint64_t vu64 = 0xFFFFFFFFFFFFFFFFull;
static volatile int64_t va = 0x123456789ABCDEFll;
static volatile int64_t vb = -7;
static volatile int vsh = 13;

int main(void) {
    printf("sizeof(void*)=%u\n", (unsigned)sizeof(void *));
    printf("sizeof(long)=%u\n", (unsigned)sizeof(long));
    printf("sizeof(long long)=%u\n", (unsigned)sizeof(long long));
    printf("sizeof(size_t)=%u\n", (unsigned)sizeof(size_t));
    printf("sizeof(wchar_t)=%u\n", (unsigned)sizeof(wchar_t));

    printf("mul=%llu\n", (unsigned long long)((uint64_t)va * (uint64_t)va));
    printf("smul=%lld\n", (long long)(va * 3));
    printf("div=%lld\n", (long long)(va / vb));
    printf("rem=%lld\n", (long long)(va % vb));
    printf("udiv=%llu\n", (unsigned long long)(vu64 / 1000003u));
    printf("urem=%llu\n", (unsigned long long)(vu64 % 1000003u));
    printf("shl=%llu\n", (unsigned long long)((uint64_t)va << vsh));
    printf("shr=%llu\n", (unsigned long long)((uint64_t)va >> vsh));
    printf("sar=%lld\n", (long long)(vb >> 1));
    printf("sar_min=%lld\n", (long long)(vs64 >> 63));
    printf("min_neg=%lld\n", (long long)(0 - (uint64_t)vs64));
    printf("min_div_pos=%lld\n", (long long)(vs64 / 3));
    printf("min_rem_neg1=%lld\n", (long long)(vs64 % 7));
    printf("max_plus1=%llu\n", (unsigned long long)(vu64 + 1u));
    printf("uint_wrap=%u\n", (unsigned)(vu64 * 3u));
    (void)vm1;

    const char *msg = "The quick brown fox jumps over the lazy dog";
    printf("crc32=%08X\n", (unsigned)crc32_of((const unsigned char *)msg, strlen(msg)));

    int arr[10] = {42, -5, 17, 0, 99, -123, 8, 8, 2147483647, -2147483647 - 1};
    qsort(arr, 10, sizeof arr[0], cmp_i32);
    printf("qsort=");
    for (int i = 0; i < 10; i++) printf("%s%d", i ? "," : "", arr[i]);
    printf("\n");

    printf("double=%.6f %.3f %10.4f %g\n", 3.14159265358979, 12345.6789, -2.5, 0.1);
    printf("double2=%.15g %.0f\n", 1.0 / 3.0, 1e15);

    printf("OK\n");
    return 0;
}

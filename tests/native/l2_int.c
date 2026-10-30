/* L2 "translated vs native" program: integer arithmetic. No floating point,
 * deterministic, no addresses or environment in the output. */
#include <stdint.h>
#include <stdio.h>
#include <string.h>

typedef unsigned __int128 u128;
typedef __int128 s128;

/* volatile keeps the compiler from folding the computations away */
static volatile int8_t v8 = -100;
static volatile uint8_t u8 = 200;
static volatile int16_t v16 = -30000;
static volatile uint16_t u16 = 60000;
static volatile int32_t v32 = -2000000000;
static volatile uint32_t u32 = 4000000000u;
static volatile int64_t v64 = -9000000000000000000ll;
static volatile uint64_t u64 = 18000000000000000000ull;
static volatile int sh = 7;

static void hex128(const char *name, u128 v) {
    printf("%s=%016llx%016llx\n", name, (unsigned long long)(v >> 64), (unsigned long long)v);
}

static void arith(void) {
    int8_t a8 = v8, b8 = 57;
    uint8_t c8 = u8, d8 = 77;
    printf("i8: %d %d %d %d %d\n", a8 + b8, a8 - b8, (int8_t)(a8 * b8), a8 / 7, a8 % 7);
    printf("u8: %u %u %u %u %u\n", c8 + d8, (uint8_t)(c8 - d8 - d8), (uint8_t)(c8 * d8), c8 / 7, c8 % 7);
    int16_t a16 = v16, b16 = 1234;
    uint16_t c16 = u16, d16 = 4321;
    printf("i16: %d %d %d %d %d\n", a16 + b16, a16 - b16, (int16_t)(a16 * b16), a16 / b16, a16 % b16);
    printf("u16: %u %u %u %u %u\n", c16 + d16, (uint16_t)(d16 - c16), (uint16_t)(c16 * d16), c16 / d16, c16 % d16);
    int32_t a32 = v32, b32 = 123456789;
    uint32_t c32 = u32, d32 = 987654321u;
    printf("i32: %d %d %d %d %d\n", (int32_t)((uint32_t)a32 + (uint32_t)b32), (int32_t)((uint32_t)a32 - (uint32_t)b32),
           (int32_t)((uint32_t)a32 * (uint32_t)b32), a32 / b32, a32 % b32);
    printf("u32: %u %u %u %u %u\n", c32 + d32, d32 - c32, c32 * d32, c32 / d32, c32 % d32);
    int64_t a64 = v64, b64 = 1234567890123ll;
    uint64_t c64 = u64, d64 = 98765432109876ull;
    printf("i64: %lld %lld %lld %lld %lld\n", (long long)(a64 / 3 + b64), (long long)(a64 / 3 - b64),
           (long long)((int64_t)((uint64_t)a64 * (uint64_t)b64)), (long long)(a64 / b64), (long long)(a64 % b64));
    printf("u64: %llu %llu %llu %llu %llu\n", (unsigned long long)(c64 / 2 + d64), (unsigned long long)(d64 - c64),
           (unsigned long long)(c64 * d64), (unsigned long long)(c64 / d64), (unsigned long long)(c64 % d64));
    /* mixed widths and conversions */
    printf("conv: %d %u %d %lld %llu\n", (int8_t)u32, (uint8_t)a32, (int16_t)c64, (long long)(int32_t)u32,
           (unsigned long long)(uint32_t)a64);
    printf("sext: %lld %llu\n", (long long)(int64_t)a8, (unsigned long long)(uint64_t)(int64_t)a16);
}

static void wide(void) {
    u128 a = ((u128)u64 << 64) | 0x0123456789abcdefull;
    u128 b = ((u128)0x1122334455667788ull << 64) | (uint64_t)v64;
    hex128("mul64", (u128)u64 * (u128)(uint64_t)v64);
    hex128("mul128", a * b);
    hex128("add128", a + b);
    hex128("sub128", b - a);
    u128 d = ((u128)0x0000000000abcdefull << 64) | 0xfedcba9876543210ull;
    hex128("udiv", a / d);
    hex128("umod", a % d);
    hex128("udiv64", a / (u128)(uint64_t)u32);
    hex128("umod64", a % (u128)(uint64_t)u32);
    s128 sa = (s128)a, sd = -(s128)(d >> 3);
    hex128("sdiv", (u128)(sa / sd));
    hex128("smod", (u128)(sa % sd));
    hex128("sdiv64", (u128)((s128)v64 * 1000003 / (s128)v32));
    hex128("smod64", (u128)((s128)v64 * 1000003 % (s128)v32));
    hex128("shl", a << (sh + 60));
    hex128("shr", a >> (sh + 60));
    hex128("sar", (u128)((s128)a >> (sh + 60)));
}

static uint32_t rotl32(uint32_t x, unsigned n) { return (x << (n & 31)) | (x >> ((32 - n) & 31)); }
static uint32_t rotr32(uint32_t x, unsigned n) { return (x >> (n & 31)) | (x << ((32 - n) & 31)); }
static uint64_t rotl64(uint64_t x, unsigned n) { return (x << (n & 63)) | (x >> ((64 - n) & 63)); }
static uint16_t rotr16(uint16_t x, unsigned n) { return (uint16_t)((x >> (n & 15)) | (x << ((16 - n) & 15))); }

static void shifts(void) {
    int s = sh;
    printf("shl: %u %u %llu\n", (unsigned)((uint8_t)(u8 << s)), (unsigned)(uint16_t)(u16 << s), (unsigned long long)(u64 << (s + 20)));
    printf("shr: %u %u %llu\n", u8 >> s, u16 >> s, (unsigned long long)(u64 >> (s + 20)));
    printf("sar: %d %d %d %lld\n", v8 >> s, v16 >> s, v32 >> (s + 10), (long long)(v64 >> (s + 40)));
    printf("rot: %08x %08x %016llx %04x\n", rotl32(u32, 5 + s), rotr32(u32, 11 + s),
           (unsigned long long)rotl64(u64, 13 + s), rotr16(u16, 3 + s));
    printf("rotz: %08x %08x\n", rotl32(u32, 0), rotr32(u32, 32));
}

static void bits(void) {
    uint32_t x = 0x00f0a500u;
    uint64_t y = 0x0000123400000000ull;
    printf("ctz: %d %d\n", __builtin_ctz(x), __builtin_ctzll(y));
    printf("clz: %d %d\n", __builtin_clz(x), __builtin_clzll(y));
    printf("popcount: %d %d %d\n", __builtin_popcount(x), __builtin_popcountll(y), __builtin_popcountll(u64));
    printf("bswap: %08x %04x %016llx\n", __builtin_bswap32(x), __builtin_bswap16((uint16_t)u16),
           (unsigned long long)__builtin_bswap64(u64));
    printf("parity: %d %d\n", __builtin_parity(x), __builtin_parityll(u64));
    printf("ffs: %d %d\n", __builtin_ffs((int)x), __builtin_ffsll((long long)y));
}

static uint32_t crc32_of(const unsigned char *p, size_t n) {
    uint32_t crc = 0xFFFFFFFFu;
    for (size_t i = 0; i < n; i++) {
        crc ^= p[i];
        for (int k = 0; k < 8; k++)
            crc = (crc >> 1) ^ (0xEDB88320u & (0u - (crc & 1u)));
    }
    return ~crc;
}

static uint32_t fnv1a32(const unsigned char *p, size_t n) {
    uint32_t h = 2166136261u;
    for (size_t i = 0; i < n; i++) h = (h ^ p[i]) * 16777619u;
    return h;
}

static uint64_t fnv1a64(const unsigned char *p, size_t n) {
    uint64_t h = 14695981039346656037ull;
    for (size_t i = 0; i < n; i++) h = (h ^ p[i]) * 1099511628211ull;
    return h;
}

static uint64_t xorshift64(uint64_t *s) {
    uint64_t x = *s;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    return *s = x;
}

static uint32_t xorshift32(uint32_t *s) {
    uint32_t x = *s;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    return *s = x;
}

static void hashes(void) {
    unsigned char buf[256];
    for (int i = 0; i < 256; i++) buf[i] = (unsigned char)(i * 7 + 3);
    const char *msg = "The quick brown fox jumps over the lazy dog";
    printf("crc32: %08x %08x\n", crc32_of((const unsigned char *)msg, strlen(msg)), crc32_of(buf, sizeof buf));
    printf("fnv32: %08x %08x\n", fnv1a32((const unsigned char *)msg, strlen(msg)), fnv1a32(buf, sizeof buf));
    printf("fnv64: %016llx %016llx\n", (unsigned long long)fnv1a64((const unsigned char *)msg, strlen(msg)),
           (unsigned long long)fnv1a64(buf, sizeof buf));
    uint64_t s64 = 88172645463325252ull;
    uint32_t s32 = 2463534242u;
    uint64_t acc = 0;
    for (int i = 0; i < 1000; i++) acc += xorshift64(&s64) ^ xorshift32(&s32);
    printf("xorshift: %016llx %08x %016llx\n", (unsigned long long)s64, s32, (unsigned long long)acc);
    for (int i = 0; i < 4; i++) printf("xs%d=%llu\n", i, (unsigned long long)(xorshift64(&s64) % 1000000007ull));
}

static int32_t bmin(int32_t a, int32_t b) { return b ^ ((a ^ b) & -(int32_t)(a < b)); }
static int32_t bmax(int32_t a, int32_t b) { return a ^ ((a ^ b) & -(int32_t)(a < b)); }
static int32_t babs(int32_t a) {
    int32_t m = a >> 31;
    return (int32_t)(((uint32_t)a ^ (uint32_t)m) - (uint32_t)m);
}
static int64_t bsign(int64_t a) { return (a >> 63) | (int64_t)((uint64_t)(-a) >> 63); }

static void branchless(void) {
    int32_t t[] = {0, 1, -1, 5, -5, 2147483647, -2147483647, 123456, -654321};
    int n = (int)(sizeof t / sizeof t[0]);
    int64_t sum = 0;
    for (int i = 0; i < n; i++)
        for (int j = 0; j < n; j++) sum += (int64_t)bmin(t[i], t[j]) * 3 + bmax(t[i], t[j]) - babs(t[i] - t[j] / 2);
    printf("branchless: %lld\n", (long long)sum);
    printf("abs: %d %d %d sign: %lld %lld %lld\n", babs(-77), babs(77), babs(0), (long long)bsign(-9000000000ll),
           (long long)bsign(0), (long long)bsign(42));
}

static int jt(int x) {
    switch (x) {
    case 0: return 11;
    case 1: return x * 3 + 5;
    case 2: return 1000 - x;
    case 3: return x << 4;
    case 4: return 77;
    case 5: return x ^ 0x55;
    case 6: return -x;
    case 7: return x * x * x;
    case 8: return 8 + 800;
    case 9: return 9999;
    case 10: return x % 4;
    case 11: return x + 1111;
    case 12: return 12 * 12;
    case 13: return ~x;
    default: return x > 0 ? 31337 : -31337;
    }
}

static int f_add(int a, int b) { return a + b; }
static int f_sub(int a, int b) { return a - b; }
static int f_mul(int a, int b) { return a * b; }
static int f_xor(int a, int b) { return a ^ b; }
static int f_shl(int a, int b) { return a << (b & 15); }

typedef int (*binop)(int, int);

static uint64_t fib(unsigned n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
static uint64_t gcd(uint64_t a, uint64_t b) { return b ? gcd(b, a % b) : a; }
static int ack(int m, int n) { return m == 0 ? n + 1 : (n == 0 ? ack(m - 1, 1) : ack(m - 1, ack(m, n - 1))); }
static uint64_t sumto(uint64_t n) { return n == 0 ? 0 : n + sumto(n - 1); }

static void control(void) {
    long total = 0;
    for (int i = -3; i < 20; i++) {
        int r = jt(i);
        total += r;
        printf("jt(%d)=%d%c", i, r, (i % 6 == 2) ? '\n' : ' ');
    }
    printf("\njt total=%ld\n", total);
    binop ops[] = {f_add, f_sub, f_mul, f_xor, f_shl};
    int acc = 1;
    for (int i = 0; i < 25; i++) acc = ops[i % 5](acc, i + 3) + 1;
    printf("fnptr: %d\n", acc);
    printf("fib(24)=%llu gcd=%llu ack(2,3)=%d sumto(5000)=%llu\n", (unsigned long long)fib(24),
           (unsigned long long)gcd(1071000000007ull * 6, 462000000007ull * 6), ack(2, 3),
           (unsigned long long)sumto(5000));
}

int main(void) {
    arith();
    wide();
    shifts();
    bits();
    hashes();
    branchless();
    control();
    puts("done");
    return 0;
}

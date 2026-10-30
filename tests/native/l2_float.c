/* L2 "translated vs native" program: floating point (SSE2, x87, MXCSR, x87 long double).
 * Deterministic, no input, no addresses. printf is MinGW's own (guest code), and no
 * msvcrt math function is used, so all floating point runs in the translated program. */
#define __USE_MINGW_ANSI_STDIO 1
#include <fenv.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <xmmintrin.h>

/* volatile keeps the compiler from folding the computations away */
static volatile double vd1 = 1.5, vd2 = -2.25, vd3 = 3.0, vd10 = 10.0, vd7 = 7.0, vdz = 0.0;
static volatile float vf1 = 1.5f, vf2 = -2.25f, vf3 = 3.0f, vf10 = 10.0f, vf7 = 7.0f, vfz = 0.0f;
static volatile long double vl1 = 1.5L, vl2 = -2.25L, vl3 = 3.0L, vl10 = 10.0L, vl7 = 7.0L;
static volatile int sink_i;
static uint64_t checksum = 0xcbf29ce484222325ull;

static void mix(uint64_t v) {
    checksum = (checksum ^ v) * 0x100000001b3ull;
    checksum ^= checksum >> 29;
}
static uint64_t bits_d(double d) { uint64_t u; memcpy(&u, &d, 8); return u; }
static uint32_t bits_f(float f) { uint32_t u; memcpy(&u, &f, 4); return u; }

static void pd(const char *name, double d) {
    printf("%-14s %016llx %.17g %a\n", name, (unsigned long long)bits_d(d), d, d);
    mix(bits_d(d));
}
static void pf(const char *name, float f) {
    printf("%-14s %08x %.9g %a\n", name, bits_f(f), (double)f, (double)f);
    mix(bits_f(f));
}
static void pi(const char *name, long long v) {
    printf("%-14s %lld\n", name, v);
    mix((uint64_t)v);
}
static void pl(const char *name, long double l) {
    printf("%-14s %.21Lg %Le %Lf\n", name, l, l, l);
    /* mix the 64-bit mantissa and the sign/exponent through exact integer ops */
    int e;
    long double m = frexpl(l, &e);
    mix((uint64_t)e);
    mix((uint64_t)(long long)ldexpl(m, 62));
}
static void flags(const char *name) {
    int f = fetestexcept(FE_ALL_EXCEPT);
    printf("%-14s inexact=%d overflow=%d underflow=%d invalid=%d divzero=%d\n", name,
           !!(f & FE_INEXACT), !!(f & FE_OVERFLOW), !!(f & FE_UNDERFLOW), !!(f & FE_INVALID),
           !!(f & FE_DIVBYZERO));
    mix((uint64_t)f);
    feclearexcept(FE_ALL_EXCEPT);
}

static void special(void) {
    puts("== special values");
    double dz = vdz, one = 1.0;
    pd("d +0", dz);
    pd("d -0", -dz);
    pd("d min", 2.2250738585072014e-308);
    pd("d denmin", 4.9406564584124654e-324);
    pd("d denmax", 2.2250738585072009e-308);
    pd("d max", 1.7976931348623157e308);
    pd("d +inf", one / dz);
    pd("d -inf", -one / dz);
    pd("d nan", dz / dz);
    pd("d -nan", -(dz / dz));
    float fz = vfz, fone = 1.0f;
    pf("f +0", fz);
    pf("f -0", -fz);
    pf("f min", 1.17549435e-38f);
    pf("f denmin", 1.40129846e-45f);
    pf("f denmax", 1.17549421e-38f);
    pf("f max", 3.40282347e38f);
    pf("f +inf", fone / fz);
    pf("f -inf", -fone / fz);
    pf("f nan", fz / fz);
    printf("classify d: %d %d %d %d\n", isnan(dz / dz), isinf(one / dz), isfinite(one),
           signbit(-dz));
    printf("classify f: %d %d %d %d\n", isnan(fz / fz), isinf(fone / fz), isfinite(fone),
           signbit(-fz));
}

static void arith(void) {
    puts("== arithmetic");
    double a = vd1, b = vd2, c = vd3, t = vd10, s = vd7;
    pd("d a+b", a + b);
    pd("d a-b", a - b);
    pd("d a*b", a * b);
    pd("d a/c", a / c);
    pd("d t/s", t / s);
    pd("d sqrt(c)", sqrt(c));
    pd("d sqrt(s)", sqrt(s));
    pd("d sqrt(-a)", sqrt(-a));
    pd("d fabs", fabs(b));
    pd("d min", a < b ? a : b);
    pd("d max", a > b ? a : b);
    pd("d fma-ish", a * b + c);
    float fa = vf1, fb = vf2, fc = vf3, ft = vf10, fs = vf7;
    pf("f a+b", fa + fb);
    pf("f a-b", fa - fb);
    pf("f a*b", fa * fb);
    pf("f a/c", fa / fc);
    pf("f t/s", ft / fs);
    pf("f sqrt(c)", sqrtf(fc));
    pf("f sqrt(s)", sqrtf(fs));
    pf("f fabs", fabsf(fb));
    pf("f min", fa < fb ? fa : fb);
    pf("f max", fa > fb ? fa : fb);
    pd("d inf-inf", (1.0 / (double)vdz) - (1.0 / (double)vdz));
    pd("d 0*inf", (double)vdz * (1.0 / (double)vdz));
}

static void convert(void) {
    puts("== conversions");
    static volatile int32_t i32s[] = {0, 1, -1, 123456789, -123456789, 2147483647, -2147483647 - 1};
    static volatile int64_t i64s[] = {0, 1, -1, 9007199254740993ll, -9007199254740993ll,
                                      9223372036854775807ll, -9223372036854775807ll - 1};
    static volatile uint32_t u32s[] = {0u, 1u, 2147483648u, 4294967295u};
    static volatile uint64_t u64s[] = {0ull, 1ull, 9223372036854775808ull, 18446744073709551615ull,
                                       16777217ull};
    for (unsigned i = 0; i < 7; i++) {
        int32_t v = i32s[i];
        printf("i32 %d: d=%.17g f=%.9g\n", v, (double)v, (double)(float)v);
        mix(bits_d((double)v));
        mix(bits_f((float)v));
    }
    for (unsigned i = 0; i < 7; i++) {
        int64_t v = i64s[i];
        printf("i64 %lld: d=%.17g f=%.9g\n", v, (double)v, (double)(float)v);
        mix(bits_d((double)v));
        mix(bits_f((float)v));
    }
    for (unsigned i = 0; i < 4; i++) {
        uint32_t v = u32s[i];
        printf("u32 %u: d=%.17g f=%.9g\n", v, (double)v, (double)(float)v);
        mix(bits_d((double)v));
        mix(bits_f((float)v));
    }
    for (unsigned i = 0; i < 5; i++) {
        uint64_t v = u64s[i];
        printf("u64 %llu: d=%.17g f=%.9g\n", v, (double)v, (double)(float)v);
        mix(bits_d((double)v));
        mix(bits_f((float)v));
    }
    static volatile double ds[] = {0.0, -0.0, 0.5, -0.5, 1.5, -1.5, 2.5, -2.5, 2.9999, -2.9999,
                                   2147483647.0, -2147483648.0, 4294967295.0, 4503599627370497.5,
                                   9.2e18, -9.2e18, 1.0e10, 1.8e19};
    for (unsigned i = 0; i < sizeof ds / sizeof ds[0]; i++) {
        double d = ds[i];
        printf("d %.17g:", d);
        if (fabs(d) < 2147483648.0) printf(" i32=%d", (int32_t)d);
        if (fabs(d) < 9.3e18) printf(" i64=%lld", (long long)d);
        if (d >= 0 && d < 4294967296.0) printf(" u32=%u", (uint32_t)d);
        if (d >= 0 && d < 1.8446744073709552e19) printf(" u64=%llu", (unsigned long long)d);
        printf(" rint=%.17g nearby=%.17g", rint(d), nearbyint(d));
        if (fabs(d) < 9.3e18) printf(" lrint=%ld llrint=%lld", lrint(d), llrint(d));
        printf("\n");
        mix(bits_d(rint(d)));
        mix(bits_d(nearbyint(d)));
    }
    static volatile float fs[] = {0.5f, -0.5f, 1.5f, -1.5f, 2.5f, 2.9999f, -2.9999f, 16777217.0f,
                                  3.0e9f, 1.0e-10f};
    for (unsigned i = 0; i < sizeof fs / sizeof fs[0]; i++) {
        float f = fs[i];
        printf("f %.9g:", (double)f);
        if (fabsf(f) < 2147483648.0f) printf(" i32=%d", (int32_t)f);
        printf(" i64=%lld", (long long)f);
        if (f >= 0) printf(" u32=%u u64=%llu", f < 4294967296.0f ? (uint32_t)f : 0u, (unsigned long long)f);
        printf(" rintf=%.9g lrintf=%ld", (double)rintf(f), lrintf(f));
        printf(" d=%.17g\n", (double)f);
        mix(bits_f(rintf(f)));
    }
    static volatile double narrow[] = {1.0 / 3.0, 1.0e39, -1.0e39, 1.0e-46, 3.4028235677973366e38,
                                       1.17549435e-38, 1.0e-40, 16777217.0};
    for (unsigned i = 0; i < 8; i++) {
        float f = (float)narrow[i];
        pf("d->f", f);
    }
}

static void compares(void) {
    puts("== compares");
    double v[4] = {vd1, vd2, 1.0 / vdz - 1.0 / vdz, 1.0 / vdz};
    const char *n[4] = {"1.5", "-2.25", "nan", "inf"};
    unsigned acc = 0;
    for (int i = 0; i < 4; i++)
        for (int j = 0; j < 4; j++) {
            double a = v[i], b = v[j];
            unsigned bitsv = (a < b) | (a <= b) << 1 | (a > b) << 2 | (a >= b) << 3 | (a == b) << 4 |
                             (a != b) << 5 | isless(a, b) << 6 | isgreater(a, b) << 7 |
                             isunordered(a, b) << 8 | islessgreater(a, b) << 9;
            printf("d %-5s ? %-5s : %03x\n", n[i], n[j], bitsv);
            acc = acc * 31 + bitsv;
        }
    float w[4] = {vf1, vf2, vfz / vfz, 1.0f / vfz};
    for (int i = 0; i < 4; i++)
        for (int j = 0; j < 4; j++) {
            float a = w[i], b = w[j];
            unsigned bitsv = (a < b) | (a <= b) << 1 | (a > b) << 2 | (a >= b) << 3 | (a == b) << 4 |
                             (a != b) << 5 | isless(a, b) << 6 | isunordered(a, b) << 7;
            printf("f %-5s ? %-5s : %03x\n", n[i], n[j], bitsv);
            acc = acc * 31 + bitsv;
        }
    printf("compare acc %u\n", acc);
    mix(acc);
}

static const char *mode_name(int m) {
    return m == FE_TONEAREST ? "nearest" : m == FE_DOWNWARD ? "down" : m == FE_UPWARD ? "up" : "zero";
}

static void rounding(void) {
    puts("== rounding modes");
    static const int modes[4] = {FE_TONEAREST, FE_DOWNWARD, FE_UPWARD, FE_TOWARDZERO};
    for (int k = 0; k < 4; k++) {
        if (fesetround(modes[k]) != 0) puts("fesetround failed");
        printf("mode %s (now %s)\n", mode_name(modes[k]), mode_name(fegetround()));
        double one = vd1 / vd1, three = vd3, ten = vd10, seven = vd7;
        pd(" d 1/3", one / three);
        pd(" d -1/3", -one / three);
        pd(" d 10/7", ten / seven);
        pd(" d sqrt(2)", sqrt(one + one));
        pd(" d sqrt(10)", sqrt(ten));
        pd(" d 1+tiny", one + 1.0e-17);
        pd(" d -1-tiny", -one - 1.0e-17);
        float fone = vf1 / vf1, fthree = vf3, ften = vf10;
        pf(" f 1/3", fone / fthree);
        pf(" f -1/3", -fone / fthree);
        pf(" f sqrt(10)", sqrtf(ften));
        pf(" f i64->f", (float)(int64_t)(9007199254740993ll + (int64_t)sink_i));
        pd(" d i64->d", (double)(int64_t)(9007199254740993ll + (int64_t)sink_i));
        pf(" f d->f", (float)(vd10 / vd3));
        pf(" f 1e38*10", ften * 1.0e37f);
        pi(" lrint 2.5", lrint(2.5 + (double)vdz));
        pi(" lrint -2.5", lrint(-2.5 + (double)vdz));
        pi(" lrint 1.4", lrint(1.4 + (double)vdz));
        pi(" lrint -1.4", lrint(-1.4 + (double)vdz));
        pi(" cast 2.7", (long long)(2.7 + (double)vdz));
        pi(" cast -2.7", (long long)(-2.7 + (double)vdz));
        long double lone = vl1 / vl1, lthree = vl3;
        pl(" l 1/3", lone / lthree);
        pl(" l -1/3", -lone / lthree);
        pl(" l sqrt(10)", sqrtl(vl10));
        printf(" printf 1/3 %.20f %.5e %.3f\n", one / three, one / three, 2.0005 + (double)vdz);
        printf(" printf 2/3 %.20Lf\n", (lone + lone) / lthree);
    }
    fesetround(FE_TONEAREST);
    printf("restored %s\n", mode_name(fegetround()));
    mix((uint64_t)fegetround());
}

static void exceptions(void) {
    puts("== exception flags");
    feclearexcept(FE_ALL_EXCEPT);
    double one = vd1 / vd1, three = vd3, big = 1.0e308 + (double)vdz, tiny = 1.0e-308 + (double)vdz;
    double zero = vdz;
    volatile double r;
    r = one + one; flags("d 1+1");
    r = one / three; flags("d 1/3");
    r = big * 10.0; flags("d big*10");
    r = tiny * 1.0e-10; flags("d tiny*1e-10");
    r = tiny / 3.0; flags("d tiny/3");
    r = zero / zero; flags("d 0/0");
    r = one / zero; flags("d 1/0");
    r = sqrt(-one); flags("d sqrt(-1)");
    r = sqrt(three); flags("d sqrt(3)");
    r = sqrt(4.0 * one); flags("d sqrt(4)");
    r = (double)(int64_t)(9007199254740993ll + sink_i); flags("d i64 inexact");
    sink_i = (int)lrint(2.5 + zero); flags("d lrint 2.5");
    sink_i = (int)(three + 0.5); flags("d cvttsd 3.5");
    sink_i = (int)(1.0e30 * one > 0 ? 0 : 1); flags("d cmp");
    { volatile double nan = zero / zero; feclearexcept(FE_ALL_EXCEPT);
      sink_i = nan < one; flags("d nan <"); sink_i = nan == one; flags("d nan =="); }
    float fone = vf1 / vf1, fz = vfz, fbig = 3.0e38f + vfz;
    volatile float fr;
    fr = fone / vf3; flags("f 1/3");
    fr = fbig * 10.0f; flags("f big*10");
    fr = fz / fz; flags("f 0/0");
    fr = fone / fz; flags("f 1/0");
    fr = 1.0e-38f * (1.0e-5f + fz); flags("f tiny");
    long double lone = vl1 / vl1;
    volatile long double lr;
    lr = lone / vl3; flags("l 1/3");
    lr = lone + lone; flags("l 1+1");
    lr = lone / 0.0L * 0 ; flags("l inf*0");
    lr = sqrtl(vl10); flags("l sqrt(10)");
    feraiseexcept(FE_INVALID | FE_DIVBYZERO);
    flags("raise");
    (void)r; (void)fr; (void)lr;
}

static volatile double vdmin = 2.2250738585072014e-308, vdhalf = 0.5, vden = 1.0e-310;
static volatile float vfmin = 1.17549435e-38f, vfhalf = 0.5f, vfden = 1.0e-40f;

static void ftz_daz(void) {
    puts("== ftz and daz");
    unsigned saved = _mm_getcsr();
    printf("mxcsr default %04x\n", saved & 0xffc0u);
    for (int m = 0; m < 4; m++) {
        unsigned csr = (saved & ~0x8040u) | ((m & 1) ? 0x8000u : 0) | ((m & 2) ? 0x0040u : 0);
        _mm_setcsr(csr);
        printf("ftz=%d daz=%d csr=%04x\n", m & 1, !!(m & 2), _mm_getcsr() & 0xffc0u);
        pd(" d min*half", vdmin * vdhalf);
        pd(" d den+0", vden + 0.0 * vdhalf);
        pd(" d den*1", vden * (1.0 + (double)vdz));
        pd(" d den*2", vden * 2.0);
        pd(" d den+den", vden + vden);
        pd(" d 1/den", 1.0 / vden);
        pf(" f min*half", vfmin * vfhalf);
        pf(" f den*1", vfden * (1.0f + vfz));
        pf(" f den+den", vfden + vfden);
        pf(" f sqrt(den)", sqrtf(vfden));
        printf(" den cmp: %d %d\n", vden == 0.0, vfden > 0.0f);
        printf(" mxcsr flags %02x\n", _mm_getcsr() & 0x3f);
        _mm_setcsr(_mm_getcsr() & ~0x3fu);
    }
    _mm_setcsr(saved);
    printf("restored csr=%04x\n", _mm_getcsr() & 0xffc0u);
    mix(_mm_getcsr() & 0xffc0u);
}

#define N 37
static float fa[N], fb[N], fc[N];
static double da[N], db[N], dc[N];
static int ia[N];

static void vec(void) {
    puts("== vectorized loops");
    float fs = vf1; double ds = vd2;
    for (int i = 0; i < N; i++) {
        ia[i] = i * 3 - 50 + sink_i;
        fa[i] = (float)i * 0.25f + fs;
        fb[i] = (float)(N - i) * 0.5f;
        da[i] = (double)i * 0.125 + ds;
        db[i] = (double)(N - i) * 0.75;
    }
    for (int i = 0; i < N; i++) fc[i] = fa[i] * fb[i] + fs;
    for (int i = 0; i < N; i++) dc[i] = da[i] * db[i] + ds;
    float fsum = 0; double dsum = 0;
    for (int i = 0; i < N; i++) { fa[i] = fa[i] + fb[i]; fb[i] = fc[i] - fa[i]; }
    for (int i = 0; i < N; i++) { da[i] = da[i] - db[i]; db[i] = dc[i] / (db[i] + 1.0); }
    for (int i = 0; i < N; i++) { fsum += fc[i]; dsum += dc[i]; }
    float fcv[N]; double dcv[N];
    for (int i = 0; i < N; i++) fcv[i] = (float)ia[i];
    for (int i = 0; i < N; i++) dcv[i] = (double)ia[i] * 0.5;
    int itr[N], idt[N];
    for (int i = 0; i < N; i++) itr[i] = (int)(fb[i] * 0.5f);
    for (int i = 0; i < N; i++) idt[i] = (int)(db[i] * 0.5);
    float fsq[N]; double dsq[N];
    for (int i = 0; i < N; i++) fsq[i] = sqrtf(fabsf(fa[i]));
    for (int i = 0; i < N; i++) dsq[i] = sqrt(fabs(da[i]));
    float fmx[N]; double dmx[N];
    for (int i = 0; i < N; i++) fmx[i] = fa[i] > fb[i] ? fa[i] : fb[i];
    for (int i = 0; i < N; i++) dmx[i] = da[i] < db[i] ? da[i] : db[i];
    double dfromf[N];
    for (int i = 0; i < N; i++) dfromf[i] = (double)fc[i];
    float ffromd[N];
    for (int i = 0; i < N; i++) ffromd[i] = (float)dc[i];
    uint64_t h = 0;
    for (int i = 0; i < N; i++) {
        h = h * 131 + bits_f(fc[i]); h = h * 131 + bits_d(dc[i]);
        h = h * 131 + bits_f(fcv[i]); h = h * 131 + bits_d(dcv[i]);
        h = h * 131 + (uint32_t)itr[i]; h = h * 131 + (uint32_t)idt[i];
        h = h * 131 + bits_f(fsq[i]); h = h * 131 + bits_d(dsq[i]);
        h = h * 131 + bits_f(fmx[i]); h = h * 131 + bits_d(dmx[i]);
        h = h * 131 + bits_d(dfromf[i]); h = h * 131 + bits_f(ffromd[i]);
    }
    pf("f sum", fsum);
    pd("d sum", dsum);
    printf("f[0..3] %.9g %.9g %.9g %.9g\n", (double)fc[0], (double)fc[1], (double)fc[2], (double)fc[3]);
    printf("d[0..3] %.17g %.17g %.17g %.17g\n", dc[0], dc[1], dc[2], dc[3]);
    printf("vec hash %016llx\n", (unsigned long long)h);
    mix(h);
}

static void longdouble(void) {
    puts("== long double");
    long double a = vl1, b = vl2, c = vl3, t = vl10, s = vl7;
    pl("l a+b", a + b);
    pl("l a-b", a - b);
    pl("l a*b", a * b);
    pl("l a/c", a / c);
    pl("l t/s", t / s);
    pl("l sqrt(s)", sqrtl(s));
    pl("l sqrt(c)", sqrtl(c));
    pl("l fabs", fabsl(b));
    pl("l -a", -a);
    pl("l 1/3*3", (a / a / c) * c);
    pl("l big", 1.0e4000L * (a / a) );
    pl("l small", 1.0e-4000L * (a / a));
    pl("l 2^64+1", 18446744073709551616.0L + (a / a));
    static volatile int64_t i64s[] = {0, 1, -1, 123456789012345678ll, 9223372036854775807ll,
                                      -9223372036854775807ll - 1, 9007199254740993ll};
    for (unsigned i = 0; i < 7; i++) {
        int64_t v = i64s[i];
        long double l = (long double)v;
        printf("i64 %lld -> l=%.21Lg back=%lld d=%.17g f=%.9g\n", v, l, (long long)l, (double)l,
               (double)(float)l);
        mix((uint64_t)(long long)l);
        mix(bits_d((double)l));
    }
    static volatile double ds[] = {0.1, -0.1, 1.0e300, 5.0e-324, 2.5, -7.9, 4503599627370497.5};
    for (unsigned i = 0; i < 7; i++) {
        double d = ds[i];
        long double l = (long double)d;
        printf("d %.17g -> l=%.21Lg back=%.17g trunc=%lld\n", d, l, (double)l,
               fabs(d) < 9.0e18 ? (long long)l : 0ll);
        mix(bits_d((double)l));
    }
    static volatile int32_t i32s[] = {0, -5, 2147483647, -2147483647 - 1};
    for (unsigned i = 0; i < 4; i++) {
        long double l = (long double)(int32_t)i32s[i];
        printf("i32 %d -> l=%Lg\n", i32s[i], l);
    }
    static volatile unsigned long long u64s[] = {0ull, 18446744073709551615ull, 9223372036854775808ull};
    for (unsigned i = 0; i < 3; i++) {
        long double l = (long double)(unsigned long long)u64s[i];
        printf("u64 %llu -> l=%.21Lg back=%llu\n", u64s[i], l, (unsigned long long)l);
        mix((uint64_t)(unsigned long long)l);
    }
    long double n = (a / 1.0L) * 0 / (long double)(int)sink_i;
    long double inf = a / (long double)(int)sink_i;
    long double xs[5] = {a, b, n, inf, -inf};
    const char *nm[5] = {"1.5", "-2.25", "nan", "inf", "-inf"};
    unsigned acc = 0;
    for (int i = 0; i < 5; i++)
        for (int j = 0; j < 5; j++) {
            long double x = xs[i], y = xs[j];
            unsigned bv = (x < y) | (x <= y) << 1 | (x > y) << 2 | (x >= y) << 3 | (x == y) << 4 |
                          (x != y) << 5 | isunordered(x, y) << 6;
            printf("l %-5s ? %-5s : %02x\n", nm[i], nm[j], bv);
            acc = acc * 31 + bv;
        }
    mix(acc);
    pl("l inf", inf);
    pl("l nan", n);
    printf("l classify %d %d %d\n", isnan(n), isinf(inf), isfinite(a));
    printf("l %%Lf %Lf %.3Lf %10.2Lf|%-10.1Lf|\n", t / s, t / s, -t / s, t / s);
    printf("l %%Lg %Lg %Lg %Lg\n", t / s, 1.0e-5L * (a / a), 1.0e20L * (a / a));
}

int main(void) {
    special();
    arith();
    convert();
    compares();
    rounding();
    exceptions();
    ftz_daz();
    vec();
    longdouble();
    printf("checksum %016llx\n", (unsigned long long)checksum);
    return 23;
}

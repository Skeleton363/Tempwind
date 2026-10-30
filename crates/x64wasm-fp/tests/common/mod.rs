//! Case generation shared by the native differential tests and the
//! `vectors` example: edge values, a seeded RNG and random operands.
#![allow(dead_code)]

/// splitmix64.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    pub fn pick<T: Copy>(&mut self, xs: &[T]) -> T {
        xs[self.below(xs.len() as u64) as usize]
    }
}

/// Positive f32 edges; the tests add the negatives.
pub const F32_EDGES: &[u32] = &[
    0x0000_0000, 0x0000_0001, 0x0000_0002, 0x0000_0003, 0x003F_FFFF, 0x0040_0000, 0x0040_0001,
    0x007F_FFFE, 0x007F_FFFF, 0x0080_0000, 0x0080_0001, 0x00FF_FFFF, 0x0100_0000, 0x0C00_0000,
    0x1F80_0000, 0x3380_0000, 0x3380_0001, 0x3400_0000, 0x3E80_0000, 0x3F00_0000, 0x3F7F_FFFF,
    0x3F80_0000, 0x3F80_0001, 0x3FC0_0000, 0x4000_0000, 0x4020_0000, 0x4040_0000, 0x4049_0FDB,
    0x4B00_0000, 0x4B7F_FFFF, 0x4B80_0000, 0x4EFF_FFFF, 0x4F00_0000, 0x4F00_0001, 0x5EFF_FFFF,
    0x5F00_0000, 0x5F80_0000, 0x7EFF_FFFF, 0x7F00_0000, 0x7F7F_FFFE, 0x7F7F_FFFF, 0x7F80_0000,
    0x7F80_0001, 0x7FA0_0000, 0x7FBF_FFFF, 0x7FC0_0000, 0x7FC0_0001, 0x7FE0_0000, 0x7FFF_FFFF,
];

pub const F64_EDGES: &[u64] = &[
    0x0000_0000_0000_0000,
    0x0000_0000_0000_0001,
    0x0000_0000_0000_0002,
    0x0000_0000_0000_0003,
    0x0007_FFFF_FFFF_FFFF,
    0x0008_0000_0000_0000,
    0x0008_0000_0000_0001,
    0x000F_FFFF_FFFF_FFFE,
    0x000F_FFFF_FFFF_FFFF,
    0x0010_0000_0000_0000,
    0x0010_0000_0000_0001,
    0x001F_FFFF_FFFF_FFFF,
    0x0020_0000_0000_0000,
    0x1FF0_0000_0000_0000,
    0x3690_0000_0000_0000, // 2^-150
    0x36A0_0000_0000_0000, // 2^-149
    0x3800_0000_0000_0000, // 2^-127
    0x380F_FFFF_E000_0000, // just below f32 MIN_NORMAL
    0x3810_0000_0000_0000, // f32 MIN_NORMAL
    0x3CA0_0000_0000_0000, // 2^-53
    0x3CA0_0000_0000_0001,
    0x3CB0_0000_0000_0000,
    0x3FD0_0000_0000_0000,
    0x3FE0_0000_0000_0000,
    0x3FEF_FFFF_FFFF_FFFF,
    0x3FF0_0000_0000_0000,
    0x3FF0_0000_0000_0001,
    0x3FF0_0000_1000_0000, // 1 + 2^-24: an f32 tie
    0x3FF8_0000_0000_0000,
    0x4000_0000_0000_0000,
    0x4004_0000_0000_0000,
    0x4008_0000_0000_0000,
    0x4009_21FB_5444_2D18,
    0x4330_0000_0000_0000,
    0x433F_FFFF_FFFF_FFFF,
    0x4340_0000_0000_0000,
    0x41DF_FFFF_FFC0_0000, // 2^31 - 1
    0x41DF_FFFF_FFE0_0000, // 2^31 - 0.5
    0x41E0_0000_0000_0000, // 2^31
    0x43DF_FFFF_FFFF_FFFF,
    0x43E0_0000_0000_0000, // 2^63
    0x43F0_0000_0000_0000,
    0x47EF_FFFF_E000_0000, // f32 MAX
    0x47EF_FFFF_F000_0000, // f32 MAX + half ulp
    0x47F0_0000_0000_0000,
    0x7FD0_0000_0000_0000,
    0x7FEF_FFFF_FFFF_FFFE,
    0x7FEF_FFFF_FFFF_FFFF,
    0x7FF0_0000_0000_0000,
    0x7FF0_0000_0000_0001,
    0x7FF4_0000_0000_0000,
    0x7FF7_FFFF_FFFF_FFFF,
    0x7FF8_0000_0000_0000,
    0x7FF8_0000_0000_0001,
    0x7FFC_0000_2000_0000,
    0x7FFF_FFFF_FFFF_FFFF,
];

pub fn with_neg32(xs: &[u32]) -> Vec<u32> {
    xs.iter().flat_map(|&x| [x, x | 0x8000_0000]).collect()
}
pub fn with_neg64(xs: &[u64]) -> Vec<u64> {
    xs.iter().flat_map(|&x| [x, x | (1 << 63)]).collect()
}

pub const I32_EDGES: &[i32] = &[
    0, 1, -1, 2, 3, 0x7F, 0xFF_FFFF, 0x100_0000, 0x100_0001, 0x100_0003, 0x7FFF_FF80, 0x7FFF_FFC0,
    0x7FFF_FFFF, -0x7FFF_FFFF, i32::MIN, -0x100_0001, 0x5555_5555,
];
pub const I64_EDGES: &[i64] = &[
    0,
    1,
    -1,
    0x100_0001,
    0x20_0000_0000_0001,
    0x20_0000_0000_0003,
    0x7FFF_FFFF_FFFF_FC00,
    0x7FFF_FFFF_FFFF_FE00,
    0x7FFF_FFFF_FFFF_FFFF,
    -0x7FFF_FFFF_FFFF_FFFF,
    i64::MIN,
    0x7FFF_FF80_0000_0000,
    0x7FFF_FFC0_0000_0000,
    -0x20_0000_0000_0001,
    0x5555_5555_5555_5555,
];

/// A random f32: uniform bits, or a structured value (exponent near an
/// edge, mantissa with runs of ones or zeros).
pub fn rand_f32(r: &mut Rng) -> u32 {
    match r.below(4) {
        0 => r.next() as u32,
        1 => r.pick(F32_EDGES) ^ ((r.next() as u32) & r.pick(&[1, 3, 0xFF, 0x7F_FFFF])) ^ ((r.below(2) as u32) << 31),
        _ => {
            let e = match r.below(4) {
                0 => r.below(8) as u32,
                1 => 0xFF - r.below(8) as u32,
                2 => 127 + r.below(64) as u32 - 32,
                _ => r.below(256) as u32,
            };
            let m = match r.below(3) {
                0 => r.next() as u32,
                1 => (r.next() as u32) >> r.below(23),
                _ => !((r.next() as u32) >> r.below(23)),
            } & 0x7F_FFFF;
            ((r.below(2) as u32) << 31) | (e << 23) | m
        }
    }
}

pub fn rand_f64(r: &mut Rng) -> u64 {
    match r.below(4) {
        0 => r.next(),
        1 => r.pick(F64_EDGES) ^ (r.next() & r.pick(&[1, 3, 0xFFFF, 0xF_FFFF_FFFF_FFFF, 0x1FFF_FFFF])) ^ (r.below(2) << 63),
        _ => {
            let e = match r.below(5) {
                0 => r.below(8),
                1 => 0x7FF - r.below(8),
                2 => 1023 + r.below(128) - 64,
                3 => 1023 + r.below(300) - 150,
                _ => r.below(2048),
            };
            let m = match r.below(3) {
                0 => r.next(),
                1 => r.next() >> r.below(52),
                _ => !(r.next() >> r.below(52)),
            } & 0xF_FFFF_FFFF_FFFF;
            (r.below(2) << 63) | (e << 52) | m
        }
    }
}

pub fn rand_i32(r: &mut Rng) -> i32 {
    match r.below(3) {
        0 => r.next() as i32,
        1 => r.pick(I32_EDGES).wrapping_add(r.below(5) as i32 - 2),
        _ => (r.next() as i32) >> r.below(32),
    }
}
pub fn rand_i64(r: &mut Rng) -> i64 {
    match r.below(3) {
        0 => r.next() as i64,
        1 => r.pick(I64_EDGES).wrapping_add(r.below(5) as i64 - 2),
        _ => (r.next() as i64) >> r.below(64),
    }
}

/// MXCSR with all exceptions masked, the given RC/FTZ/DAZ combination
/// (0..16) and the given sticky flags.
pub fn mxcsr(mode: u32, sticky: u32) -> u32 {
    0x1F80 | ((mode & 3) << 13) | if mode & 4 != 0 { 0x8000 } else { 0 } | if mode & 8 != 0 { 0x40 } else { 0 } | (sticky & 0x3F)
}

/// Random sticky flags: none half the time.
pub fn sticky(r: &mut Rng) -> u32 {
    if r.below(2) == 0 { 0 } else { r.below(64) as u32 }
}

// ---- x87 ----

/// Positive 80-bit edges (m, se); the tests add the negatives.
pub const F80_EDGES: &[(u64, u32)] = &[
    (0, 0),
    (1, 0),
    (0x7FFF_FFFF_FFFF_FFFF, 0),
    (0x4000_0000_0000_0000, 0),
    (0x8000_0000_0000_0000, 0), // pseudo-denormals
    (0xFFFF_FFFF_FFFF_FFFF, 0),
    (0x8000_0000_0000_0000, 1), // MIN_NORMAL
    (0x8000_0000_0000_0001, 1),
    (0xFFFF_FFFF_FFFF_FFFF, 1),
    (0x8000_0000_0000_0000, 0x3FFF), // 1
    (0x8000_0000_0000_0001, 0x3FFF),
    (0xFFFF_FFFF_FFFF_FFFF, 0x3FFE),
    (0xC000_0000_0000_0000, 0x3FFF), // 1.5
    (0x8000_0000_0000_0000, 0x3FFE), // 0.5
    (0x8000_0000_0000_0000, 0x4000), // 2
    (0xA000_0000_0000_0000, 0x4000), // 2.5
    (0xC000_0000_0000_0000, 0x4000), // 3
    (0x8000_0080_0000_0000, 0x3FFF), // PC 24 tie
    (0x8000_0180_0000_0000, 0x3FFF),
    (0x8000_0000_0000_0400, 0x3FFF), // PC 53 tie
    (0x8000_0000_0000_0C00, 0x3FFF),
    (0xFFFF_FF80_0000_0000, 0x3FFF),
    (0xFFFF_FFFF_FFFF_FC00, 0x3FFF),
    (0xC90F_DAA2_2168_C235, 0x4000), // pi
    (0x8000_0000_0000_0000, 0x3FFF + 15), // 2^15
    (0xFFFE_0000_0000_0000, 0x3FFF + 14), // 32767
    (0xFFFF_0000_0000_0000, 0x3FFF + 14), // 32767.5
    (0x8000_0000_0000_0000, 0x3FFF + 31), // 2^31
    (0xFFFF_FFFE_0000_0000, 0x3FFF + 30), // 2^31 - 1
    (0xFFFF_FFFF_0000_0000, 0x3FFF + 30), // 2^31 - 0.5
    (0x8000_0000_0000_0000, 0x3FFF + 63), // 2^63
    (0xFFFF_FFFF_FFFF_FFFF, 0x3FFF + 62), // 2^63 - 1
    (0xFFFF_FFFF_FFFF_FFFF, 0x3FFF + 61), // 2^62 - 0.5
    (0x8000_0000_0000_0000, 0x3FFF + 64),
    (0x8000_0000_0000_0000, 0x3FFF + 100),
    (0x8000_0000_0000_0000, 0x3F81), // f32 MIN_NORMAL
    (0xFFFF_FF00_0000_0000, 0x3F80), // just below it
    (0x8000_0000_0000_0000, 0x3C01), // f64 MIN_NORMAL
    (0xFFFF_FFFF_FFFF_F800, 0x407E), // f32 MAX
    (0xFFFF_FFFF_FFFF_F800, 0x43FE), // f64 MAX
    (0x8000_0000_0000_0000, 0x2000),
    (0x8000_0000_0000_0000, 0x6000),
    (0xFFFF_FFFF_FFFF_FFFF, 0x7FFE), // MAX
    (0x8000_0000_0000_0000, 0x7FFE),
    (0x8000_0000_0000_0000, 0x7FFF), // inf
    (0x0000_0000_0000_0000, 0x7FFF), // pseudo-infinity
    (0x4000_0000_0000_0000, 0x7FFF), // pseudo-NaN
    (0x0000_0000_0000_0001, 0x7FFF),
    (0xC000_0000_0000_0000, 0x7FFF), // QNaN
    (0xC000_0000_0000_0001, 0x7FFF),
    (0xFFFF_FFFF_FFFF_FFFF, 0x7FFF),
    (0x8000_0000_0000_0001, 0x7FFF), // SNaN
    (0xA000_0000_0000_0000, 0x7FFF),
    (0xBFFF_FFFF_FFFF_FFFF, 0x7FFF),
    (0x4000_0000_0000_0000, 0x3FFF), // unnormals
    (0x0000_0000_0000_0000, 0x3FFF),
    (0x0000_0000_0000_0001, 0x0001),
];

pub fn with_neg80(xs: &[(u64, u32)]) -> Vec<(u64, u32)> {
    xs.iter().flat_map(|&(m, se)| [(m, se), (m, se | 0x8000)]).collect()
}

pub fn rand_f80(r: &mut Rng) -> (u64, u32) {
    let sign = (r.below(2) as u32) << 15;
    match r.below(6) {
        0 => (r.next(), r.next() as u32 & 0xFFFF),
        1 => {
            let (m, se) = r.pick(F80_EDGES);
            (m ^ (r.next() & r.pick(&[1, 3, 0xFF, 0xFF_FFFF_FFFF, u64::MAX >> 1])), se ^ sign)
        }
        2 => {
            // An exact f32 or f64 value, so x87 results meet ties.
            let (m, se) = if r.below(2) == 0 {
                (r.next() & !((1 << 40) - 1), 0x3FFF + r.below(64) as u32 - 32)
            } else {
                (r.next() & !((1 << 11) - 1), 0x3FFF + r.below(256) as u32 - 128)
            };
            (m | (1 << 63), se | sign)
        }
        _ => {
            let e = match r.below(5) {
                0 => r.below(70) as u32,
                1 => 0x7FFF - r.below(4) as u32,
                2 => 0x3FFF + r.below(140) as u32 - 70,
                3 => 0x3FFF + r.below(0x800) as u32 - 0x400,
                _ => r.below(0x8000) as u32,
            };
            let m = match r.below(3) {
                0 => r.next(),
                1 => r.next() >> r.below(63),
                _ => !(r.next() >> r.below(63)),
            };
            // Mostly normalized (J set unless the exponent is 0).
            let m = if e != 0 && r.below(16) != 0 { m | (1 << 63) } else { m };
            (m, e | sign)
        }
    }
}

/// x87 control word with all exceptions masked, the given PC (0..4) and RC.
pub fn fcw(pc: u32, rc: u32) -> u32 {
    0x7F | (pc << 8) | (rc << 10)
}

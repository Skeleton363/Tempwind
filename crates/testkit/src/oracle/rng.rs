//! A small deterministic PRNG (SplitMix64) so recording is reproducible on
//! every machine and toolchain. No external crate: the sequence must never
//! change underneath the committed fixtures.

/// SplitMix64 (Steele, Lea, Flood 2014).
#[derive(Clone, Debug)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng { state: seed }
    }

    /// A generator for one named stream: `seed` mixed with the FNV-1a hash of
    /// `name`. Each form has its own stream, so adding or changing one form
    /// leaves every other form's cases untouched.
    pub fn for_stream(seed: u64, name: &str) -> Self {
        Rng::new(seed ^ fnv1a(name))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        assert!(n > 0);
        ((u128::from(self.next_u64()) * u128::from(n)) >> 64) as u64
    }

    /// True with probability `num / den`.
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        self.below(den) < num
    }

    /// Uniform in `lo..=hi`.
    pub fn range_i64(&mut self, lo: i64, hi: i64) -> i64 {
        assert!(lo <= hi);
        let span = (hi as i128 - lo as i128 + 1) as u128;
        if span > u128::from(u64::MAX) {
            return self.next_u64() as i64;
        }
        lo.wrapping_add(self.below(span as u64) as i64)
    }

    pub fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}

/// 64-bit FNV-1a.
pub fn fnv1a(text: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix_reference_values() {
        // Reference outputs of SplitMix64 seeded with 0.
        let mut rng = Rng::new(0);
        assert_eq!(rng.next_u64(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(rng.next_u64(), 0x6e78_9e6a_a1b9_65f4);
    }

    #[test]
    fn ranges_stay_in_bounds() {
        let mut rng = Rng::new(1);
        for _ in 0..1000 {
            let v = rng.range_i64(-3, 5);
            assert!((-3..=5).contains(&v));
            assert!(rng.below(7) < 7);
        }
        assert_eq!(fnv1a(""), 0xcbf2_9ce4_8422_2325);
    }
}

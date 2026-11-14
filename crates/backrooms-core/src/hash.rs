//! Stateless, counter-based hashing.
//!
//! Nothing here keeps a running RNG stream. Every random decision in the world
//! is `hash(seed, level, purpose, coordinates)`, so any piece of the world can
//! be computed in isolation, in any order, on any thread, and always comes out
//! the same.
//!
//! Everything is a `const fn`, which means the compiler can evaluate it at
//! compile time (the Z-stack shuffle is built that way) and, more importantly
//! here, that it cannot touch global state, time or the OS.

/// Fixed-point probability or fraction with 16 fractional bits: `65536` is 1.0.
pub type Q16 = u32;

/// 1.0 in [`Q16`].
pub const ONE: Q16 = 1 << 16;

/// Converts a percentage to [`Q16`] at compile time, e.g. `pct(25)` is 0.25.
pub const fn pct(percent: u32) -> Q16 {
    percent * ONE / 100
}

/// 2^64 / golden ratio. Odd, so adding it walks through all 2^64 states.
const GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

/// The SplitMix64 finaliser: a bijection on `u64` with strong avalanche (each
/// input bit flips about half of the output bits).
#[inline]
pub const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A hash in progress. Absorb words with [`Key::with`], then read the result
/// with [`Key::roll`].
///
/// `Key` is `Copy`, so a partially built key (say "level 0, wall segments")
/// can be kept and extended many times without being consumed. That is how the
/// generator avoids re-hashing the common prefix for every coordinate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key(u64);

impl Key {
    #[inline]
    pub const fn new(seed: u64) -> Self {
        Self(mix64(seed.wrapping_add(GAMMA)))
    }

    /// Absorbs one 64-bit word. For a fixed key this is a bijection of `word`,
    /// so two different words can never collide at this step.
    #[inline]
    pub const fn with(self, word: u64) -> Self {
        Self(mix64(self.0.wrapping_add(GAMMA) ^ word))
    }

    /// Absorbs a coordinate pair as a single word.
    ///
    /// `x as u32 as u64` first reinterprets the sign bit (no change of bits),
    /// then zero-extends. Going straight to `u64` would sign-extend and smear
    /// the sign of `x` over the half that belongs to `y`.
    #[inline]
    pub const fn with_xy(self, x: i32, y: i32) -> Self {
        self.with(((x as u32 as u64) << 32) | (y as u32 as u64))
    }

    #[inline]
    pub const fn roll(self) -> Roll {
        Roll(self.0)
    }
}

/// A finished 64-bit hash, with helpers that turn it into decisions.
///
/// [`Roll::chance`] reads the top 16 bits and [`Roll::below`] the low 32, so
/// both may be used on the same roll without correlating. For anything beyond
/// that, call [`Roll::next`] to derive a fresh, independent roll.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Roll(pub u64);

impl Roll {
    /// True with probability `p` (clamped to `0..=ONE`).
    #[inline]
    pub const fn chance(self, p: Q16) -> bool {
        (self.0 >> 48) < p as u64
    }

    /// Uniform integer in `0..n` by multiply-shift (no modulo, so no bias
    /// towards small values beyond 2^-32). `n` must be non-zero.
    #[inline]
    pub const fn below(self, n: u32) -> u32 {
        (((self.0 & 0xFFFF_FFFF) * n as u64) >> 32) as u32
    }

    /// The top 16 bits as a [`Q16`] fraction in `0..ONE`.
    #[inline]
    pub const fn unit(self) -> Q16 {
        (self.0 >> 48) as Q16
    }

    /// An independent roll derived from this one.
    #[inline]
    pub const fn next(self) -> Roll {
        Roll(mix64(self.0.wrapping_add(GAMMA)))
    }
}

/// Linear interpolation between `a` and `b` by `t` in `0..=ONE`.
#[inline]
pub const fn lerp(a: u32, b: u32, t: Q16) -> u32 {
    // i64 so that `b < a` works; `>>` on a signed integer is an arithmetic
    // shift (rounds towards negative infinity) on every platform.
    (a as i64 + (((b as i64 - a as i64) * t as i64) >> 16)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known-answer test: pins the exact bit pattern of the hash so that an
    /// accidental edit to a constant fails loudly here rather than as a
    /// confusing golden-chunk mismatch.
    #[test]
    fn known_answers() {
        // First outputs of the reference SplitMix64 generator seeded with 0.
        assert_eq!(mix64(GAMMA), 0xE220_A839_7B1D_CDAF);
        assert_eq!(mix64(GAMMA.wrapping_mul(2)), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn avalanche() {
        // Flipping any single input bit should flip roughly half the output.
        let key = Key::new(1234);
        let mut total = 0u32;
        let mut samples = 0u32;
        for i in 0..512u64 {
            let base = key.with(i).roll().0;
            for bit in 0..64 {
                total += (base ^ key.with(i ^ (1 << bit)).roll().0).count_ones();
                samples += 1;
            }
        }
        let mean_x100 = total * 100 / samples;
        assert!((3100..=3300).contains(&mean_x100), "mean flipped bits x100 = {mean_x100}");
    }

    #[test]
    fn below_is_uniform_and_in_range() {
        let key = Key::new(99);
        let mut buckets = [0u32; 7];
        for i in 0..70_000u64 {
            buckets[key.with(i).roll().below(7) as usize] += 1;
        }
        for count in buckets {
            assert!((9_500..=10_500).contains(&count), "bucket count {count}");
        }
    }

    #[test]
    fn chance_matches_probability() {
        let key = Key::new(7);
        let hits = (0..100_000u64).filter(|&i| key.with(i).roll().chance(pct(25))).count();
        assert!((24_000..=26_000).contains(&hits), "hits {hits}");
        assert!(!key.roll().chance(0));
        assert!(key.roll().chance(ONE));
    }

    #[test]
    fn xy_packing_keeps_axes_apart() {
        let key = Key::new(0);
        assert_ne!(key.with_xy(-1, 0), key.with_xy(0, -1));
        assert_ne!(key.with_xy(-1, 0), key.with_xy(-1, -1));
        assert_ne!(key.with_xy(1, 2), key.with_xy(2, 1));
    }

    #[test]
    fn lerp_endpoints() {
        assert_eq!(lerp(10, 50, 0), 10);
        assert_eq!(lerp(10, 50, ONE), 50);
        assert_eq!(lerp(50, 10, ONE / 2), 30);
    }
}

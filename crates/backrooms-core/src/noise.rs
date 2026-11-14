//! Coherent noise in pure integer maths.
//!
//! This is value noise: hash a value at every point of a coarse lattice and
//! blend smoothly between the four surrounding lattice points. Layering
//! several lattice sizes gives fractal noise (fBm), and nudging the sample
//! position by another noise field (domain warping) breaks up the grid
//! alignment that plain value noise shows.
//!
//! All values are `Q16` fractions carried in a `u32`, in `0..=65535`.
//!
//! **Torus:** lattice spacings are powers of two, so every lattice tiles the
//! 2^32 world exactly, and lattice coordinates are derived with wrapping
//! arithmetic. The noise is therefore seamless across the world edge with no
//! special handling.

use crate::hash::{Key, Q16, lerp};

/// Smoothstep `3t^2 - 2t^3` for `t` in `0..65536`: an S-curve with zero slope
/// at both ends, so the blend has no visible creases at lattice lines.
#[inline]
const fn smooth(t: Q16) -> Q16 {
    let t = t as u64;
    ((t * t * (3 * 65536 - 2 * t)) >> 32) as Q16
}

/// How far `v` is through its lattice cell, as a `Q16` fraction.
#[inline]
const fn fraction(v: i32, shift: u32) -> Q16 {
    let f = (v as u32) & ((1u32 << shift) - 1);
    if shift <= 16 { f << (16 - shift) } else { f >> (shift - 16) }
}

#[inline]
const fn lattice(key: Key, ix: i32, iy: i32) -> u32 {
    key.with_xy(ix, iy).roll().unit()
}

/// One octave of value noise with a lattice spacing of `2^shift` cells.
pub const fn value(key: Key, x: i32, y: i32, shift: u32) -> u32 {
    debug_assert!(shift <= 30);
    let step = 1i32 << shift;
    let (x0, y0) = (x >> shift, y >> shift);
    // The far lattice line is found by stepping in cell space, where wrapping
    // is exact, rather than by adding 1 to the lattice index.
    let (x1, y1) = (x.wrapping_add(step) >> shift, y.wrapping_add(step) >> shift);
    let (sx, sy) = (smooth(fraction(x, shift)), smooth(fraction(y, shift)));
    let south = lerp(lattice(key, x0, y0), lattice(key, x1, y0), sx);
    let north = lerp(lattice(key, x0, y1), lattice(key, x1, y1), sx);
    lerp(south, north, sy)
}

/// Fractal noise: `octaves` layers, each half the size and half the weight of
/// the one before. Requires `1 <= octaves <= 8` and `octaves <= shift + 1`.
pub const fn fbm(key: Key, x: i32, y: i32, shift: u32, octaves: u32) -> u32 {
    debug_assert!(octaves >= 1 && octaves <= 8 && octaves <= shift + 1);
    let mut sum = 0u32;
    // `for` loops are not allowed in `const fn` yet (they need the Iterator
    // trait, which is not const), hence the manual `while`.
    let mut o = 0;
    while o < octaves {
        sum += value(key.with(o as u64), x, y, shift - o) << (octaves - 1 - o);
        o += 1;
    }
    sum / ((1 << octaves) - 1)
}

/// A named, configured noise field: domain-warped fBm.
#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub key: Key,
    /// Largest feature size is `2^shift` cells.
    pub shift: u32,
    pub octaves: u32,
    /// Feature size of the warp, `2^warp_shift` cells.
    pub warp_shift: u32,
    /// Largest displacement of the warp in cells. Must be below 32768.
    pub warp_cells: i32,
    /// Contrast applied around the midpoint, in 1/256ths (256 = unchanged).
    /// fBm values bunch up around the middle; a gain above 256 spreads them
    /// so that thresholds near 0 and 1 are actually reached.
    pub gain: u32,
}

impl Field {
    pub const fn sample(&self, x: i32, y: i32) -> Q16 {
        // Two extra fBm lookups give a displacement vector for this position.
        let wx = fbm(self.key.with(0x100), x, y, self.warp_shift, 2) as i32 - 32768;
        let wy = fbm(self.key.with(0x101), x, y, self.warp_shift, 2) as i32 - 32768;
        let dx = (wx * self.warp_cells) >> 15;
        let dy = (wy * self.warp_cells) >> 15;
        let n = fbm(self.key, x.wrapping_add(dx), y.wrapping_add(dy), self.shift, self.octaves);
        stretch(n, self.gain)
    }
}

/// Scales `n` away from the midpoint by `gain / 256`, clamped to `0..=65535`.
#[inline]
pub const fn stretch(n: Q16, gain: u32) -> Q16 {
    let v = (((n as i64 - 32768) * gain as i64) >> 8) + 32768;
    if v < 0 {
        0
    } else if v > 65535 {
        65535
    } else {
        v as Q16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: Key = Key::new(42);

    #[test]
    fn hits_lattice_values_exactly() {
        // At a lattice point the fraction is zero, so the result is the hash.
        assert_eq!(value(KEY, 64, -128, 6), lattice(KEY, 1, -2));
    }

    /// Smoothstep's steepest slope is 1.5, so neighbouring cells can differ by
    /// at most 1.5 lattice-value ranges per lattice cell, in each octave.
    fn max_step(shift: u32) -> u32 {
        3 * 65536 / (2 << shift) + 2
    }

    #[test]
    fn continuous_including_across_the_wrap() {
        let shift = 6;
        let limit = max_step(shift);
        // A stretch around the origin and one straddling the world edge.
        for start in [-300i32, i32::MAX - 300] {
            for i in 0..600 {
                let x = start.wrapping_add(i);
                let a = value(KEY, x, 17, shift);
                let b = value(KEY, x.wrapping_add(1), 17, shift);
                assert!(a.abs_diff(b) <= limit, "x jump at {x}: {a} -> {b}");
                let a = value(KEY, 17, x, shift);
                let b = value(KEY, 17, x.wrapping_add(1), shift);
                assert!(a.abs_diff(b) <= limit, "y jump at {x}: {a} -> {b}");
            }
        }
    }

    #[test]
    fn field_is_continuous_across_the_wrap() {
        let field = Field { key: KEY, shift: 7, octaves: 3, warp_shift: 6, warp_cells: 24, gain: 256 };
        // The warp can stretch space, so allow a generous multiple.
        let limit = max_step(5) * 8;
        for i in 0..600 {
            let x = (i32::MAX - 300).wrapping_add(i);
            let a = field.sample(x, x);
            let b = field.sample(x.wrapping_add(1), x);
            assert!(a.abs_diff(b) <= limit, "jump at {x}: {a} -> {b}");
        }
    }

    #[test]
    fn stays_in_range_and_uses_it() {
        let field = Field { key: KEY, shift: 7, octaves: 3, warp_shift: 6, warp_cells: 24, gain: 640 };
        let (mut low, mut high) = (u32::MAX, 0);
        for y in 0..200 {
            for x in 0..200 {
                let n = field.sample(x * 13, y * 13);
                assert!(n <= 65535);
                low = low.min(n);
                high = high.max(n);
            }
        }
        assert!(low < 6_000 && high > 59_000, "range {low}..{high}");
    }

    #[test]
    fn stretch_clamps() {
        assert_eq!(stretch(32768, 1024), 32768);
        assert_eq!(stretch(0, 512), 0);
        assert_eq!(stretch(65535, 512), 65535);
        assert_eq!(stretch(40000, 256), 40000);
    }
}

//! The vertical order of the 128 levels.
//!
//! Depth 0 is the top of the stack and depth 127 the bottom. Which level sits
//! at which depth is a fixed shuffle of the level ids, so the stack order has
//! nothing to do with id order. The shuffle covers all 128 ids, built or not,
//! so building a reserved level later cannot move any other level.
//!
//! The stack does not wrap: nothing is above depth 0 or below depth 127.

use crate::GeneratorVersion;
use crate::generator::v1;
use crate::hash::Key;
use crate::level::{LEVEL_COUNT, LevelId};

/// Fisher-Yates shuffle of `0..128`, with each swap chosen by a stateless
/// hash of its step number instead of by a running RNG.
///
/// Being a `const fn`, this runs inside the compiler when used to initialise
/// a `const`: the table below is baked into the binary and costs nothing at
/// run time.
pub(crate) const fn shuffle(key: Key) -> [u8; LEVEL_COUNT] {
    let mut order = [0u8; LEVEL_COUNT];
    let mut i = 0;
    while i < LEVEL_COUNT {
        order[i] = i as u8;
        i += 1;
    }
    let mut i = LEVEL_COUNT - 1;
    while i > 0 {
        let j = key.with(i as u64).roll().below(i as u32 + 1) as usize;
        order.swap(i, j);
        i -= 1;
    }
    order
}

pub(crate) const fn invert(order: &[u8; LEVEL_COUNT]) -> [u8; LEVEL_COUNT] {
    let mut depth_of = [0u8; LEVEL_COUNT];
    let mut depth = 0;
    while depth < LEVEL_COUNT {
        depth_of[order[depth] as usize] = depth as u8;
        depth += 1;
    }
    depth_of
}

/// The level at `depth` (0 = top), or `None` if `depth` is outside the stack.
pub const fn level_at(version: GeneratorVersion, depth: u8) -> Option<LevelId> {
    if depth as usize >= LEVEL_COUNT {
        return None;
    }
    match version {
        GeneratorVersion::V1 => LevelId::new(v1::Z_ORDER[depth as usize]),
    }
}

/// The depth of `level` in the stack (0 = top).
pub const fn depth_of(version: GeneratorVersion, level: LevelId) -> u8 {
    match version {
        GeneratorVersion::V1 => v1::Z_DEPTH[level.get() as usize],
    }
}

/// The level directly above `level`, or `None` at the top of the stack.
pub const fn above(version: GeneratorVersion, level: LevelId) -> Option<LevelId> {
    match depth_of(version, level) {
        0 => None,
        depth => level_at(version, depth - 1),
    }
}

/// The level directly below `level`, or `None` at the bottom of the stack.
pub const fn below(version: GeneratorVersion, level: LevelId) -> Option<LevelId> {
    level_at(version, depth_of(version, level) + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: GeneratorVersion = GeneratorVersion::V1;

    #[test]
    fn order_is_a_permutation() {
        let mut seen = [false; LEVEL_COUNT];
        for depth in 0..LEVEL_COUNT as u8 {
            let level = level_at(V1, depth).unwrap();
            assert!(!seen[level.get() as usize], "level {level} appears twice");
            seen[level.get() as usize] = true;
            assert_eq!(depth_of(V1, level), depth);
        }
        assert!(level_at(V1, 128).is_none());
    }

    #[test]
    fn order_is_actually_shuffled() {
        let in_place = (0..LEVEL_COUNT as u8).filter(|&d| level_at(V1, d).unwrap().get() == d).count();
        assert!(in_place < 12, "{in_place} levels left at their own id");
    }

    #[test]
    fn neighbours_are_consistent_and_the_stack_ends() {
        let top = level_at(V1, 0).unwrap();
        let bottom = level_at(V1, 127).unwrap();
        assert!(above(V1, top).is_none());
        assert!(below(V1, bottom).is_none());
        for depth in 0..127 {
            let level = level_at(V1, depth).unwrap();
            let under = below(V1, level).unwrap();
            assert_eq!(above(V1, under), Some(level));
        }
    }
}

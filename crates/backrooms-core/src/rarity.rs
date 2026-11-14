//! Rarity tiers for the world's oddities.
//!
//! Every place where something unusual could be (a wall piece, a doorway, a
//! cell) rolls once, and the roll lands in at most one tier. Each tier is
//! eight times rarer than the one before, from one site in 32 down to one in
//! about a million.
//!
//! The generator decides what each tier means at each kind of site. Tiers a
//! generator does not use yet are simply ordinary places for now.

use crate::hash::Roll;

/// How unusual a site is.
///
/// `#[repr(u8)]` with explicit values, because the numbers feed hashes and
/// tests and must never be renumbered.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Rarity {
    /// One site in 32.
    Common = 0,
    /// One in 256.
    Uncommon = 1,
    /// One in 2,048.
    Rare = 2,
    /// One in 16,384.
    Epic = 3,
    /// One in 131,072.
    Legendary = 4,
    /// One in 1,048,576.
    Mythic = 5,
}

impl Rarity {
    pub const ALL: [Rarity; 6] = [Self::Common, Self::Uncommon, Self::Rare, Self::Epic, Self::Legendary, Self::Mythic];

    /// A site is at least this rare with probability `1 / one_in()`.
    pub const fn one_in(self) -> u32 {
        // 2^5, 2^8, 2^11, ... 2^20.
        1 << (5 + 3 * self as u32)
    }

    /// The tier a roll lands in, if any. A roll counts as its rarest tier
    /// only: a Rare site is not also reported as Common.
    pub const fn of(roll: Roll) -> Option<Rarity> {
        // Twenty random bits: a number from 0 to 1,048,575. The smaller it
        // is, the rarer the site.
        let n = (roll.0 >> 44) as u32;
        if n < 1 {
            Some(Self::Mythic)
        } else if n < 1 << 3 {
            Some(Self::Legendary)
        } else if n < 1 << 6 {
            Some(Self::Epic)
        } else if n < 1 << 9 {
            Some(Self::Rare)
        } else if n < 1 << 12 {
            Some(Self::Uncommon)
        } else if n < 1 << 15 {
            Some(Self::Common)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::Key;

    #[test]
    fn each_tier_is_eight_times_rarer() {
        assert_eq!(Rarity::Common.one_in(), 32);
        assert_eq!(Rarity::Mythic.one_in(), 1_048_576);
        for pair in Rarity::ALL.windows(2) {
            assert_eq!(pair[1].one_in(), pair[0].one_in() * 8);
        }
    }

    #[test]
    fn boundaries_fall_where_documented() {
        let at = |n: u64| Rarity::of(Roll(n << 44));
        assert_eq!(at(0), Some(Rarity::Mythic));
        assert_eq!(at(1), Some(Rarity::Legendary));
        assert_eq!(at(7), Some(Rarity::Legendary));
        assert_eq!(at(8), Some(Rarity::Epic));
        assert_eq!(at(511), Some(Rarity::Rare));
        assert_eq!(at(4095), Some(Rarity::Uncommon));
        assert_eq!(at(32767), Some(Rarity::Common));
        assert_eq!(at(32768), None);
        assert_eq!(at((1 << 20) - 1), None);
    }

    #[test]
    fn observed_frequencies_match() {
        let key = Key::new(0xA11CE);
        let mut counts = [0u32; 6];
        let sites = 2_000_000u64;
        for i in 0..sites {
            if let Some(tier) = Rarity::of(key.with(i).roll()) {
                counts[tier as usize] += 1;
            }
        }
        // "At least this rare" should be sites / one_in, within sampling noise.
        for tier in [Rarity::Common, Rarity::Uncommon, Rarity::Rare, Rarity::Epic] {
            let at_least: u32 = counts[tier as usize..].iter().sum();
            let expected = (sites / tier.one_in() as u64) as u32;
            let tolerance = expected / 5 + 10;
            assert!(at_least.abs_diff(expected) <= tolerance, "{tier:?}: {at_least} vs {expected}");
        }
    }
}

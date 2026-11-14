//! The Backrooms as a pure function of its coordinates.
//!
//! ```text
//! world(version, level, x, y) -> walls, surfaces, lights, ...
//! ```
//!
//! Nothing is stored and nothing is random at run time: the same address
//! yields the same place on every machine, forever.
//!
//! # Rules of this crate
//!
//! * **Integers only.** No `f32`/`f64` anywhere; fractions are fixed-point
//!   ([`hash::Q16`]). A test scans the source to enforce it.
//! * **No state.** Every random choice is a hash of `(seed, level, purpose,
//!   coordinates)`. There is no RNG stream to advance, so results cannot
//!   depend on generation order, thread scheduling, time or hardware.
//! * **Frozen versions.** A [`GeneratorVersion`] that has shipped never
//!   changes its output. New behaviour means a new version, and old versions
//!   stay runnable.
//! * **No engine types**, no dependencies, no `unsafe`, and `no_std`: the
//!   crate uses only `core`, so it runs anywhere and cannot reach the clock,
//!   the file system or OS randomness even by accident.

#![no_std]

pub mod chunk;
pub mod coord;
pub mod generator;
pub mod hash;
pub mod level;
pub mod noise;
pub mod rarity;
pub mod zstack;

pub use chunk::{Cell, Chunk, Dir, Edge, Fixture, LightState, Skew};
pub use coord::{Address, AddressError, CellPos, ChunkPos, RegionPos};
pub use generator::{GenError, generate_chunk, level_info};
pub use level::{LEVEL_COUNT, LevelGroup, LevelId, LevelInfo};
pub use rarity::Rarity;

/// Which generator produced (or should produce) a piece of world.
///
/// Adding a version means adding a variant here. Every `match` on this enum
/// in the crate then stops compiling until the new version is handled, which
/// is exactly the reminder wanted.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum GeneratorVersion {
    V1 = 1,
}

impl GeneratorVersion {
    pub const LATEST: Self = Self::V1;

    #[inline]
    pub const fn number(self) -> u8 {
        self as u8
    }

    pub const fn from_number(n: u8) -> Option<Self> {
        match n {
            1 => Some(Self::V1),
            _ => None,
        }
    }
}

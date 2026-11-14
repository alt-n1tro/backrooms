//! Entry points of the world function, dispatching on generator version.
//!
//! Each version lives in its own module and shares only the version-neutral
//! building blocks (`hash`, `noise`, `coord`, `chunk`). Those building blocks
//! are covered by the same freeze: changing what they compute changes every
//! version built on them.
//!
//! (The module is called `generator` rather than `gen` because `gen` is a
//! reserved keyword from the 2024 edition on.)

use core::fmt;

use crate::GeneratorVersion;
use crate::chunk::Chunk;
use crate::coord::{CellPos, ChunkPos};
use crate::level::{LevelId, LevelInfo};

pub mod v1;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GenError {
    /// The level id is reserved but has no generator in this version.
    LevelNotBuilt(LevelId),
}

impl fmt::Display for GenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LevelNotBuilt(level) => write!(f, "level {level} is reserved but not built yet"),
        }
    }
}

impl core::error::Error for GenError {}

/// Generates one chunk. Pure: same arguments, same result, always.
pub fn generate_chunk(version: GeneratorVersion, level: LevelId, pos: ChunkPos) -> Result<Chunk, GenError> {
    match version {
        GeneratorVersion::V1 => v1::generate_chunk(level, pos),
    }
}

pub fn level_info(version: GeneratorVersion, level: LevelId) -> LevelInfo {
    match version {
        GeneratorVersion::V1 => v1::level_info(level),
    }
}

/// The far end of the portal whose doorway is on the west or south side of
/// `cell`: the cell in the same position on the same side of the paired
/// doorway. Applying it twice comes back to `cell`.
pub fn portal_partner(version: GeneratorVersion, level: LevelId, cell: CellPos) -> CellPos {
    match version {
        GeneratorVersion::V1 => v1::portal_partner(level, cell),
    }
}

/// Where passing through the phasing wall piece at `cell` lands: any built
/// level, anywhere on it. The caller moves the arrival to the nearest open
/// floor.
pub fn noclip_exit(version: GeneratorVersion, level: LevelId, cell: CellPos) -> (LevelId, CellPos) {
    match version {
        GeneratorVersion::V1 => v1::noclip_exit(level, cell),
    }
}

/// The level a stairwell on `level` leads to: the nearest built level up
/// or down the stack, or `None` if there is none that way.
pub fn stair_exit(version: GeneratorVersion, level: LevelId, up: bool) -> Option<LevelId> {
    match version {
        GeneratorVersion::V1 => v1::stair_exit(level, up),
    }
}

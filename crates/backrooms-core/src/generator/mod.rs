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
use crate::coord::ChunkPos;
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

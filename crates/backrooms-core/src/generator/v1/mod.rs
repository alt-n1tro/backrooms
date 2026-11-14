//! Generator version 1.
//!
//! **Status: draft.** v1 has not shipped, so its output may still change
//! between milestones; each change re-blesses the golden hashes on purpose.
//! From the first public release on, this module is frozen.

use crate::GeneratorVersion;
use crate::chunk::Chunk;
use crate::coord::ChunkPos;
use crate::generator::GenError;
use crate::hash::Key;
use crate::level::{LEVEL_COUNT, LevelId, LevelInfo};
use crate::zstack;

pub mod golden;
pub mod level0;

/// Root seed of every hash in v1 (the ASCII bytes of "BACKRMS1").
pub const WORLD_SEED: u64 = 0x4241_434B_524D_5331;

/// What a hash is *for*. Mixing the purpose into the key keeps unrelated
/// decisions at the same coordinates independent of each other.
///
/// The numbers are part of the world function: never change or reuse one.
pub mod purpose {
    pub const Z_SHUFFLE: u64 = 1;
    pub const DENSITY: u64 = 2;
    pub const WALL: u64 = 3;
    pub const CARVE: u64 = 4;
    pub const PILLAR: u64 = 5;
    pub const FIXTURE: u64 = 6;
    pub const POWER: u64 = 7;
    pub const FLICKER: u64 = 8;
    pub const DAMP: u64 = 9;
    pub const WEAR: u64 = 10;
    pub const REGION_LIGHTING: u64 = 11;
}

/// Key for a world-wide decision.
pub const fn world_key(purpose: u64) -> Key {
    Key::new(WORLD_SEED).with(purpose)
}

/// Key for a decision on one level. Bit 63 marks level keys, so they can
/// never coincide with a world key.
pub const fn level_key(level: LevelId, purpose: u64) -> Key {
    Key::new(WORLD_SEED).with(purpose | (level.get() as u64) << 32 | 1 << 63)
}

/// `Z_ORDER[depth]` is the id of the level at that depth (0 = top).
pub const Z_ORDER: [u8; LEVEL_COUNT] = zstack::shuffle(world_key(purpose::Z_SHUFFLE));
/// `Z_DEPTH[id]` is the depth of that level.
pub const Z_DEPTH: [u8; LEVEL_COUNT] = zstack::invert(&Z_ORDER);

/// How a level is generated in v1.
enum Profile {
    Reserved,
    Level0 { name: &'static str, params: &'static level0::Params },
}

const fn profile(level: LevelId) -> Profile {
    match level.get() {
        0 => Profile::Level0 { name: "0", params: &level0::LEVEL_0 },
        _ => Profile::Reserved,
    }
}

pub fn level_info(level: LevelId) -> LevelInfo {
    let (name, built, ceiling_mm) = match profile(level) {
        Profile::Reserved => ("", false, 0),
        Profile::Level0 { name, params } => (name, true, params.ceiling_mm),
    };
    LevelInfo { id: level, group: level.group(), name, built, ceiling_mm }
}

pub fn generate_chunk(level: LevelId, pos: ChunkPos) -> Result<Chunk, GenError> {
    match profile(level) {
        Profile::Reserved => Err(GenError::LevelNotBuilt(level)),
        Profile::Level0 { params, .. } => Ok(level0::generate(GeneratorVersion::V1, params, level, pos)),
    }
}

//! The fixed set of chunks whose hashes pin down generator v1.
//!
//! The golden test compares [`table`] with a file checked into the
//! repository. If they differ, the world changed. The same table must come
//! out on every platform.

use core::fmt;

use crate::GeneratorVersion;
use crate::coord::ChunkPos;
use crate::generator::generate_chunk;
use crate::hash::Key;
use crate::level::LevelId;

const MIN: i32 = ChunkPos::MIN;
const MAX: i32 = ChunkPos::MAX;

/// Individually listed chunks: the origin and its neighbours, all four world
/// corners and edges (where wrapping bugs would show), and scattered far-off
/// places. Written out as literals so the set itself can never drift.
pub const CHUNKS: [(i32, i32); 32] = [
    (0, 0),
    (1, 0),
    (0, 1),
    (-1, 0),
    (0, -1),
    (-1, -1),
    (MIN, MIN),
    (MAX, MAX),
    (MIN, MAX),
    (MAX, MIN),
    (MIN, 0),
    (MAX, 0),
    (0, MIN),
    (0, MAX),
    (31, 31),
    (32, 32),
    (-32, 31),
    (1_000, -1_000),
    (65_535, 65_536),
    (-65_536, 65_535),
    (1_234_567, -7_654_321),
    (-9_999_999, 9_999_999),
    (48_271, 16_807),
    (100_000_000, 100_000_000),
    (-100_000_000, 100_000_000),
    (134_217_000, -134_217_000),
    (77_777_777, -33_333_333),
    (-2, 134_217_727),
    (3, -134_217_728),
    (8_388_607, 8_388_608),
    (-8_388_609, -8_388_608),
    (20_011, -40_009),
];

/// Square blocks hashed as a whole, to cover far more ground than the list
/// above: `(south-west chunk x, y, side)`. One sits at the origin, one
/// straddles the world corner where both axes wrap, and one is far out.
pub const BLOCKS: [(i32, i32, i32); 3] = [(-24, -24, 48), (MAX - 23, MAX - 23, 48), (40_000_000, -90_000_000, 48)];

/// The levels the golden set covers.
pub const LEVELS: [LevelId; 1] = [LevelId::ZERO];

fn chunk_digest(level: LevelId, pos: ChunkPos) -> u64 {
    generate_chunk(GeneratorVersion::V1, level, pos).expect("golden levels are built").digest()
}

/// Combined digest of a `side` x `side` block of chunks.
pub fn block_digest(level: LevelId, x: i32, y: i32, side: i32) -> u64 {
    let south_west = ChunkPos::new(x, y);
    let mut key = Key::new(0xB10C);
    for dy in 0..side {
        for dx in 0..side {
            key = key.with(chunk_digest(level, south_west.offset(dx, dy)));
        }
    }
    key.roll().0
}

/// The golden table. It implements `Display`, so it can be printed or turned
/// into a `String` by code that has an allocator, while this crate itself
/// stays allocation-free.
pub struct Table;

pub fn table() -> Table {
    Table
}

impl fmt::Display for Table {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "# Golden hashes for generator v1. One line per chunk or block.")?;
        writeln!(f, "# Regenerate with: BACKROOMS_BLESS=1 cargo test -p backrooms-core --test golden")?;
        for level in LEVELS {
            for (x, y) in CHUNKS {
                let pos = ChunkPos::new(x, y);
                writeln!(f, "v1 level {level} chunk {x} {y} {:016x}", chunk_digest(level, pos))?;
            }
            for (x, y, side) in BLOCKS {
                writeln!(f, "v1 level {level} block {x} {y} {side} {:016x}", block_digest(level, x, y, side))?;
            }
        }
        Ok(())
    }
}

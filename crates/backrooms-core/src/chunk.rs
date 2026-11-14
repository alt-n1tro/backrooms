//! The generated contents of one 16 x 16 cell chunk.
//!
//! A [`Chunk`] is plain data in fixed-size arrays: no heap allocation, no
//! pointers. It can be generated on any thread, thrown away at any time and
//! regenerated bit-for-bit.
//!
//! **Walls live on edges, not in cells.** Each cell owns the edge on its west
//! side and the edge on its south side. A chunk stores one extra column and
//! one extra row of edges so that it also holds its own east and north
//! boundary. Those boundary edges are computed from the same global edge
//! coordinates as the neighbouring chunk's west and south edges, so the two
//! chunks always agree on the seam.

use crate::GeneratorVersion;
use crate::coord::{CHUNK_SIZE, ChunkPos};
use crate::hash::Key;
use crate::level::LevelId;

/// Cells along one side of a chunk, as an array length.
pub const N: usize = CHUNK_SIZE as usize;

/// What stands on the boundary between two cells.
///
/// `#[repr(u8)]` fixes the in-memory size and the numeric value of each
/// variant. The values are part of the golden-hash encoding, so they are
/// written out explicitly and must never be renumbered.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Edge {
    #[default]
    Open = 0,
    Wall = 1,
    /// A wall with an unglazed opening at chest height: you can see and
    /// hear through it, but not walk through.
    Window = 2,
    /// A wall with a low hole at floor level, passable only on hands and
    /// knees.
    Crawl = 3,
    /// A doorway filled with darkness. Walking through it comes out of
    /// another doorway far away on the same level.
    Portal = 4,
    /// A piece of wall that looks like any other and is not there. Walking
    /// through it leaves the level.
    Phase = 5,
}

impl Edge {
    /// Height of the gap a body can pass through, in millimetres from the
    /// floor, given the ceiling height. Zero means no way through.
    pub const fn clearance_mm(self, ceiling_mm: u16) -> u16 {
        match self {
            Self::Open => ceiling_mm,
            Self::Wall | Self::Window => 0,
            Self::Crawl => CRAWL_HEIGHT_MM,
            Self::Portal => PORTAL_HEIGHT_MM,
            Self::Phase => ceiling_mm,
        }
    }
}

/// Height of a portal's doorway.
pub const PORTAL_HEIGHT_MM: u16 = 2100;
/// Height of a crawl hole.
pub const CRAWL_HEIGHT_MM: u16 = 700;
/// A window's opening runs from its sill to its head.
pub const WINDOW_SILL_MM: u16 = 900;
pub const WINDOW_HEAD_MM: u16 = 2000;

/// How far a point of the 2 m wall lattice is pushed out of true, in
/// centimetres. Almost always zero; where it is not, the walls meeting at
/// that point are no longer square to each other.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Skew {
    /// East is positive.
    pub dx: i8,
    /// North is positive.
    pub dy: i8,
}

/// Lattice points along one side of a chunk, counting both ends.
pub const LATTICE: usize = N / 4 + 1;

/// One floor tile.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Cell {
    /// Filled floor to ceiling (a pillar). All four of its edges are walls.
    pub solid: bool,
    /// Moisture, 0 = dry to 255 = soaked. Low-frequency only: the renderer
    /// adds the fine stain detail on top.
    pub damp: u8,
    /// Age and damage, 0 = pristine to 255 = ruined.
    pub wear: u8,
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum LightState {
    #[default]
    On = 0,
    Dead = 1,
    Flicker = 2,
}

/// A ceiling light: a 1.0 m x 0.5 m panel covering cell `(x, y)` and the cell
/// east of it, both in chunk-local coordinates.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Fixture {
    pub x: u8,
    pub y: u8,
    pub state: LightState,
    /// Per-fixture random bits for the renderer and audio: flicker pattern,
    /// phase, slight colour and hum differences.
    pub seed: u32,
}

/// Most fixtures a chunk can hold.
pub const MAX_FIXTURES: usize = 8;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Chunk {
    pub version: GeneratorVersion,
    pub level: LevelId,
    pub pos: ChunkPos,
    /// `cells[y][x]`.
    pub cells: [[Cell; N]; N],
    /// `west[y][x]` is the edge on the west side of cell `(x, y)`. Column
    /// `x == 16` is the chunk's east boundary.
    pub west: [[Edge; N + 1]; N],
    /// `south[y][x]` is the edge on the south side of cell `(x, y)`. Row
    /// `y == 16` is the chunk's north boundary.
    pub south: [[Edge; N]; N + 1],
    /// `skew[j][i]` belongs to the lattice point at cell corner
    /// `(4 * i, 4 * j)`. Index 4 is the chunk's far boundary, shared with
    /// the neighbouring chunk's index 0.
    pub skew: [[Skew; LATTICE]; LATTICE],
    /// The stairwell in this chunk, if it has one. They are rare enough
    /// that a chunk never needs room for two.
    pub stair: Option<Stair>,
    // A fixed array plus a length is a heap-free stand-in for `Vec`. The two
    // fields are private so the length can never disagree with the contents.
    fixtures: [Fixture; MAX_FIXTURES],
    fixture_count: u8,
}

impl Chunk {
    /// A chunk of bare, dry, unlit floor with no walls.
    pub fn empty(version: GeneratorVersion, level: LevelId, pos: ChunkPos) -> Self {
        Self {
            version,
            level,
            pos,
            cells: [[Cell::default(); N]; N],
            west: [[Edge::Open; N + 1]; N],
            south: [[Edge::Open; N]; N + 1],
            skew: [[Skew::default(); LATTICE]; LATTICE],
            stair: None,
            fixtures: [Fixture::default(); MAX_FIXTURES],
            fixture_count: 0,
        }
    }

    /// The ceiling lights, in a fixed order (south to north, west to east).
    pub fn fixtures(&self) -> &[Fixture] {
        &self.fixtures[..self.fixture_count as usize]
    }

    /// Adds a fixture. Returns false (and adds nothing) if the chunk is full.
    pub fn push_fixture(&mut self, fixture: Fixture) -> bool {
        let Some(slot) = self.fixtures.get_mut(self.fixture_count as usize) else {
            return false;
        };
        *slot = fixture;
        self.fixture_count += 1;
        true
    }

    /// The state of the light over local cell `(x, y)`, if a fixture covers it.
    pub fn light_at(&self, x: usize, y: usize) -> Option<LightState> {
        self.fixtures()
            .iter()
            .find(|f| f.y as usize == y && (f.x as usize == x || f.x as usize + 1 == x))
            .map(|f| f.state)
    }

    /// Whether the player can step from local cell `(x, y)` one cell in the
    /// given direction, as far as this chunk's walls are concerned.
    pub fn open_towards(&self, x: usize, y: usize, dir: Dir) -> bool {
        let edge = match dir {
            Dir::West => self.west[y][x],
            Dir::East => self.west[y][x + 1],
            Dir::South => self.south[y][x],
            Dir::North => self.south[y + 1][x],
        };
        edge == Edge::Open
    }

    /// A 64-bit fingerprint of everything in the chunk, for the golden tests.
    ///
    /// Every field is fed in as an explicit integer in a fixed order. The
    /// struct's raw memory is never hashed, because padding, field order and
    /// byte order are not guaranteed to match between platforms.
    pub fn digest(&self) -> u64 {
        let mut key = Key::new(0xD16E_5700)
            .with(self.version.number() as u64)
            .with(self.level.get() as u64)
            .with_xy(self.pos.x(), self.pos.y());
        for cell in self.cells.as_flattened() {
            key = key.with(cell.solid as u64 | (cell.damp as u64) << 8 | (cell.wear as u64) << 16);
        }
        for &edge in self.west.as_flattened() {
            key = key.with(edge as u64);
        }
        for &edge in self.south.as_flattened() {
            key = key.with(edge as u64);
        }
        for skew in self.skew.as_flattened() {
            // `as u8` keeps the bit pattern of a negative number, so each
            // value fills exactly one byte of the word.
            key = key.with(skew.dx as u8 as u64 | (skew.dy as u8 as u64) << 8);
        }
        // 0 for no stairwell; otherwise every field, offset so that a
        // stairwell at 0,0 facing north going down is not also 0.
        key =
            key.with(self.stair.map_or(0, |s| {
                1 | (s.x as u64) << 8 | (s.y as u64) << 16 | (s.facing as u64) << 24 | (s.up as u64) << 32
            }));
        key = key.with(self.fixture_count as u64);
        for f in self.fixtures() {
            key = key.with(f.x as u64 | (f.y as u64) << 8 | (f.state as u64) << 16).with(f.seed as u64);
        }
        key.roll().0
    }
}

/// A compass direction. +x is east, +y is north.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Dir {
    North = 0,
    East = 1,
    South = 2,
    West = 3,
}

impl Dir {
    /// One cell's step in this direction, as `(dx, dy)`.
    pub const fn step(self) -> (i32, i32) {
        match self {
            Self::North => (0, 1),
            Self::East => (1, 0),
            Self::South => (0, -1),
            Self::West => (-1, 0),
        }
    }
}

/// A stairwell: a 2 m square booth, walled on three sides, with a flight of
/// steps leading to the next level up or down the stack.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Stair {
    /// South-west cell of the booth, in chunk-local coordinates. Always a
    /// multiple of four.
    pub x: u8,
    pub y: u8,
    /// The open side, where you step on.
    pub facing: Dir,
    /// True if the steps go up.
    pub up: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> Chunk {
        Chunk::empty(GeneratorVersion::V1, LevelId::ZERO, ChunkPos::new(3, -4))
    }

    #[test]
    fn fixtures_are_bounded() {
        let mut chunk = empty();
        for i in 0..MAX_FIXTURES as u8 {
            assert!(chunk.push_fixture(Fixture { x: i, ..Fixture::default() }));
        }
        assert!(!chunk.push_fixture(Fixture::default()));
        assert_eq!(chunk.fixtures().len(), MAX_FIXTURES);
    }

    #[test]
    fn digest_sees_every_kind_of_field() {
        let base = empty();
        let mut variants = [base.clone(), base.clone(), base.clone(), base.clone(), base.clone(), base.clone()];
        variants[5].skew[4][4].dy = -1;
        variants[0].cells[15][15].wear = 1;
        variants[1].west[15][16] = Edge::Wall;
        variants[2].south[16][15] = Edge::Wall;
        variants[3].push_fixture(Fixture::default());
        variants[4].pos = ChunkPos::new(3, -5);
        for (i, a) in variants.iter().enumerate() {
            assert_ne!(a.digest(), base.digest(), "variant {i} not seen by digest");
            for b in &variants[i + 1..] {
                assert_ne!(a.digest(), b.digest());
            }
        }
    }

    #[test]
    fn only_some_edges_can_be_passed() {
        assert_eq!(Edge::Open.clearance_mm(2600), 2600);
        assert_eq!(Edge::Wall.clearance_mm(2600), 0);
        assert_eq!(Edge::Window.clearance_mm(2600), 0);
        assert_eq!(Edge::Crawl.clearance_mm(2600), CRAWL_HEIGHT_MM);
        assert_eq!(Edge::Portal.clearance_mm(2600), PORTAL_HEIGHT_MM);
        assert_eq!(Edge::Phase.clearance_mm(2600), 2600);
    }

    #[test]
    fn open_towards_reads_the_right_edge() {
        let mut chunk = empty();
        chunk.west[2][5] = Edge::Wall;
        chunk.south[2][5] = Edge::Wall;
        assert!(!chunk.open_towards(5, 2, Dir::West));
        assert!(!chunk.open_towards(4, 2, Dir::East));
        assert!(!chunk.open_towards(5, 2, Dir::South));
        assert!(!chunk.open_towards(5, 1, Dir::North));
        assert!(chunk.open_towards(5, 2, Dir::North));
        assert!(chunk.open_towards(5, 2, Dir::East));
    }
}

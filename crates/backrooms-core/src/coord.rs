//! Coordinates on the torus.
//!
//! A level is a 2^32 x 2^32 grid of cells that wraps at its edges. A signed
//! 32-bit integer with wrapping arithmetic *is* that torus: walking east from
//! `i32::MAX` lands on `i32::MIN` with no special case. So the rule for this
//! crate is simple: positions are only ever combined with `wrapping_*`
//! operations, never with plain `+`/`-`.
//!
//! Compass convention: +x is east, +y is north.
//!
//! Hierarchy (each size is a power of two, so each tiles the torus exactly):
//!
//! | Unit   | Size in cells | Role                                     |
//! |--------|---------------|------------------------------------------|
//! | Cell   | 1 (0.5 m)     | floor tile                               |
//! | Chunk  | 16            | unit of generation, meshing and caching  |
//! | Region | 512           | zones, lighting regimes, rarity rolls    |

use core::fmt;
use core::str::FromStr;

use crate::GeneratorVersion;
use crate::level::LevelId;

/// Edge length of a cell in millimetres.
pub const CELL_MM: u32 = 500;

pub const CHUNK_SHIFT: u32 = 4;
/// Cells along one side of a chunk.
pub const CHUNK_SIZE: i32 = 1 << CHUNK_SHIFT;

pub const REGION_SHIFT: u32 = 9;
/// Cells along one side of a region.
pub const REGION_SIZE: i32 = 1 << REGION_SHIFT;

/// A cell on a level. Every `i32` pair is a valid cell.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct CellPos {
    pub x: i32,
    pub y: i32,
}

impl CellPos {
    pub const ORIGIN: Self = Self::new(0, 0);

    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// The cell `dx` east and `dy` north of this one, wrapping around the torus.
    #[inline]
    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x.wrapping_add(dx), self.y.wrapping_add(dy))
    }

    /// The chunk containing this cell.
    ///
    /// `>>` on a signed integer is an arithmetic shift: it rounds towards
    /// negative infinity, so cell -1 is in chunk -1 (not chunk 0, which is
    /// what `/ 16` would give).
    #[inline]
    pub const fn chunk(self) -> ChunkPos {
        ChunkPos { x: self.x >> CHUNK_SHIFT, y: self.y >> CHUNK_SHIFT }
    }

    /// Position of this cell inside its chunk, each in `0..16`.
    #[inline]
    pub const fn local(self) -> (usize, usize) {
        ((self.x & (CHUNK_SIZE - 1)) as usize, (self.y & (CHUNK_SIZE - 1)) as usize)
    }

    /// The region containing this cell.
    #[inline]
    pub const fn region(self) -> RegionPos {
        RegionPos { x: self.x >> REGION_SHIFT, y: self.y >> REGION_SHIFT }
    }

    /// Shortest displacement from `self` to `other` on the torus.
    #[inline]
    pub const fn delta_to(self, other: Self) -> (i32, i32) {
        (other.x.wrapping_sub(self.x), other.y.wrapping_sub(self.y))
    }
}

/// Sign-extends the low `bits` bits of `v`, i.e. wraps `v` into the signed
/// range of a `bits`-bit integer. Shifting bits out of the top of an integer
/// is not an overflow in Rust; only a shift *amount* >= 32 is.
#[inline]
const fn wrap_bits(v: i32, bits: u32) -> i32 {
    (v << (32 - bits)) >> (32 - bits)
}

/// A chunk on a level. There are 2^28 chunks per axis.
///
/// The fields are private so that a `ChunkPos` is always in canonical range;
/// two values that name the same chunk are then always `==`. This is the
/// "make invalid states unrepresentable" pattern.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct ChunkPos {
    x: i32,
    y: i32,
}

impl ChunkPos {
    pub const BITS: u32 = 32 - CHUNK_SHIFT;
    /// Smallest chunk coordinate. The largest is `-MIN - 1`.
    pub const MIN: i32 = -(1 << (Self::BITS - 1));
    pub const MAX: i32 = -Self::MIN - 1;

    /// Any `i32` pair is accepted and wrapped onto the torus.
    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x: wrap_bits(x, Self::BITS), y: wrap_bits(y, Self::BITS) }
    }

    #[inline]
    pub const fn x(self) -> i32 {
        self.x
    }

    #[inline]
    pub const fn y(self) -> i32 {
        self.y
    }

    /// The south-west cell of the chunk.
    #[inline]
    pub const fn origin(self) -> CellPos {
        CellPos::new(self.x << CHUNK_SHIFT, self.y << CHUNK_SHIFT)
    }

    #[inline]
    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x.wrapping_add(dx), self.y.wrapping_add(dy))
    }
}

/// A region on a level. There are 2^23 regions per axis.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct RegionPos {
    x: i32,
    y: i32,
}

impl RegionPos {
    pub const BITS: u32 = 32 - REGION_SHIFT;

    #[inline]
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x: wrap_bits(x, Self::BITS), y: wrap_bits(y, Self::BITS) }
    }

    #[inline]
    pub const fn x(self) -> i32 {
        self.x
    }

    #[inline]
    pub const fn y(self) -> i32 {
        self.y
    }

    #[inline]
    pub const fn origin(self) -> CellPos {
        CellPos::new(self.x << REGION_SHIFT, self.y << REGION_SHIFT)
    }

    #[inline]
    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        Self::new(self.x.wrapping_add(dx), self.y.wrapping_add(dy))
    }
}

/// A complete, shareable location: `backrooms://v1/<level>/<x>/<y>`.
///
/// The generator version is part of the address because the same coordinates
/// may hold a different place under a different generator.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Address {
    pub version: GeneratorVersion,
    pub level: LevelId,
    pub cell: CellPos,
}

impl Address {
    pub const SCHEME: &'static str = "backrooms://";
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}/{}/{}/{}", Self::SCHEME, self.version.number(), self.level.get(), self.cell.x, self.cell.y)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AddressError {
    /// Does not start with `backrooms://`.
    Scheme,
    /// Not exactly `v<n>/<level>/<x>/<y>` after the scheme.
    Shape,
    UnknownVersion,
    /// Level is not a number in `0..128`.
    Level,
    /// x or y is not a signed 32-bit integer.
    Coordinate,
}

impl fmt::Display for AddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Scheme => "address must start with backrooms://",
            Self::Shape => "address must look like backrooms://v1/<level>/<x>/<y>",
            Self::UnknownVersion => "unknown generator version",
            Self::Level => "level must be a number from 0 to 127",
            Self::Coordinate => "x and y must be whole numbers from -2147483648 to 2147483647",
        })
    }
}

impl core::error::Error for AddressError {}

/// Implementing `FromStr` is what makes `"backrooms://...".parse::<Address>()`
/// work, and lets argument parsers accept an `Address` directly.
impl FromStr for Address {
    type Err = AddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let rest = s.trim().strip_prefix(Self::SCHEME).ok_or(AddressError::Scheme)?;
        let mut parts = rest.split('/');
        // `?` on an Option inside a function returning Result needs the
        // explicit `ok_or`: it says which error a missing part becomes.
        let mut next = || parts.next().ok_or(AddressError::Shape);

        let version = next()?
            .strip_prefix('v')
            .and_then(|n| n.parse::<u8>().ok())
            .and_then(GeneratorVersion::from_number)
            .ok_or(AddressError::UnknownVersion)?;
        let level = next()?.parse::<u8>().ok().and_then(LevelId::new).ok_or(AddressError::Level)?;
        let x = next()?.parse::<i32>().map_err(|_| AddressError::Coordinate)?;
        let y = next()?.parse::<i32>().map_err(|_| AddressError::Coordinate)?;
        if parts.next().is_some() {
            return Err(AddressError::Shape);
        }
        Ok(Self { version, level, cell: CellPos::new(x, y) })
    }
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::string::ToString;

    use super::*;

    #[test]
    fn cells_wrap_at_the_edge() {
        let east_edge = CellPos::new(i32::MAX, 0);
        assert_eq!(east_edge.offset(1, 0), CellPos::new(i32::MIN, 0));
        assert_eq!(CellPos::new(0, i32::MIN).offset(0, -1), CellPos::new(0, i32::MAX));
        assert_eq!(east_edge.delta_to(CellPos::new(i32::MIN, 0)), (1, 0));
    }

    #[test]
    fn negative_cells_round_down() {
        assert_eq!(CellPos::new(-1, -16).chunk(), ChunkPos::new(-1, -1));
        assert_eq!(CellPos::new(-17, 15).chunk(), ChunkPos::new(-2, 0));
        assert_eq!(CellPos::new(-1, -16).local(), (15, 0));
        assert_eq!(CellPos::new(-1, 511).region(), RegionPos::new(-1, 0));
    }

    #[test]
    fn chunks_wrap_at_the_edge() {
        let last = CellPos::new(i32::MAX, i32::MAX).chunk();
        assert_eq!((last.x(), last.y()), (ChunkPos::MAX, ChunkPos::MAX));
        assert_eq!(last.offset(1, 1), CellPos::new(i32::MIN, i32::MIN).chunk());
        assert_eq!(last.offset(1, 0).x(), ChunkPos::MIN);
        assert_eq!(ChunkPos::new(ChunkPos::MAX + 1, 0), ChunkPos::new(ChunkPos::MIN, 0));
        assert_eq!(last.origin(), CellPos::new(i32::MAX - 15, i32::MAX - 15));
    }

    #[test]
    fn chunk_and_origin_round_trip() {
        for &(x, y) in &[(0, 0), (-1, 5), (ChunkPos::MIN, ChunkPos::MAX), (123_456, -98_765)] {
            let chunk = ChunkPos::new(x, y);
            assert_eq!(chunk.origin().chunk(), chunk);
            assert_eq!(chunk.origin().offset(15, 15).chunk(), chunk);
            assert_eq!(chunk.origin().offset(16, 0).chunk(), chunk.offset(1, 0));
        }
    }

    #[test]
    fn address_round_trip() {
        let text = "backrooms://v1/0/-2147483648/2147483647";
        let address: Address = text.parse().unwrap();
        assert_eq!(address.cell, CellPos::new(i32::MIN, i32::MAX));
        assert_eq!(address.to_string(), text);
    }

    #[test]
    fn address_errors() {
        let parse = |s: &str| s.parse::<Address>();
        assert_eq!(parse("http://v1/0/0/0"), Err(AddressError::Scheme));
        assert_eq!(parse("backrooms://v9/0/0/0"), Err(AddressError::UnknownVersion));
        assert_eq!(parse("backrooms://v1/128/0/0"), Err(AddressError::Level));
        assert_eq!(parse("backrooms://v1/0/2147483648/0"), Err(AddressError::Coordinate));
        assert_eq!(parse("backrooms://v1/0/0"), Err(AddressError::Shape));
        assert_eq!(parse("backrooms://v1/0/0/0/0"), Err(AddressError::Shape));
    }
}

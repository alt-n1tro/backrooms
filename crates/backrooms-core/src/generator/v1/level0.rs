//! The yellow rooms.
//!
//! # Layout
//!
//! Walls stand only on the lines of a 2 m lattice (every 4 cells), which
//! divides the floor into 2 m x 2 m **bays**. A piece of lattice line between
//! two neighbouring lattice points is a **segment**: 4 cell edges long.
//!
//! Three nested lattices decide where walls go: 2 m, 4 m and 8 m. On each
//! one, every lattice-sized stretch of line is independently a wall or not,
//! by a hash of its own position. Long 8 m walls give the space its
//! structure; the finer ones subdivide it. How likely a wall is at each scale
//! depends on a slowly varying **density** noise, so the level drifts between
//! open pillar halls and tight warrens. A wall may have a doorway, and a
//! doorway always wins over a wall from another scale.
//!
//! # No sealed rooms
//!
//! Every bay owns its north and its east segment. If both come out
//! impassable, a doorway is carved into one of them. So from any bay you can
//! always move north or east, which rules out sealed boxes, and since two
//! such north-east paths wander into each other sooner or later, the whole
//! level is connected in practice (a test checks a large sample).
//!
//! Each segment is owned by exactly one bay, so this fix-up needs no
//! knowledge of the neighbours' decisions and chunks still agree on seams.
//!
//! # Guarantee
//!
//! Every opening is at least 2 cells (1 m) wide. A doorway never straddles a
//! lattice point, so no wall ever ends in the middle of one.

use super::{level_key, purpose};
use crate::GeneratorVersion;
use crate::chunk::{Cell, Chunk, Edge, Fixture, LightState, N};
use crate::coord::{CHUNK_SIZE, CellPos, ChunkPos, RegionPos};
use crate::hash::{Key, ONE, Q16, lerp, pct};
use crate::level::LevelId;
use crate::noise::Field;

/// Side of a bay in cells.
pub const BAY: i32 = 4;
const BAY_SHIFT: u32 = 2;
/// Bays along one side of a chunk.
const BAYS: i32 = CHUNK_SIZE / BAY;
/// Number of nested wall lattices (2 m, 4 m, 8 m).
const SCALES: usize = 3;
/// Pillars stand on the 4 m lattice.
const PILLAR_PITCH: i32 = 8;

/// The knobs of the yellow-room generator. The 64 yellow levels will be 64
/// values of this struct.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Params {
    pub ceiling_mm: u16,
    /// Chance of a wall on a stretch of the 2 m, 4 m and 8 m lattice, each
    /// given at lowest, middle and highest density.
    pub wall: [[Q16; 3]; SCALES],
    /// Chance that a wall at each scale has a doorway.
    pub door: [Q16; SCALES],
    /// Chance of a pillar at a free 4 m lattice point, at lowest density.
    /// Falls linearly to zero at `pillar_until`.
    pub pillar: Q16,
    pub pillar_until: Q16,
    /// Chance that a ceiling grid position has no fixture at all.
    pub fixture_missing: Q16,
    /// Chance of a dead tube in a healthy area and in the worst of a failing
    /// one. Areas fail where the power noise rises above `dead_floor`.
    pub dead: [Q16; 2],
    pub dead_floor: Q16,
    /// Same for flickering tubes, driven by a separate noise.
    pub flicker: [Q16; 2],
    pub flicker_floor: Q16,
    /// Chance that a whole region is browned out or blacked out.
    pub brownout: Q16,
    pub blackout: Q16,
    /// Damp below this noise value is bone dry; above it, moisture rises to
    /// soaked at the top of the range.
    pub damp_floor: Q16,
    /// Same for wear.
    pub wear_floor: Q16,
}

pub const LEVEL_0: Params = Params {
    ceiling_mm: 2600,
    wall: [[pct(0), pct(5), pct(38)], [pct(6), pct(30), pct(52)], [pct(38), pct(56), pct(66)]],
    door: [pct(25), pct(50), pct(70)],
    pillar: pct(75),
    pillar_until: pct(42),
    fixture_missing: pct(6),
    dead: [pct(2), pct(55)],
    dead_floor: pct(78),
    flicker: [pct(1), pct(30)],
    flicker_floor: pct(80),
    brownout: pct(6),
    blackout: pct(2),
    damp_floor: pct(66),
    wear_floor: pct(50),
};

/// Which way a segment runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    /// Along a line of constant x: a wall between a west and an east cell.
    NorthSouth = 0,
    /// Along a line of constant y: a wall between a south and a north cell.
    EastWest = 1,
}

/// How a whole region's lights are doing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegionLighting {
    Normal,
    /// Unstable supply: many tubes flicker, more are dead.
    Brownout,
    /// No power: every tube is dead.
    Blackout,
}

/// Everything derived from `(params, level)`, built once per chunk.
struct Gen<'a> {
    p: &'a Params,
    density: Field,
    power: Field,
    restless: Field,
    damp: Field,
    wear: Field,
    wall: Key,
    carve: Key,
    pillar: Key,
    fixture: Key,
    region_lighting: Key,
}

/// 4-bit mask of a segment: bit `b` set means the cell edge `b` cells along
/// the segment (from its south or west end) is a wall.
type Mask = u8;
const FULL: Mask = 0b1111;

/// A player can pass a segment if it has two adjacent open cell edges (1 m).
const fn passable(mask: Mask) -> bool {
    let open = !mask & FULL;
    open & (open >> 1) != 0
}

impl<'a> Gen<'a> {
    fn new(p: &'a Params, level: LevelId) -> Self {
        let field = |purpose, shift, octaves, warp_shift, warp_cells, gain| Field {
            key: level_key(level, purpose),
            shift,
            octaves,
            warp_shift,
            warp_cells,
            gain,
        };
        Self {
            p,
            // 64 m zones of open versus dense layout.
            density: field(purpose::DENSITY, 7, 3, 6, 40, 620),
            // 128 m zones of failing power.
            power: field(purpose::POWER, 8, 2, 7, 48, 700),
            // 32 m pockets of flickering tubes.
            restless: field(purpose::FLICKER, 6, 2, 5, 16, 700),
            // Damp patches 16 m across with detail down to 2 m.
            damp: field(purpose::DAMP, 5, 4, 5, 12, 560),
            // Wear in 32 m patches.
            wear: field(purpose::WEAR, 6, 4, 4, 10, 520),
            wall: level_key(level, purpose::WALL),
            carve: level_key(level, purpose::CARVE),
            pillar: level_key(level, purpose::PILLAR),
            fixture: level_key(level, purpose::FIXTURE),
            region_lighting: level_key(level, purpose::REGION_LIGHTING),
        }
    }

    /// Wall probability at lattice `scale` for density `d`: a straight line
    /// from the low value to the middle value, then on to the high value.
    fn wall_chance(&self, scale: usize, d: Q16) -> Q16 {
        let [low, mid, high] = self.p.wall[scale];
        if d < ONE / 2 { lerp(low, mid, d * 2) } else { lerp(mid, high, (d - ONE / 2) * 2) }
    }

    /// The walls on one segment before the no-sealed-rooms fix-up.
    ///
    /// `(x, y)` is the cell at the segment's south or west end; both
    /// coordinates are multiples of [`BAY`].
    fn raw_segment(&self, axis: Axis, x: i32, y: i32) -> Mask {
        let (line, along) = match axis {
            Axis::NorthSouth => (x, y),
            Axis::EastWest => (y, x),
        };
        let mut walls: Mask = 0;
        let mut doors: Mask = 0;
        for scale in 0..SCALES {
            let pitch = BAY << scale;
            // The lattices nest: a line that is not on this lattice is not on
            // any coarser one either.
            if line & (pitch - 1) != 0 {
                break;
            }
            // The stretch of this scale that contains the segment, and which
            // of its 2 m pieces the segment is.
            let start = along & !(pitch - 1);
            let piece = ((along & (pitch - 1)) >> BAY_SHIFT) as u32;
            let middle = start.wrapping_add(pitch / 2);
            let (sx, sy, mx, my) = match axis {
                Axis::NorthSouth => (line, start, line, middle),
                Axis::EastWest => (start, line, middle, line),
            };

            let roll = self.wall.with(axis as u64 | (scale as u64) << 8).with_xy(sx, sy).roll();
            if !roll.chance(self.wall_chance(scale, self.density.sample(mx, my))) {
                continue;
            }
            walls = FULL;

            let roll = roll.next();
            if !roll.chance(self.p.door[scale]) {
                continue;
            }
            // The doorway sits inside one 2 m piece of the wall.
            let roll = roll.next();
            if roll.below(1 << scale) == piece {
                doors |= door_mask(roll.next());
            }
        }
        walls & !doors
    }

    /// The final north and east segments of the bay whose south-west cell is
    /// `(x, y)`, after carving an exit if the bay would have none.
    fn bay_exits(&self, x: i32, y: i32) -> (Mask, Mask) {
        let north = self.raw_segment(Axis::EastWest, x, y.wrapping_add(BAY));
        let east = self.raw_segment(Axis::NorthSouth, x.wrapping_add(BAY), y);
        if passable(north) || passable(east) {
            return (north, east);
        }
        let roll = self.carve.with_xy(x, y).roll();
        let gap: Mask = 0b11 << roll.below(3);
        if roll.chance(ONE / 2) { (north & !gap, east) } else { (north, east & !gap) }
    }

    /// Whether the cell at `(x, y)` is a pillar. Only cells at a 4 m lattice
    /// point can be, and only where none of the four segments meeting at that
    /// point carries any wall: pillars are free-standing.
    fn pillar_at(&self, x: i32, y: i32) -> bool {
        debug_assert!(x & (PILLAR_PITCH - 1) == 0 && y & (PILLAR_PITCH - 1) == 0);
        let d = self.density.sample(x, y);
        if d >= self.p.pillar_until {
            return false;
        }
        let chance = lerp(self.p.pillar, 0, d * ONE / self.p.pillar_until);
        if !self.pillar.with_xy(x, y).roll().chance(chance) {
            return false;
        }
        // The fix-up only ever removes wall from a segment that had some, so
        // "raw segment is empty" and "final segment is empty" are the same.
        self.raw_segment(Axis::NorthSouth, x, y) == 0
            && self.raw_segment(Axis::NorthSouth, x, y.wrapping_sub(BAY)) == 0
            && self.raw_segment(Axis::EastWest, x, y) == 0
            && self.raw_segment(Axis::EastWest, x.wrapping_sub(BAY), y) == 0
    }

    fn region_lighting(&self, region: RegionPos) -> RegionLighting {
        let roll = self.region_lighting.with_xy(region.x(), region.y()).roll();
        if roll.chance(self.p.blackout) {
            RegionLighting::Blackout
        } else if roll.chance(self.p.blackout + self.p.brownout) {
            RegionLighting::Brownout
        } else {
            RegionLighting::Normal
        }
    }

    /// The fixture of the bay whose south-west cell is `(x, y)`, if it has
    /// one. Fixtures sit in every other bay, in a checkerboard.
    fn fixture_at(&self, x: i32, y: i32) -> Option<(LightState, u32)> {
        if ((x >> BAY_SHIFT) ^ (y >> BAY_SHIFT)) & 1 != 0 {
            return None;
        }
        let roll = self.fixture.with_xy(x, y).roll();
        if roll.chance(self.p.fixture_missing) {
            return None;
        }
        // Truncating cast: the low 32 bits of the hash.
        let seed = roll.0 as u32;

        let (mut dead, mut flicker) = (
            lerp(self.p.dead[0], self.p.dead[1], ramp(self.power.sample(x, y), self.p.dead_floor)),
            lerp(self.p.flicker[0], self.p.flicker[1], ramp(self.restless.sample(x, y), self.p.flicker_floor)),
        );
        match self.region_lighting(CellPos::new(x, y).region()) {
            RegionLighting::Normal => {}
            RegionLighting::Brownout => {
                dead = dead.max(pct(30));
                flicker = pct(60);
            }
            RegionLighting::Blackout => dead = ONE,
        }

        let roll = roll.next();
        let state = if roll.chance(dead) {
            LightState::Dead
        } else if roll.next().chance(flicker) {
            LightState::Flicker
        } else {
            LightState::On
        };
        Some((state, seed))
    }
}

/// A doorway inside a 2 m segment: 1 m, 1.5 m or the full 2 m wide.
fn door_mask(roll: crate::hash::Roll) -> Mask {
    let width = match roll.unit() {
        u if u < pct(60) => 2,
        u if u < pct(85) => 3,
        _ => 4,
    };
    let offset = roll.below(5 - width);
    ((1 << width) - 1) << offset
}

/// Zero up to `floor`, then a straight rise to (just under) 1.0 at the top of
/// the noise range. `floor` must be below 65535.
fn ramp(n: Q16, floor: Q16) -> Q16 {
    if n <= floor { 0 } else { (n - floor) * (ONE - 1) / (ONE - 1 - floor) }
}

/// The lighting regime of a region on a yellow level.
pub fn region_lighting(params: &Params, level: LevelId, region: RegionPos) -> RegionLighting {
    Gen::new(params, level).region_lighting(region)
}

pub fn generate(version: GeneratorVersion, params: &Params, level: LevelId, pos: ChunkPos) -> Chunk {
    let g = Gen::new(params, level);
    let origin = pos.origin();
    let mut chunk = Chunk::empty(version, level, pos);

    // Walls. The bays one step outside the chunk to the west and south own
    // the segments on the chunk's west and south boundary, so they are
    // visited too.
    for j in -1..BAYS {
        for i in -1..BAYS {
            let bay = origin.offset(i * BAY, j * BAY);
            let (north, east) = g.bay_exits(bay.x, bay.y);
            for b in 0..BAY {
                if i >= 0 && north >> b & 1 != 0 {
                    chunk.south[((j + 1) * BAY) as usize][(i * BAY + b) as usize] = Edge::Wall;
                }
                if j >= 0 && east >> b & 1 != 0 {
                    chunk.west[(j * BAY + b) as usize][((i + 1) * BAY) as usize] = Edge::Wall;
                }
            }
        }
    }

    // Pillars. Lattice points on the east and north boundary belong to the
    // neighbouring chunks, but a pillar there puts a wall on our boundary
    // edge, so those points are checked as well.
    for py in (0..=CHUNK_SIZE).step_by(PILLAR_PITCH as usize) {
        for px in (0..=CHUNK_SIZE).step_by(PILLAR_PITCH as usize) {
            let cell = origin.offset(px, py);
            if !g.pillar_at(cell.x, cell.y) {
                continue;
            }
            let (x, y) = (px as usize, py as usize);
            if x < N && y < N {
                chunk.cells[y][x].solid = true;
            }
            if y < N {
                chunk.west[y][x] = Edge::Wall;
                if x < N {
                    chunk.west[y][x + 1] = Edge::Wall;
                }
            }
            if x < N {
                chunk.south[y][x] = Edge::Wall;
                if y < N {
                    chunk.south[y + 1][x] = Edge::Wall;
                }
            }
        }
    }

    // Ceiling lights: one candidate per bay, over the two cells just inside
    // the bay's south-west corner (clear of any pillar on the corner itself).
    for j in 0..BAYS {
        for i in 0..BAYS {
            let bay = origin.offset(i * BAY, j * BAY);
            if let Some((state, seed)) = g.fixture_at(bay.x, bay.y) {
                let fixture = Fixture { x: (i * BAY + 1) as u8, y: (j * BAY + 1) as u8, state, seed };
                let stored = chunk.push_fixture(fixture);
                debug_assert!(stored, "checkerboard yields at most 8 fixtures per chunk");
            }
        }
    }

    // Surface condition per cell.
    for (y, row) in chunk.cells.iter_mut().enumerate() {
        for (x, cell) in row.iter_mut().enumerate() {
            let at = origin.offset(x as i32, y as i32);
            *cell = Cell {
                solid: cell.solid,
                // `>> 8` keeps the top byte of the 16-bit fraction.
                damp: (ramp(g.damp.sample(at.x, at.y), params.damp_floor) >> 8) as u8,
                wear: (ramp(g.wear.sample(at.x, at.y), params.wear_floor) >> 8) as u8,
            };
        }
    }

    chunk
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passable_needs_one_metre() {
        assert!(passable(0b0000));
        assert!(passable(0b1100));
        assert!(passable(0b1001));
        assert!(!passable(0b1010));
        assert!(!passable(0b0101));
        assert!(!passable(0b1111));
        assert!(!passable(0b1101));
    }

    #[test]
    fn doors_are_at_least_one_metre_and_inside_the_segment() {
        let key = Key::new(5);
        for i in 0..2000 {
            let mask = door_mask(key.with(i).roll());
            assert!(mask != 0 && mask & !FULL == 0, "door {mask:#b} leaves the segment");
            assert!(passable(FULL & !mask), "door {mask:#b} is too narrow");
        }
    }

    #[test]
    fn ramp_spans_the_range() {
        assert_eq!(ramp(0, pct(40)), 0);
        assert_eq!(ramp(pct(40), pct(40)), 0);
        assert_eq!(ramp(ONE - 1, pct(40)), ONE - 1);
        assert_eq!(ramp(ONE - 1, pct(40)) >> 8, 255);
    }

    #[test]
    fn every_bay_has_an_exit() {
        let g = Gen::new(&LEVEL_0, LevelId::ZERO);
        // Around the origin and across the world corner.
        for start in [-400i32, i32::MAX - 399] {
            for j in 0..200 {
                for i in 0..200 {
                    let (x, y) = (start.wrapping_add(i * BAY), start.wrapping_add(j * BAY));
                    let (north, east) = g.bay_exits(x, y);
                    assert!(passable(north) || passable(east), "bay at {x},{y} is sealed");
                }
            }
        }
    }
}

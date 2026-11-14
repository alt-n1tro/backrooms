//! The yellow rooms.
//!
//! # Layout
//!
//! The level is an endless office floor drawn by something that has seen
//! floor plans but never worked in a building.
//!
//! The plane is cut into **blocks** 32 m square. Each block is divided the
//! way an architect divides a floor: cut the space in two, then cut each
//! part again, until the parts are room-sized. Often a cut leaves a 2 m
//! strip between the two halves, which becomes a **corridor**. The parts
//! that are not cut further are the **rooms**. A slow **density** noise sets
//! how far each block is cut up, so the level drifts between open halls with
//! grids of columns and warrens of small offices.
//!
//! Every cut is a wall, on the 2 m lattice (every 4 cells), and every wall
//! gets at least one doorway. So each division joins its two sides, the
//! walls between blocks get doorways the same way, and by induction every
//! ordinary room can be walked to from every other.
//!
//! Everything is a pure function of position: to find what stands between
//! two neighbouring 2 m bays, the generator replays the cuts of their block
//! from the top until one separates them. Nothing is stored.
//!
//! # Secret rooms
//!
//! A rare small room is **secret**: no doorway may touch it, and exactly one
//! piece of its wall has a crawl hole instead. Crawl holes exist nowhere
//! else, so finding one always means finding a room that cannot be walked
//! into.
//!
//! # Guarantee
//!
//! Every opening is at least 2 cells (1 m) wide and lies within one 2 m
//! piece of wall, so no wall ever ends in the middle of a doorway.
//!
//! # Oddities
//!
//! A plain wall piece occasionally has an unglazed window in its middle
//! metre. Separately, a slow **wrongness** noise marks out zones where the
//! wall lattice itself is out of true: each lattice point there is pushed a
//! few centimetres off its place, so walls stop being parallel.

use super::{level_key, purpose};
use crate::GeneratorVersion;
use crate::chunk::{Cell, Chunk, Dir, Edge, Fixture, LightState, N, Skew, Stair};
use crate::coord::{CHUNK_SIZE, CellPos, ChunkPos, RegionPos};
use crate::hash::{Key, ONE, Q16, lerp, pct};
use crate::level::LevelId;
use crate::noise::Field;
use crate::rarity::Rarity;

/// Side of a bay in cells.
pub const BAY: i32 = 4;
const BAY_SHIFT: u32 = 2;
/// Bays along one side of a chunk.
const BAYS: i32 = CHUNK_SIZE / BAY;
/// Side of a block in cells (32 m).
pub const BLOCK: i32 = 64;
/// Columns stand on every other lattice point.
const PILLAR_PITCH: i32 = 8;

/// The knobs of the yellow-room generator. The 64 yellow levels will be 64
/// values of this struct.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Params {
    pub ceiling_mm: u16,
    /// A space is cut further while it is longer than this many bays, in
    /// the most open blocks and in the densest.
    pub room_limit: [u8; 2],
    /// Chance that a cut leaves a corridor between its two sides, in the
    /// most open blocks and in the densest.
    pub corridor: [Q16; 2],
    /// Chance of a doorway at each place along a wall, beyond the one every
    /// wall is guaranteed.
    pub extra_door: Q16,
    /// Chance that a cut is left without a wall at all, in the most open
    /// blocks and in the densest.
    pub open_plan: [Q16; 2],
    /// Chance that a doorway is the full 2 m wide instead of 1 m.
    pub wide_door: Q16,
    /// Chance that a hall (a room at least 8 m both ways) has columns.
    pub pillars: Q16,
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
    /// Walls go out of true where the wrongness noise rises above this.
    pub skew_floor: Q16,
    /// Furthest a lattice point is pushed, per axis, in centimetres.
    pub skew_cm: u8,
    /// Whether there is a level for stairs to lead up or down to. Filled in
    /// from the stack, not chosen.
    pub stairs_up: bool,
    pub stairs_down: bool,
}

pub const LEVEL_0: Params = Params {
    ceiling_mm: 2600,
    room_limit: [9, 3],
    corridor: [pct(25), pct(75)],
    extra_door: pct(10),
    open_plan: [pct(22), pct(3)],
    wide_door: pct(30),
    pillars: pct(70),
    fixture_missing: pct(6),
    dead: [pct(2), pct(55)],
    dead_floor: pct(78),
    flicker: [pct(1), pct(30)],
    flicker_floor: pct(80),
    brownout: pct(6),
    blackout: pct(2),
    damp_floor: pct(66),
    wear_floor: pct(50),
    skew_floor: pct(84),
    skew_cm: 28,
    stairs_up: false,
    stairs_down: false,
};

/// The parameters and wallpaper tint of one of the yellow levels 1 to 63.
///
/// Each is Level 0 with its knobs turned by a hash of the level's id: some
/// are low and cramped, some airy, some drowned, some mostly dark. A level
/// keeps its character everywhere, which is what makes arriving on a new
/// one feel like a different place rather than more of the same.
pub fn variant(level: LevelId) -> (Params, [u8; 3]) {
    let key = level_key(level, purpose::VARIANT);
    // A whole number from `low` to `high`, different for each knob.
    let pick = |knob: u64, low: u32, high: u32| low + key.with(knob).roll().below(high - low + 1);
    let base = LEVEL_0;
    let open = pick(1, 5, 12);
    let dense = pick(2, 2, 4);
    let params = Params {
        // Mostly ordinary ceilings, now and then a very low or very high one.
        ceiling_mm: match pick(0, 0, 9) {
            0 => 2200,
            1 => 3400,
            2 => 4400,
            n => 2400 + (n as u16 - 3) * 100,
        },
        room_limit: [open as u8, dense as u8],
        corridor: [pct(pick(3, 5, 45)), pct(pick(4, 45, 95))],
        extra_door: pct(pick(5, 3, 18)),
        open_plan: [pct(pick(6, 5, 45)), pct(pick(7, 0, 10))],
        wide_door: pct(pick(8, 10, 60)),
        pillars: pct(pick(9, 20, 100)),
        fixture_missing: pct(pick(10, 2, 30)),
        dead: [pct(pick(11, 1, 12)), pct(pick(12, 30, 90))],
        dead_floor: pct(pick(13, 55, 85)),
        flicker: [pct(pick(14, 0, 5)), pct(pick(15, 10, 50))],
        damp_floor: pct(pick(16, 35, 85)),
        wear_floor: pct(pick(17, 25, 75)),
        skew_floor: pct(pick(18, 70, 95)),
        skew_cm: pick(19, 10, 40) as u8,
        ..base
    };
    // Tints stay within "sickly yellow": towards green, ochre or bone.
    let tint = [pick(20, 200, 255) as u8, pick(21, 205, 255) as u8, pick(22, 150, 255) as u8];
    (params, tint)
}

/// Which way a wall piece runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Axis {
    /// Along a line of constant x: a wall between a west and an east bay.
    NorthSouth = 0,
    /// Along a line of constant y: a wall between a south and a north bay.
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

/// A rectangle of floor, in cells: `x0..x1` by `y0..y1`. All four are
/// multiples of [`BAY`]. Coordinates are relative to the block's corner, so
/// they never wrap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Rect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl Rect {
    const fn bays_wide(self) -> i32 {
        (self.x1 - self.x0) / BAY
    }
    const fn bays_deep(self) -> i32 {
        (self.y1 - self.y0) / BAY
    }
    const fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

/// What a room is, as far as the layout cares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoomKind {
    Room,
    /// A 2 m wide strip left between the two sides of a cut, running east.
    CorridorEast,
    /// The same, running north.
    CorridorNorth,
    /// A room no doorway leads to; see the module docs.
    Secret,
}

/// A room: a part of a block that is not cut any further.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Room {
    /// Cell at the block's south-west corner; the rectangle is relative to
    /// it.
    block: CellPos,
    rect: Rect,
    /// Identifies the room. Later milestones hash it for furniture and use.
    pub key: Key,
    pub kind: RoomKind,
}

impl Room {
    /// Size in bays (2 m each): east-west, then north-south.
    pub const fn size(&self) -> (i32, i32) {
        (self.rect.bays_wide(), self.rect.bays_deep())
    }

    /// The room's south-west cell.
    pub const fn corner(&self) -> CellPos {
        self.block.offset(self.rect.x0, self.rect.y0)
    }
}

/// How a space is cut: across which axis and where. A corridor cut has two
/// lines with the corridor between them; a plain cut has `from == to`.
#[derive(Clone, Copy)]
struct Cut {
    /// True if the cut lines run north-south (dividing east from west).
    vertical: bool,
    /// Cell coordinates of the first and second line.
    from: i32,
    to: i32,
}

/// A wall line found between two neighbouring bays.
#[derive(Clone, Copy)]
struct Line {
    key: Key,
    /// Cell at the block's corner, and the line's extent relative to it:
    /// `at` across, `lo..lo + 4 * len` along.
    block: CellPos,
    vertical: bool,
    at: i32,
    lo: i32,
    /// Length in bays.
    len: i32,
    /// True for a wall between two blocks.
    boundary: bool,
}

/// Everything derived from `(params, level)`, built once per chunk.
struct Gen<'a> {
    p: &'a Params,
    density: Field,
    power: Field,
    restless: Field,
    damp: Field,
    wear: Field,
    wrongness: Field,
    wall: Key,
    fixture: Key,
    region_lighting: Key,
    feature: Key,
    skew: Key,
    portal: Key,
    level: LevelId,
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
            // 128 m zones of open versus dense layout.
            density: field(purpose::DENSITY, 8, 2, 7, 60, 640),
            // 128 m zones of failing power.
            power: field(purpose::POWER, 8, 2, 7, 48, 700),
            // 32 m pockets of flickering tubes.
            restless: field(purpose::FLICKER, 6, 2, 5, 16, 700),
            // Damp patches 16 m across with detail down to 2 m.
            damp: field(purpose::DAMP, 5, 4, 5, 12, 560),
            // Wear in 32 m patches.
            wear: field(purpose::WEAR, 6, 4, 4, 10, 520),
            // 64 m zones where the geometry goes subtly wrong.
            wrongness: field(purpose::WRONGNESS, 7, 2, 6, 24, 640),
            wall: level_key(level, purpose::WALL),
            fixture: level_key(level, purpose::FIXTURE),
            region_lighting: level_key(level, purpose::REGION_LIGHTING),
            feature: level_key(level, purpose::FEATURE),
            skew: level_key(level, purpose::SKEW),
            portal: level_key(level, purpose::PORTAL),
            level,
        }
    }

    /// The corner of the block containing cell `(x, y)`.
    fn block_of(x: i32, y: i32) -> CellPos {
        CellPos::new(x & !(BLOCK - 1), y & !(BLOCK - 1))
    }

    /// How densely a block is divided, 0 (open halls) to 65535 (warren).
    /// One value per block, so a block has one character throughout.
    fn block_density(&self, block: CellPos) -> Q16 {
        let middle = block.offset(BLOCK / 2, BLOCK / 2);
        self.density.sample(middle.x, middle.y)
    }

    fn block_key(&self, block: CellPos) -> Key {
        self.wall.with(0).with_xy(block.x, block.y)
    }

    /// How (and whether) the space `rect` of a block is cut. `None` means it
    /// is a room.
    fn cut(&self, rect: Rect, key: Key, density: Q16) -> Option<Cut> {
        let (w, h) = (rect.bays_wide(), rect.bays_deep());
        let roll = key.roll();
        // Spaces are cut until they fit the block's room size, give or take
        // a bay so that rooms in one block are not all alike.
        let limit = lerp(self.p.room_limit[0] as u32, self.p.room_limit[1] as u32, density) as i32;
        let limit = (limit + roll.below(3) as i32 - 1).max(1);
        let long = if w > h { w } else { h };
        // A strip one bay wide is a corridor or a closet: never cut along
        // its length into slivers, only across.
        if long <= limit || long < 2 {
            return None;
        }
        // Cut across the longer side; a square is cut either way.
        let vertical = if w != h { w > h } else { roll.next().chance(ONE / 2) };
        let start = if vertical { rect.x0 } else { rect.y0 };
        let roll = roll.next().next();

        let corridor_chance = lerp(self.p.corridor[0], self.p.corridor[1], density);
        if long >= 5 && roll.chance(corridor_chance) {
            // A corridor one bay wide, with at least two bays either side.
            let first = 2 + roll.below((long - 4) as u32) as i32;
            return Some(Cut { vertical, from: start + first * BAY, to: start + (first + 1) * BAY });
        }
        // A plain cut, leaving at least two bays either side where there is
        // room for that.
        let margin = if long >= 4 { 2 } else { 1 };
        let at = start + (margin + roll.below((long - 2 * margin + 1) as u32) as i32) * BAY;
        Some(Cut { vertical, from: at, to: at })
    }

    /// The parts a cut divides `rect` into: two, or three with a corridor in
    /// the middle. Unused entries are `None`.
    fn parts(rect: Rect, cut: Cut) -> [Option<Rect>; 3] {
        let strip = |a: i32, b: i32| {
            if cut.vertical { Rect { x0: a, x1: b, ..rect } } else { Rect { y0: a, y1: b, ..rect } }
        };
        let (start, end) = if cut.vertical { (rect.x0, rect.x1) } else { (rect.y0, rect.y1) };
        if cut.from == cut.to {
            [Some(strip(start, cut.from)), Some(strip(cut.from, end)), None]
        } else {
            [Some(strip(start, cut.from)), Some(strip(cut.from, cut.to)), Some(strip(cut.to, end))]
        }
    }

    /// The room containing cell `(x, y)`.
    fn room_at(&self, x: i32, y: i32) -> Room {
        let block = Self::block_of(x, y);
        let density = self.block_density(block);
        let (lx, ly) = (x & (BLOCK - 1), y & (BLOCK - 1));
        let mut rect = Rect { x0: 0, y0: 0, x1: BLOCK, y1: BLOCK };
        let mut key = self.block_key(block);
        let mut kind = RoomKind::Room;
        while let Some(cut) = self.cut(rect, key, density) {
            let parts = Self::parts(rect, cut);
            let (index, part) = parts
                .iter()
                .enumerate()
                .find_map(|(i, part)| part.filter(|p| p.contains(lx, ly)).map(|p| (i, p)))
                .expect("the parts of a cut cover the whole space");
            rect = part;
            key = key.with(index as u64 + 1);
            // The middle part of a three-way cut is the corridor. A
            // corridor is never cut again: it runs the full length of the
            // space it was laid in.
            if parts[2].is_some() && index == 1 {
                kind = if cut.vertical { RoomKind::CorridorNorth } else { RoomKind::CorridorEast };
                break;
            }
        }
        // A small ordinary room is occasionally secret.
        let small = rect.bays_wide() * rect.bays_deep() <= 6;
        if kind == RoomKind::Room && small && Rarity::of(key.with(0x5EC).roll()).is_some() {
            kind = RoomKind::Secret;
        }
        Room { block, rect, key, kind }
    }

    /// The wall line that separates the two bays on either side of the
    /// piece at `(x, y)`, or `None` if they are in the same room.
    fn line_between(&self, axis: Axis, x: i32, y: i32) -> Option<Line> {
        let vertical = axis == Axis::NorthSouth;
        // The bay on the low side (west or south) and on the high side.
        let (ax, ay) = if vertical { (x.wrapping_sub(BAY), y) } else { (x, y.wrapping_sub(BAY)) };
        let (block_a, block_b) = (Self::block_of(ax, ay), Self::block_of(x, y));
        if block_a != block_b {
            // The wall between two blocks: a line the length of a block.
            let key = self.wall.with(1 + axis as u64).with_xy(block_b.x, block_b.y);
            return Some(Line { key, block: block_b, vertical, at: 0, lo: 0, len: BLOCK / BAY, boundary: true });
        }
        let block = block_b;
        let density = self.block_density(block);
        let (alx, aly) = (ax & (BLOCK - 1), ay & (BLOCK - 1));
        let (blx, bly) = (x & (BLOCK - 1), y & (BLOCK - 1));
        let mut rect = Rect { x0: 0, y0: 0, x1: BLOCK, y1: BLOCK };
        let mut key = self.block_key(block);
        while let Some(cut) = self.cut(rect, key, density) {
            let parts = Self::parts(rect, cut);
            let index_of = |px: i32, py: i32| {
                parts.iter().position(|part| part.is_some_and(|p| p.contains(px, py))).expect("covered")
            };
            let (ia, ib) = (index_of(alx, aly), index_of(blx, bly));
            if ia != ib {
                // This cut separates them. Its line runs the full width of
                // the space being cut.
                let at = if ia == 0 { cut.from } else { cut.to };
                let (lo, len) = if cut.vertical { (rect.y0, rect.bays_deep()) } else { (rect.x0, rect.bays_wide()) };
                let key = key.with(0x11E + ia as u64);
                return Some(Line { key, block, vertical: cut.vertical, at, lo, len, boundary: false });
            }
            // Both are in the corridor of this cut, which is one room.
            if parts[2].is_some() && ia == 1 {
                return None;
            }
            rect = parts[ia].expect("index came from this array");
            key = key.with(ia as u64 + 1);
        }
        None
    }

    /// The rooms on the low and high side of a line, `place` bays along it.
    fn rooms_beside(&self, line: &Line, place: i32) -> (Room, Room) {
        let along = line.lo + place * BAY;
        let (hx, hy) = if line.vertical { (line.at, along) } else { (along, line.at) };
        let high = line.block.offset(hx, hy);
        let low = if line.vertical { high.offset(-BAY, 0) } else { high.offset(0, -BAY) };
        (self.room_at(low.x, low.y), self.room_at(high.x, high.y))
    }

    /// Whether the wall piece at cell `(x, y)` is the one piece of a secret
    /// room's wall that has the crawl hole.
    fn is_secret_entrance(&self, room: &Room, axis: Axis, x: i32, y: i32) -> bool {
        if room.kind != RoomKind::Secret {
            return false;
        }
        let (w, h) = room.size();
        let pick = room.key.with(0xC4A).roll().below(2 * (w + h) as u32) as i32;
        let corner = room.corner();
        // Pieces are numbered along the south side, the north, the west and
        // then the east.
        let (pick_axis, px, py) = if pick < w {
            (Axis::EastWest, pick * BAY, 0)
        } else if pick < 2 * w {
            (Axis::EastWest, (pick - w) * BAY, h * BAY)
        } else if pick < 2 * w + h {
            (Axis::NorthSouth, 0, (pick - 2 * w) * BAY)
        } else {
            (Axis::NorthSouth, w * BAY, (pick - 2 * w - h) * BAY)
        };
        let chosen = corner.offset(px, py);
        pick_axis == axis && chosen.x == x && chosen.y == y
    }

    /// The place along a line where its guaranteed doorway goes: a hashed
    /// place, moved along if it would open into a secret room.
    fn guaranteed_door(&self, line: &Line) -> Option<i32> {
        let first = line.key.roll().below(line.len as u32) as i32;
        (0..line.len).map(|step| (first + step) % line.len).find(|&place| {
            let (low, high) = self.rooms_beside(line, place);
            low.kind != RoomKind::Secret && high.kind != RoomKind::Secret
        })
    }

    /// What stands on each of the four cell edges of the 2 m wall piece
    /// whose south or west end is at cell `(x, y)`.
    fn segment(&self, axis: Axis, x: i32, y: i32) -> [Edge; 4] {
        const WALL: [Edge; 4] = [Edge::Wall; 4];
        const OPEN: [Edge; 4] = [Edge::Open; 4];
        let middle = |edge: Edge| [Edge::Wall, edge, edge, Edge::Wall];

        let Some(line) = self.line_between(axis, x, y) else {
            // Inside a room there is nothing, except around a stairwell.
            let room = self.room_at(x, y);
            return match self.stair_of(&room) {
                Some((bay, facing, _)) if Self::booth_side(bay, axis, x, y).is_some_and(|side| side != facing) => WALL,
                _ => OPEN,
            };
        };
        let local = CellPos::new(x, y);
        let (dx, dy) = line.block.delta_to(local);
        let place = ((if line.vertical { dy } else { dx }) - line.lo) / BAY;
        let (low, high) = self.rooms_beside(&line, place);

        // Secret rooms: one crawl hole, otherwise blank wall.
        if low.kind == RoomKind::Secret || high.kind == RoomKind::Secret {
            let entrance = self.is_secret_entrance(&low, axis, x, y) || self.is_secret_entrance(&high, axis, x, y);
            return if entrance { middle(Edge::Crawl) } else { WALL };
        }

        let here = line.key.with(1 + place as u64).roll();
        let density = self.block_density(line.block);
        // Some cuts inside a block are never built, which merges rooms into
        // L-shapes and open-plan floors.
        if !line.boundary && line.len >= 2 {
            let open_plan = lerp(self.p.open_plan[0], self.p.open_plan[1], density);
            if line.key.with(0).roll().chance(open_plan) {
                return OPEN;
            }
        }
        // A corridor that reaches this wall end-on runs straight through it.
        let ends_here =
            |room: &Room| room.kind == if line.vertical { RoomKind::CorridorEast } else { RoomKind::CorridorNorth };
        if ends_here(&low) || ends_here(&high) {
            return OPEN;
        }
        // Doorways: the guaranteed one, and the occasional extra. Walls
        // between blocks get more, so blocks do not feel like islands.
        let extra = if line.boundary { self.p.extra_door * 2 } else { self.p.extra_door };
        if self.guaranteed_door(&line) == Some(place) || here.chance(extra) {
            return if here.next().chance(self.p.wide_door) { OPEN } else { middle(Edge::Open) };
        }
        // A plain piece of wall. Rarely it is one end of a portal: the same
        // piece of the block a long way off is the other end. Both ends roll
        // the same hash (of whichever of the two comes first), so they agree.
        let partner = super::portal_partner(self.level, local);
        let first = if (partner.x, partner.y) < (x, y) { partner } else { local };
        let pair = self.portal.with(axis as u64).with_xy(first.x, first.y).roll();
        if Rarity::of(pair).is_some_and(|tier| tier >= Rarity::Rare) {
            return middle(Edge::Portal);
        }
        // Otherwise it now and then has a window in it, and very rarely is
        // not really there at all.
        let feature = self.feature.with(axis as u64).with_xy(x, y).roll();
        match Rarity::of(feature) {
            None => WALL,
            Some(Rarity::Common | Rarity::Uncommon) => middle(Edge::Window),
            Some(_) => middle(Edge::Phase),
        }
    }

    /// The stairwell of a room, if it has one: the booth's south-west cell,
    /// its open side, and whether the steps go up.
    ///
    /// Only rooms at least three bays each way can have one, and the booth
    /// stands clear of the room's walls, so it never blocks a doorway.
    fn stair_of(&self, room: &Room) -> Option<(CellPos, Dir, bool)> {
        let (w, h) = room.size();
        if room.kind != RoomKind::Room || w < 3 || h < 3 || !(self.p.stairs_up || self.p.stairs_down) {
            return None;
        }
        let roll = room.key.with(0x57A1).roll();
        if !Rarity::of(roll).is_some_and(|tier| tier >= Rarity::Rare) {
            return None;
        }
        let roll = roll.next();
        let i = 1 + roll.below((w - 2) as u32) as i32;
        let roll = roll.next();
        let j = 1 + roll.below((h - 2) as u32) as i32;
        let roll = roll.next();
        let facing = [Dir::North, Dir::East, Dir::South, Dir::West][roll.below(4) as usize];
        // Either way if both exist; otherwise the way that leads somewhere.
        let up = if self.p.stairs_up && self.p.stairs_down { roll.next().chance(ONE / 2) } else { self.p.stairs_up };
        Some((room.corner().offset(i * BAY, j * BAY), facing, up))
    }

    /// Which side of the booth at `bay` the wall piece at `(x, y)` is, if
    /// any.
    fn booth_side(bay: CellPos, axis: Axis, x: i32, y: i32) -> Option<Dir> {
        let at = CellPos::new(x, y);
        match axis {
            Axis::EastWest if at == bay => Some(Dir::South),
            Axis::EastWest if at == bay.offset(0, BAY) => Some(Dir::North),
            Axis::NorthSouth if at == bay => Some(Dir::West),
            Axis::NorthSouth if at == bay.offset(BAY, 0) => Some(Dir::East),
            _ => None,
        }
    }

    /// How far the lattice point at cell corner `(x, y)` is out of true.
    fn skew_at(&self, x: i32, y: i32) -> Skew {
        let strength = ramp(self.wrongness.sample(x, y), self.p.skew_floor);
        // Largest push here, in centimetres: none outside a wrong zone,
        // rising to the full amount at its heart.
        let reach = (self.p.skew_cm as u32 * strength) >> 16;
        if reach == 0 {
            return Skew::default();
        }
        let roll = self.skew.with_xy(x, y).roll();
        // A whole number from -reach to +reach, each equally likely.
        let push = |roll: crate::hash::Roll| (roll.below(2 * reach + 1) as i32 - reach as i32) as i8;
        Skew { dx: push(roll), dy: push(roll.next()) }
    }

    /// Whether the cell at `(x, y)` is a column. Columns stand in halls, on
    /// every other lattice point that is well inside the room.
    fn pillar_at(&self, x: i32, y: i32) -> bool {
        debug_assert!(x & (PILLAR_PITCH - 1) == 0 && y & (PILLAR_PITCH - 1) == 0);
        let room = self.room_at(x, y);
        let (w, h) = room.size();
        // A hall with a stairwell has no columns, so the two never collide.
        if room.kind != RoomKind::Room || w < 4 || h < 4 || self.stair_of(&room).is_some() {
            return false;
        }
        // The lattice point must be inside the room, not on its walls: the
        // bays diagonally across it belong to the same room.
        let corner = room.corner();
        let (dx, dy) = corner.delta_to(CellPos::new(x, y));
        if dx <= 0 || dy <= 0 {
            return false;
        }
        // One decision per hall: it has columns or it does not.
        room.key.with(0x9111).roll().chance(self.p.pillars)
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

    // Walls: every 2 m piece on every lattice line of the chunk, including
    // the far boundary, which the neighbouring chunk computes identically.
    for j in 0..=BAYS {
        for i in 0..=BAYS {
            let at = origin.offset(i * BAY, j * BAY);
            if i < BAYS {
                let piece = g.segment(Axis::EastWest, at.x, at.y);
                chunk.south[(j * BAY) as usize][(i * BAY) as usize..][..4].copy_from_slice(&piece);
            }
            if j < BAYS {
                let piece = g.segment(Axis::NorthSouth, at.x, at.y);
                for (b, edge) in piece.into_iter().enumerate() {
                    chunk.west[(j * BAY) as usize + b][(i * BAY) as usize] = edge;
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

    // Where the lattice is out of true.
    for (j, row) in chunk.skew.iter_mut().enumerate() {
        for (i, skew) in row.iter_mut().enumerate() {
            let point = origin.offset(i as i32 * BAY, j as i32 * BAY);
            *skew = g.skew_at(point.x, point.y);
        }
    }

    // Ceiling lights: one candidate per bay, over the two cells just inside
    // the bay's south-west corner (clear of any pillar on the corner itself).
    for j in 0..BAYS {
        for i in 0..BAYS {
            let bay = origin.offset(i * BAY, j * BAY);
            // A stairwell: note it, and leave its ceiling (or the hole in
            // it) without a light.
            let room = g.room_at(bay.x, bay.y);
            if let Some((at, facing, up)) = g.stair_of(&room)
                && at == bay
            {
                chunk.stair = Some(Stair { x: (i * BAY) as u8, y: (j * BAY) as u8, facing, up });
                continue;
            }
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
    // The crate itself is `no_std`; its tests may use the standard library.
    extern crate std;
    use std::vec::Vec;

    use super::*;

    fn generator() -> Gen<'static> {
        Gen::new(&LEVEL_0, LevelId::ZERO)
    }

    /// Bay corners over a few blocks near the origin and across the world
    /// corner.
    fn sample_bays() -> impl Iterator<Item = (i32, i32)> {
        [-256i32, i32::MAX - 255].into_iter().flat_map(|start| {
            (0..96).flat_map(move |j| (0..96).map(move |i| (start.wrapping_add(i * BAY), start.wrapping_add(j * BAY))))
        })
    }

    #[test]
    fn ramp_spans_the_range() {
        assert_eq!(ramp(0, pct(40)), 0);
        assert_eq!(ramp(pct(40), pct(40)), 0);
        assert_eq!(ramp(ONE - 1, pct(40)), ONE - 1);
        assert_eq!(ramp(ONE - 1, pct(40)) >> 8, 255);
    }

    #[test]
    fn rooms_tile_their_block() {
        let g = generator();
        for (x, y) in sample_bays() {
            let room = g.room_at(x, y);
            let (w, h) = room.size();
            assert!(w >= 1 && h >= 1 && w <= 16 && h <= 16);
            // Every bay of the room reports the same room.
            let corner = room.corner();
            let far = corner.offset((w - 1) * BAY, (h - 1) * BAY);
            assert_eq!(g.room_at(corner.x, corner.y), room);
            assert_eq!(g.room_at(far.x, far.y), room);
            // And the bay just past its east side is a different one.
            let beyond = corner.offset(w * BAY, 0);
            assert_ne!(g.room_at(beyond.x, beyond.y).key, room.key);
        }
    }

    #[test]
    fn walls_stand_exactly_between_different_rooms() {
        let g = generator();
        for (x, y) in sample_bays() {
            for (axis, lx, ly) in [(Axis::NorthSouth, x.wrapping_sub(BAY), y), (Axis::EastWest, x, y.wrapping_sub(BAY))]
            {
                let same_room =
                    g.room_at(lx, ly).key == g.room_at(x, y).key && Gen::block_of(lx, ly) == Gen::block_of(x, y);
                let piece = g.segment(axis, x, y);
                if same_room {
                    assert_eq!(piece, [Edge::Open; 4], "wall inside a room at {x},{y}");
                }
                assert_eq!(g.line_between(axis, x, y).is_none(), same_room);
            }
        }
    }

    #[test]
    fn every_piece_is_one_of_the_known_shapes() {
        let g = generator();
        let w = Edge::Wall;
        for (x, y) in sample_bays() {
            for axis in [Axis::NorthSouth, Axis::EastWest] {
                let piece = g.segment(axis, x, y);
                let known = [
                    [Edge::Open; 4],
                    [w; 4],
                    [w, Edge::Open, Edge::Open, w],
                    [w, Edge::Window, Edge::Window, w],
                    [w, Edge::Crawl, Edge::Crawl, w],
                    [w, Edge::Portal, Edge::Portal, w],
                    [w, Edge::Phase, Edge::Phase, w],
                ];
                assert!(known.contains(&piece), "odd piece {piece:?} at {x},{y}");
            }
        }
    }

    #[test]
    fn secret_rooms_have_one_crawl_hole_and_no_doorway() {
        let g = generator();
        let mut secrets = 0;
        for (x, y) in sample_bays() {
            let room = g.room_at(x, y);
            if room.kind != RoomKind::Secret || room.corner() != CellPos::new(x, y) {
                continue;
            }
            secrets += 1;
            let (w, h) = room.size();
            let corner = room.corner();
            let mut pieces = Vec::new();
            for i in 0..w {
                pieces.push(g.segment(Axis::EastWest, corner.offset(i * BAY, 0).x, corner.y));
                pieces.push(g.segment(Axis::EastWest, corner.offset(i * BAY, 0).x, corner.offset(0, h * BAY).y));
            }
            for j in 0..h {
                pieces.push(g.segment(Axis::NorthSouth, corner.x, corner.offset(0, j * BAY).y));
                pieces.push(g.segment(Axis::NorthSouth, corner.offset(w * BAY, 0).x, corner.offset(0, j * BAY).y));
            }
            let holes = pieces.iter().filter(|p| p[1] == Edge::Crawl).count();
            let open = pieces.iter().filter(|p| p.contains(&Edge::Open)).count();
            // Two secret rooms side by side may each put their hole in the
            // shared wall, so "at least one" is the rule.
            assert!(holes >= 1, "secret room at {x},{y} has no way in");
            assert_eq!(open, 0, "secret room at {x},{y} has a doorway");
        }
        assert!(secrets >= 3, "only {secrets} secret rooms in the sample");
    }

    /// A generator for a level that has stairs both ways, whatever the stack
    /// happens to offer level 0.
    fn with_stairs() -> Gen<'static> {
        static PARAMS: Params = Params { stairs_up: true, stairs_down: true, ..LEVEL_0 };
        Gen::new(&PARAMS, LevelId::ZERO)
    }

    #[test]
    fn stairwells_are_booths_with_one_open_side_clear_of_the_walls() {
        let g = with_stairs();
        let mut found = 0;
        // Stairwells are rare, so look over a wide area, one probe per room.
        for j in 0..600 {
            for i in 0..600 {
                let (x, y) = (i * BAY * 3, j * BAY * 3);
                let room = g.room_at(x, y);
                let Some((bay, facing, _)) = g.stair_of(&room) else { continue };
                found += 1;
                let sides = [
                    (Dir::South, Axis::EastWest, bay),
                    (Dir::North, Axis::EastWest, bay.offset(0, BAY)),
                    (Dir::West, Axis::NorthSouth, bay),
                    (Dir::East, Axis::NorthSouth, bay.offset(BAY, 0)),
                ];
                for (side, axis, at) in sides {
                    let piece = g.segment(axis, at.x, at.y);
                    let expected = if side == facing { [Edge::Open; 4] } else { [Edge::Wall; 4] };
                    assert_eq!(piece, expected, "{side:?} side of the booth at {bay:?}");
                }
                // The bay in front of the open side is in the same room.
                let (dx, dy) = facing.step();
                let front = bay.offset(dx * BAY, dy * BAY);
                assert_eq!(g.room_at(front.x, front.y).key, room.key);
                assert!(!g.pillar_at(bay.x & !7, bay.y & !7));
            }
        }
        assert!(found >= 3, "only {found} stairwells found");
    }

    #[test]
    fn a_portal_is_matched_by_a_portal_or_open_floor_at_the_far_end() {
        let g = generator();
        let (mut portals, mut two_way) = (0, 0);
        for j in 0..400 {
            for i in 0..400 {
                let (x, y) = (i * BAY, j * BAY);
                for axis in [Axis::NorthSouth, Axis::EastWest] {
                    if g.segment(axis, x, y)[1] != Edge::Portal {
                        continue;
                    }
                    portals += 1;
                    let far = super::super::portal_partner(LevelId::ZERO, CellPos::new(x, y));
                    let other = g.segment(axis, far.x, far.y);
                    // The far end rolls the same hash. It is a portal too
                    // unless something else already decided that piece (a
                    // doorway, open floor, a secret room's wall).
                    two_way += (other[1] == Edge::Portal) as u32;
                    assert_ne!(other[1], Edge::Window, "far end at {far:?} should have rolled a portal");
                    assert_ne!(other[1], Edge::Phase);
                }
            }
        }
        assert!(portals >= 10, "only {portals} portals");
        assert!(two_way >= 1, "no two-way portal among {portals}");
    }

    #[test]
    fn variants_differ_from_level_0_and_from_each_other() {
        let mut seen = Vec::new();
        for id in 1..64u8 {
            let (params, tint) = variant(LevelId::new(id).unwrap());
            assert!(params != LEVEL_0);
            assert!(params.room_limit[0] >= params.room_limit[1]);
            assert!(params.ceiling_mm >= 2200 && params.ceiling_mm <= 4400);
            assert!(tint.iter().all(|&c| c >= 150));
            seen.push((params.ceiling_mm, params.room_limit, tint));
        }
        seen.sort_unstable();
        seen.dedup();
        assert!(seen.len() >= 60, "only {} distinct variants", seen.len());
    }

    #[test]
    fn every_wall_line_has_a_doorway_unless_it_only_borders_secrets() {
        let g = generator();
        for (x, y) in sample_bays() {
            for axis in [Axis::NorthSouth, Axis::EastWest] {
                let Some(line) = g.line_between(axis, x, y) else { continue };
                let Some(place) = g.guaranteed_door(&line) else { continue };
                let along = line.lo + place * BAY;
                let at =
                    if line.vertical { line.block.offset(line.at, along) } else { line.block.offset(along, line.at) };
                let piece = g.segment(axis, at.x, at.y);
                assert!(piece[1] == Edge::Open, "no doorway at the guaranteed place {at:?}: {piece:?}");
            }
        }
    }
}

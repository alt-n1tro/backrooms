//! Generator version 1.
//!
//! **Status: draft.** v1 has not shipped, so its output may still change
//! between milestones; each change re-blesses the golden hashes on purpose.
//! From the first public release on, this module is frozen.

use crate::GeneratorVersion;
use crate::chunk::Chunk;
use crate::coord::CellPos;
use crate::coord::ChunkPos;
use crate::generator::GenError;
use crate::hash::Key;
use crate::level::{LEVEL_COUNT, LevelId, LevelInfo, Theme};
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
    pub const CARVE: u64 = 4; // retired, never reuse
    pub const PILLAR: u64 = 5;
    pub const FIXTURE: u64 = 6;
    pub const POWER: u64 = 7;
    pub const FLICKER: u64 = 8;
    pub const DAMP: u64 = 9;
    pub const WEAR: u64 = 10;
    pub const REGION_LIGHTING: u64 = 11;
    pub const FEATURE: u64 = 12;
    pub const WRONGNESS: u64 = 13;
    pub const SKEW: u64 = 14;
    pub const PORTAL: u64 = 15;
    pub const NOCLIP: u64 = 16;
    pub const VARIANT: u64 = 17;
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

/// Display names of the yellow levels.
const NUMBERS: [&str; 64] = [
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20",
    "21", "22", "23", "24", "25", "26", "27", "28", "29", "30", "31", "32", "33", "34", "35", "36", "37", "38", "39",
    "40", "41", "42", "43", "44", "45", "46", "47", "48", "49", "50", "51", "52", "53", "54", "55", "56", "57", "58",
    "59", "60", "61", "62", "63",
];

/// Ids at or below this have a generator. Levels are built in id order
/// within v1's draft, so one number says which exist.
const LAST_BUILT: u8 = 67;

/// How a level is generated in v1.
struct Profile {
    name: &'static str,
    params: level0::Params,
    theme: Theme,
    tint: [u8; 3],
}

const fn is_built(id: u8) -> bool {
    id <= LAST_BUILT
}

/// The level a stairwell on `level` leads to: the nearest built level in
/// that direction of the stack. Most of the 128 slots are still reserved,
/// and a stair into nothing would be a dead end, so empty slots are stepped
/// over.
pub fn stair_exit(level: LevelId, up: bool) -> Option<LevelId> {
    let mut depth = Z_DEPTH[level.get() as usize] as i32;
    loop {
        depth += if up { -1 } else { 1 };
        if !(0..LEVEL_COUNT as i32).contains(&depth) {
            return None;
        }
        let id = Z_ORDER[depth as usize];
        if is_built(id) {
            return LevelId::new(id);
        }
    }
}

/// The far end of a portal: the same place in a block a long way off. The
/// offset is a fixed pattern of bits flipped in each coordinate, so going
/// through twice returns to the start, and it leaves the position within
/// the 32 m block alone.
pub fn portal_partner(level: LevelId, cell: CellPos) -> CellPos {
    let roll = level_key(level, purpose::PORTAL).roll();
    // Clear the bits below a block; force one high bit so the partner is
    // never the same block (at least 500 km away).
    let flip = |bits: u64| (bits as i32 & !(level0::BLOCK - 1)) | 1 << 20;
    CellPos::new(cell.x ^ flip(roll.0), cell.y ^ flip(roll.0 >> 32))
}

/// Where a phasing wall piece leads: a hashed built level and a hashed cell.
pub fn noclip_exit(level: LevelId, cell: CellPos) -> (LevelId, CellPos) {
    let roll = level_key(level, purpose::NOCLIP).with_xy(cell.x, cell.y).roll();
    let target = LevelId::new(roll.below(LAST_BUILT as u32 + 1) as u8).expect("below the level count");
    let place = roll.next();
    (target, CellPos::new(place.0 as i32, (place.0 >> 32) as i32))
}

fn profile(level: LevelId) -> Option<Profile> {
    use crate::hash::pct;
    let id = level.get();
    if !is_built(id) {
        return None;
    }
    let base = level0::LEVEL_0;
    let (name, mut params, theme, tint) = match id {
        0 => ("0", base, Theme::Yellow, [255, 255, 255]),
        1..=63 => {
            let (params, tint) = level0::variant(level);
            (NUMBERS[id as usize], params, Theme::Yellow, tint)
        }
        64 => (
            "WAREHOUSE",
            level0::Params {
                ceiling_mm: 5200,
                room_limit: [16, 7],
                corridor: [pct(5), pct(25)],
                open_plan: [pct(45), pct(15)],
                pillars: pct(100),
                dead: [pct(10), pct(70)],
                damp_floor: pct(55),
                skew_floor: pct(97),
                ..base
            },
            Theme::Concrete,
            [205, 205, 200],
        ),
        65 => (
            "OFFICE",
            level0::Params {
                ceiling_mm: 2700,
                room_limit: [5, 2],
                corridor: [pct(70), pct(92)],
                open_plan: [pct(8), pct(1)],
                extra_door: pct(6),
                dead: [pct(1), pct(25)],
                damp_floor: pct(85),
                wear_floor: pct(70),
                ..base
            },
            Theme::Office,
            [235, 232, 222],
        ),
        66 => (
            "TUNNELS",
            level0::Params {
                ceiling_mm: 2200,
                room_limit: [3, 1],
                corridor: [pct(85), pct(95)],
                open_plan: [pct(2), pct(0)],
                pillars: pct(0),
                fixture_missing: pct(45),
                dead: [pct(20), pct(85)],
                flicker: [pct(6), pct(45)],
                damp_floor: pct(40),
                wear_floor: pct(30),
                ..base
            },
            Theme::Concrete,
            [120, 118, 112],
        ),
        _ => (
            "TILES",
            level0::Params {
                ceiling_mm: 3000,
                room_limit: [8, 4],
                corridor: [pct(15), pct(40)],
                open_plan: [pct(40), pct(15)],
                pillars: pct(40),
                damp_floor: pct(35),
                wear_floor: pct(60),
                skew_floor: pct(92),
                ..base
            },
            Theme::Tile,
            [225, 240, 238],
        ),
    };
    // A stairwell needs somewhere to go.
    params.stairs_up = stair_exit(level, true).is_some();
    params.stairs_down = stair_exit(level, false).is_some();
    Some(Profile { name, params, theme, tint })
}

pub fn level_info(level: LevelId) -> LevelInfo {
    let group = level.group();
    match profile(level) {
        None => {
            LevelInfo { id: level, group, name: "", built: false, ceiling_mm: 0, theme: Theme::Yellow, tint: [0; 3] }
        }
        Some(p) => LevelInfo {
            id: level,
            group,
            name: p.name,
            built: true,
            ceiling_mm: p.params.ceiling_mm,
            theme: p.theme,
            tint: p.tint,
        },
    }
}

pub fn generate_chunk(level: LevelId, pos: ChunkPos) -> Result<Chunk, GenError> {
    match profile(level) {
        None => Err(GenError::LevelNotBuilt(level)),
        Some(p) => Ok(level0::generate(GeneratorVersion::V1, &p.params, level, pos)),
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    #[test]
    fn portals_pair_up_far_apart_and_in_the_same_spot_of_their_blocks() {
        for level in [0u8, 17, 64] {
            let level = LevelId::new(level).unwrap();
            for cell in [CellPos::new(0, 0), CellPos::new(-777, 1234), CellPos::new(i32::MAX, i32::MIN)] {
                let partner = portal_partner(level, cell);
                assert_eq!(portal_partner(level, partner), cell);
                assert_eq!(partner.x & 63, cell.x & 63);
                assert_eq!(partner.y & 63, cell.y & 63);
                let (dx, dy) = cell.delta_to(partner);
                assert!(dx.unsigned_abs() >= 1 << 20 && dy.unsigned_abs() >= 1 << 20);
            }
        }
    }

    #[test]
    fn exits_always_lead_to_built_levels() {
        for id in 0..LEVEL_COUNT as u8 {
            let level = LevelId::new(id).unwrap();
            assert_eq!(level_info(level).built, id <= LAST_BUILT);
            for up in [true, false] {
                if let Some(to) = stair_exit(level, up) {
                    assert!(level_info(to).built && to != level);
                    // "Up" means nearer the top of the stack.
                    assert_eq!(Z_DEPTH[to.get() as usize] < Z_DEPTH[id as usize], up);
                }
            }
            let (to, _) = noclip_exit(level, CellPos::new(id as i32 * 9973, -5));
            assert!(level_info(to).built);
        }
        // Stairs that go down and then up come back.
        let start = LevelId::ZERO;
        if let Some(below) = stair_exit(start, false) {
            assert_eq!(stair_exit(below, true), Some(start));
        }
    }

    #[test]
    fn noclip_scatters_across_levels() {
        let mut seen = [false; LEVEL_COUNT];
        for i in 0..2000 {
            let (to, _) = noclip_exit(LevelId::ZERO, CellPos::new(i * 4, i * -8));
            seen[to.get() as usize] = true;
        }
        assert!(seen.iter().filter(|&&s| s).count() > 60);
    }

    #[test]
    fn every_built_level_generates_and_they_differ() {
        let mut digests = std::vec::Vec::new();
        for id in 0..=LAST_BUILT {
            let chunk = generate_chunk(LevelId::new(id).unwrap(), ChunkPos::new(3, -2)).unwrap();
            digests.push(chunk.digest());
        }
        digests.sort_unstable();
        digests.dedup();
        assert_eq!(digests.len(), LAST_BUILT as usize + 1);
    }
}

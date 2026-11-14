//! Where the integer world meets the float renderer.
//!
//! The core speaks in whole cells on a 2^32 torus. The renderer speaks in
//! `f32` metres, which only stay precise near zero. The bridge is a
//! [`RenderOrigin`]: one cell that is declared to be at world position zero.
//! Everything drawn is positioned by its (exact, integer) distance from that
//! cell, so precision depends on how far you are from the origin, never on
//! how large the coordinates are.
//!
//! Axes: the core's +x (east) is Bevy's +X, the core's +y (north) is Bevy's
//! -Z, and up is +Y. "Plan" coordinates in this crate are `Vec2(east, north)`
//! in metres from the origin.

use backrooms_core::{CellPos, Chunk, ChunkPos, Edge, GeneratorVersion, LevelId};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;

/// Edge length of a cell in metres.
pub const CELL: f32 = backrooms_core::coord::CELL_MM as f32 / 1000.0;
/// Half the thickness of a wall, in metres. The core's walls are infinitely
/// thin lines between cells; this is the body the game gives them.
pub const WALL_HALF: f32 = 0.05;

/// The cell that sits at world position zero. Always a chunk corner.
#[derive(Resource, Clone, Copy, Debug)]
pub struct RenderOrigin {
    pub version: GeneratorVersion,
    pub level: LevelId,
    pub cell: CellPos,
}

impl RenderOrigin {
    /// Plan position of the south-west corner of `cell`.
    pub fn plan_of(&self, cell: CellPos) -> Vec2 {
        // `delta_to` wraps, so this is the short way round the torus even if
        // the origin and the cell are on opposite sides of the world edge.
        let (dx, dy) = self.cell.delta_to(cell);
        Vec2::new(dx as f32, dy as f32) * CELL
    }

    /// The cell containing a plan position.
    pub fn cell_at(&self, plan: Vec2) -> CellPos {
        let cell = (plan / CELL).floor();
        self.cell.offset(cell.x as i32, cell.y as i32)
    }
}

pub fn plan_to_world(plan: Vec2, height: f32) -> Vec3 {
    Vec3::new(plan.x, height, -plan.y)
}

pub fn world_to_plan(world: Vec3) -> Vec2 {
    Vec2::new(world.x, -world.z)
}

/// The generated chunks currently held in memory.
///
/// This is only a cache: any chunk can be dropped and regenerated, and will
/// come back identical.
#[derive(Resource)]
pub struct ChunkStore {
    pub version: GeneratorVersion,
    pub level: LevelId,
    chunks: HashMap<ChunkPos, Chunk>,
}

impl ChunkStore {
    pub fn new(version: GeneratorVersion, level: LevelId) -> Self {
        Self { version, level, chunks: HashMap::default() }
    }

    /// Makes sure the chunk at `pos` is generated.
    pub fn ensure(&mut self, pos: ChunkPos) {
        self.chunks.entry(pos).or_insert_with(|| {
            backrooms_core::generate_chunk(self.version, self.level, pos)
                .expect("the game only runs on levels that are built")
        });
    }

    /// Makes sure `pos` and its eight neighbours are generated.
    pub fn ensure_around(&mut self, pos: ChunkPos) {
        for dy in -1..=1 {
            for dx in -1..=1 {
                self.ensure(pos.offset(dx, dy));
            }
        }
    }

    pub fn get(&self, pos: ChunkPos) -> Option<&Chunk> {
        self.chunks.get(&pos)
    }

    /// Drops every chunk for which `keep` returns false.
    pub fn retain(&mut self, mut keep: impl FnMut(ChunkPos) -> bool) {
        self.chunks.retain(|&pos, _| keep(pos));
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    /// A window onto the floor plan with cell `(0, 0)` at `base`.
    pub fn plan(&self, base: CellPos) -> StorePlan<'_> {
        StorePlan { store: self, base }
    }
}

/// Read access to the floor plan in coordinates relative to some base cell.
///
/// Both the mesher (base = chunk corner) and the collision code (base =
/// render origin) work through this trait, so they can never disagree about
/// where a wall is. Tests implement it with a hand-written plan.
pub trait Plan {
    /// Whether the core has a wall on the west side of cell `(x, y)`.
    fn edge_west(&self, x: i32, y: i32) -> bool;
    /// Whether the core has a wall on the south side of cell `(x, y)`.
    fn edge_south(&self, x: i32, y: i32) -> bool;
    /// Whether cell `(x, y)` is a pillar.
    fn solid(&self, x: i32, y: i32) -> bool;
    /// Moisture and wear of cell `(x, y)`, each 0.0 to 1.0.
    fn condition(&self, x: i32, y: i32) -> Vec2;

    /// A wall slab running north-south on the west side of cell `(x, y)`.
    ///
    /// The core also marks the four edges of a pillar as walls. Those are
    /// left out here: a pillar is drawn and collided with as a solid block,
    /// not as four thin slabs.
    fn wall_ns(&self, x: i32, y: i32) -> bool {
        self.edge_west(x, y) && !self.solid(x, y) && !self.solid(x - 1, y)
    }

    /// A wall slab running east-west on the south side of cell `(x, y)`.
    fn wall_ew(&self, x: i32, y: i32) -> bool {
        self.edge_south(x, y) && !self.solid(x, y) && !self.solid(x, y - 1)
    }
}

/// A [`Plan`] backed by the chunk store. Cells whose chunk is not in memory
/// read as empty floor, so callers `ensure` what they need first.
pub struct StorePlan<'a> {
    store: &'a ChunkStore,
    base: CellPos,
}

impl StorePlan<'_> {
    fn look<T: Default>(&self, x: i32, y: i32, read: impl FnOnce(&Chunk, usize, usize) -> T) -> T {
        let cell = self.base.offset(x, y);
        let (lx, ly) = cell.local();
        self.store.get(cell.chunk()).map(|chunk| read(chunk, lx, ly)).unwrap_or_default()
    }
}

impl Plan for StorePlan<'_> {
    fn edge_west(&self, x: i32, y: i32) -> bool {
        self.look(x, y, |c, lx, ly| c.west[ly][lx] == Edge::Wall)
    }

    fn edge_south(&self, x: i32, y: i32) -> bool {
        self.look(x, y, |c, lx, ly| c.south[ly][lx] == Edge::Wall)
    }

    fn solid(&self, x: i32, y: i32) -> bool {
        self.look(x, y, |c, lx, ly| c.cells[ly][lx].solid)
    }

    fn condition(&self, x: i32, y: i32) -> Vec2 {
        self.look(x, y, |c, lx, ly| {
            let cell = c.cells[ly][lx];
            Vec2::new(cell.damp as f32, cell.wear as f32) / 255.0
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin(x: i32, y: i32) -> RenderOrigin {
        RenderOrigin { version: GeneratorVersion::V1, level: LevelId::ZERO, cell: CellPos::new(x, y) }
    }

    #[test]
    fn plan_positions_are_relative_to_the_origin() {
        let o = origin(1_000_000_000, -1_000_000_000);
        assert_eq!(o.plan_of(o.cell), Vec2::ZERO);
        assert_eq!(o.plan_of(o.cell.offset(3, -2)), Vec2::new(1.5, -1.0));
        assert_eq!(o.cell_at(Vec2::new(1.74, -0.01)), o.cell.offset(3, -1));
    }

    #[test]
    fn the_origin_can_sit_on_the_world_edge() {
        let o = origin(i32::MAX - 15, i32::MAX - 15);
        // 20 cells east of the origin is past the wrap, but still 10 m away.
        let far = o.cell.offset(20, 20);
        assert!(far.x < 0);
        assert_eq!(o.plan_of(far), Vec2::splat(10.0));
        assert_eq!(o.cell_at(Vec2::splat(10.2)), far);
    }

    #[test]
    fn north_is_minus_z() {
        let world = plan_to_world(Vec2::new(2.0, 3.0), 1.0);
        assert_eq!(world, Vec3::new(2.0, 1.0, -3.0));
        assert_eq!(world_to_plan(world), Vec2::new(2.0, 3.0));
    }

    #[test]
    fn store_plan_matches_the_core() {
        let mut store = ChunkStore::new(GeneratorVersion::V1, LevelId::ZERO);
        let pos = ChunkPos::new(-3, 7);
        store.ensure_around(pos);
        assert_eq!(store.len(), 9);
        let chunk = store.get(pos).unwrap().clone();
        let plan = store.plan(pos.origin());
        for y in 0..16 {
            for x in 0..16 {
                assert_eq!(plan.edge_west(x, y), chunk.west[y as usize][x as usize] == Edge::Wall);
                assert_eq!(plan.edge_south(x, y), chunk.south[y as usize][x as usize] == Edge::Wall);
                assert_eq!(plan.solid(x, y), chunk.cells[y as usize][x as usize].solid);
            }
            // Column 16 of this chunk is column 0 of the eastern neighbour.
            assert_eq!(plan.edge_west(16, y), chunk.west[y as usize][16] == Edge::Wall);
        }
    }
}

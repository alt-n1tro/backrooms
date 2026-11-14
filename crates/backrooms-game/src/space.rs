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

use std::sync::{Arc, Mutex};

use backrooms_core::{CellPos, Chunk, ChunkPos, Edge, GeneratorVersion, LevelId, LightState};
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

/// The generated chunks currently held in memory, shared between threads.
///
/// This is only a cache: any chunk can be dropped and regenerated, and will
/// come back identical. When it is full, the chunks that have gone longest
/// without being asked for are dropped (a "least recently used" cache).
///
/// `Arc` is a reference-counted pointer that may be shared across threads;
/// cloning a `ChunkCache` or an `Arc<Chunk>` copies the pointer, not the
/// data. `Mutex` lets only one thread touch the map at a time.
#[derive(Resource, Clone)]
pub struct ChunkCache(Arc<CacheInner>);

struct CacheInner {
    version: GeneratorVersion,
    level: LevelId,
    capacity: usize,
    map: Mutex<CacheMap>,
}

#[derive(Default)]
struct CacheMap {
    /// Each chunk with the time it was last asked for.
    chunks: HashMap<ChunkPos, (Arc<Chunk>, u64)>,
    /// Counts requests; stands in for a clock.
    clock: u64,
}

impl ChunkCache {
    pub fn new(version: GeneratorVersion, level: LevelId, capacity: usize) -> Self {
        Self(Arc::new(CacheInner { version, level, capacity: capacity.max(16), map: Mutex::default() }))
    }

    /// The chunk at `pos`, generated now if it is not in memory.
    pub fn get(&self, pos: ChunkPos) -> Arc<Chunk> {
        let inner = &*self.0;
        {
            // The lock is released at the end of this block, before the slow
            // part, so other threads are not held up while a chunk generates.
            let mut map = inner.map.lock().expect("no thread panics while holding the cache");
            map.clock += 1;
            let now = map.clock;
            if let Some((chunk, used)) = map.chunks.get_mut(&pos) {
                *used = now;
                return chunk.clone();
            }
        }
        let chunk = Arc::new(
            backrooms_core::generate_chunk(inner.version, inner.level, pos)
                .expect("the game only runs on levels that are built"),
        );
        let mut map = inner.map.lock().expect("no thread panics while holding the cache");
        let now = map.clock;
        // Two threads may have generated the same chunk at once. Both results
        // are identical, so it does not matter which one is kept.
        map.chunks.insert(pos, (chunk.clone(), now));
        if map.chunks.len() > inner.capacity {
            // Drop the stalest eighth in one go, so this scan is rare.
            let mut ages: Vec<u64> = map.chunks.values().map(|&(_, used)| used).collect();
            let cut = ages.len() / 8;
            let (_, &mut threshold, _) = ages.select_nth_unstable(cut);
            map.chunks.retain(|_, &mut (_, used)| used > threshold);
        }
        chunk
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.0.map.lock().expect("no thread panics while holding the cache").chunks.len()
    }
}

/// Read access to the floor plan in coordinates relative to some base cell.
///
/// The mesher, the light baker and the collision code all work through this
/// trait, so they can never disagree about where a wall is. Tests implement
/// it with a hand-written plan.
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

/// Cells along one side of a chunk.
const N: i32 = backrooms_core::coord::CHUNK_SIZE;
/// Cells along one side of a [`LocalPlan`]: three chunks, plus one for the
/// far boundary edges.
const SPAN: i32 = 3 * N + 1;

const WEST: u8 = 1;
const SOUTH: u8 = 2;
const SOLID: u8 = 4;

/// A ceiling fixture as seen from a [`LocalPlan`].
#[derive(Clone, Copy, Debug)]
pub struct PlanFixture {
    /// South-west cell of the panel, relative to the plan's base. The panel
    /// also covers the cell east of it.
    pub cell: IVec2,
    pub state: LightState,
}

/// A flat copy of the floor plan of one chunk and its eight neighbours.
///
/// Cell `(0, 0)` is the south-west cell of the middle chunk, and cells from
/// -16 to 31 on each axis are covered. Everything is in plain arrays, so
/// lookups cost a few nanoseconds: the light baker makes hundreds of
/// thousands of them per chunk. Cells outside read as empty floor.
pub struct LocalPlan {
    /// The cell that is `(0, 0)` here.
    pub base: CellPos,
    flags: Vec<u8>,
    condition: Vec<[u8; 2]>,
    pub fixtures: Vec<PlanFixture>,
}

impl LocalPlan {
    pub fn around(cache: &ChunkCache, centre: ChunkPos) -> Self {
        let mut plan = Self {
            base: centre.origin(),
            flags: vec![0; (SPAN * SPAN) as usize],
            condition: vec![[0; 2]; (SPAN * SPAN) as usize],
            fixtures: Vec::new(),
        };
        for cy in -1..=1 {
            for cx in -1..=1 {
                let chunk = cache.get(centre.offset(cx, cy));
                let corner = IVec2::new(cx, cy) * N;
                // Index 16 of a chunk's edge arrays is its far boundary. It
                // is the same edge as index 0 of the next chunk, so writing
                // both is harmless, and on the outer rim it is the only copy.
                for ly in 0..=N {
                    for lx in 0..=N {
                        let (ux, uy) = (lx as usize, ly as usize);
                        let mut flags = 0;
                        if ly < N && chunk.west[uy][ux] == Edge::Wall {
                            flags |= WEST;
                        }
                        if lx < N && chunk.south[uy][ux] == Edge::Wall {
                            flags |= SOUTH;
                        }
                        let index = Self::index(corner.x + lx, corner.y + ly).expect("inside the plan");
                        if lx < N && ly < N {
                            let cell = chunk.cells[uy][ux];
                            if cell.solid {
                                flags |= SOLID;
                            }
                            plan.condition[index] = [cell.damp, cell.wear];
                            // A cell's own entry: replace whatever boundary
                            // copy a neighbour wrote earlier.
                            plan.flags[index] = flags;
                        } else {
                            // A boundary copy: add to it, never erase.
                            plan.flags[index] |= flags;
                        }
                    }
                }
                plan.fixtures.extend(
                    chunk
                        .fixtures()
                        .iter()
                        .map(|f| PlanFixture { cell: corner + IVec2::new(f.x as i32, f.y as i32), state: f.state }),
                );
            }
        }
        plan
    }

    fn index(x: i32, y: i32) -> Option<usize> {
        let (gx, gy) = (x + N, y + N);
        ((0..SPAN).contains(&gx) && (0..SPAN).contains(&gy)).then(|| (gy * SPAN + gx) as usize)
    }

    fn flag(&self, x: i32, y: i32, flag: u8) -> bool {
        Self::index(x, y).is_some_and(|i| self.flags[i] & flag != 0)
    }
}

impl Plan for LocalPlan {
    fn edge_west(&self, x: i32, y: i32) -> bool {
        self.flag(x, y, WEST)
    }

    fn edge_south(&self, x: i32, y: i32) -> bool {
        self.flag(x, y, SOUTH)
    }

    fn solid(&self, x: i32, y: i32) -> bool {
        self.flag(x, y, SOLID)
    }

    fn condition(&self, x: i32, y: i32) -> Vec2 {
        Self::index(x, y)
            .map_or(Vec2::ZERO, |i| Vec2::new(self.condition[i][0] as f32, self.condition[i][1] as f32) / 255.0)
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
    fn local_plan_matches_the_core() {
        let cache = ChunkCache::new(GeneratorVersion::V1, LevelId::ZERO, 64);
        for centre in [ChunkPos::new(-3, 7), ChunkPos::new(ChunkPos::MAX, ChunkPos::MIN)] {
            let plan = LocalPlan::around(&cache, centre);
            for cy in -1..=1 {
                for cx in -1..=1 {
                    let chunk = cache.get(centre.offset(cx, cy));
                    for ly in 0..16 {
                        for lx in 0..16 {
                            let (x, y) = (cx * 16 + lx, cy * 16 + ly);
                            let (ux, uy) = (lx as usize, ly as usize);
                            assert_eq!(plan.edge_west(x, y), chunk.west[uy][ux] == Edge::Wall, "west {x},{y}");
                            assert_eq!(plan.edge_south(x, y), chunk.south[uy][ux] == Edge::Wall, "south {x},{y}");
                            assert_eq!(plan.solid(x, y), chunk.cells[uy][ux].solid, "solid {x},{y}");
                        }
                        // The far boundary of the outermost chunks is covered too.
                        let (ux, far) = (ly as usize, 16usize);
                        assert_eq!(plan.edge_west(cx * 16 + 16, cy * 16 + ly), chunk.west[ux][far] == Edge::Wall);
                        assert_eq!(plan.edge_south(cx * 16 + ly, cy * 16 + 16), chunk.south[far][ux] == Edge::Wall);
                    }
                }
            }
            let expected: usize = (-1..=1)
                .flat_map(|cy| (-1..=1).map(move |cx| (cx, cy)))
                .map(|(cx, cy)| cache.get(centre.offset(cx, cy)).fixtures().len())
                .sum();
            assert_eq!(plan.fixtures.len(), expected);
            // Outside the three-by-three block everything is open floor.
            assert!(!plan.edge_west(-17, 0) && !plan.solid(40, 40));
        }
    }

    #[test]
    fn the_cache_forgets_the_oldest_chunks_and_regenerates_them_identically() {
        let cache = ChunkCache::new(GeneratorVersion::V1, LevelId::ZERO, 16);
        let first = cache.get(ChunkPos::new(0, 0));
        for i in 1..200 {
            cache.get(ChunkPos::new(i, 0));
            // Asking for a chunk keeps it fresh.
            cache.get(ChunkPos::new(1, 0));
        }
        assert!(cache.len() <= 17, "cache grew to {}", cache.len());
        // Chunk 0,0 was evicted long ago; it comes back the same.
        assert_eq!(*cache.get(ChunkPos::new(0, 0)), *first);
    }

    #[test]
    fn the_cache_is_safe_to_share_between_threads() {
        let cache = ChunkCache::new(GeneratorVersion::V1, LevelId::ZERO, 64);
        let reference: Vec<u64> = (0..40).map(|i| cache.get(ChunkPos::new(i, -i)).digest()).collect();
        std::thread::scope(|scope| {
            for t in 0..4 {
                let (cache, reference) = (cache.clone(), &reference);
                scope.spawn(move || {
                    for i in (0..40).rev().cycle().skip(t * 7).take(120) {
                        assert_eq!(cache.get(ChunkPos::new(i, -i)).digest(), reference[i as usize]);
                    }
                });
            }
        });
    }
}

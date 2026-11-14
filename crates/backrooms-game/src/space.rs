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
    /// What the core has on the west side of cell `(x, y)`.
    fn west(&self, x: i32, y: i32) -> Edge;
    /// What the core has on the south side of cell `(x, y)`.
    fn south(&self, x: i32, y: i32) -> Edge;
    /// Whether cell `(x, y)` is a pillar.
    fn solid(&self, x: i32, y: i32) -> bool;
    /// Moisture and wear of cell `(x, y)`, each 0.0 to 1.0.
    fn condition(&self, x: i32, y: i32) -> Vec2;
    /// How far the lattice point at cell corner `(x, y)` is out of true, in
    /// metres. Only asked for corners where both coordinates are multiples
    /// of four.
    fn skew(&self, x: i32, y: i32) -> Vec2;

    /// The wall slab running north-south on the west side of cell `(x, y)`,
    /// or `Edge::Open` if there is none.
    ///
    /// The core also marks the four edges of a pillar as walls. Those are
    /// left out here: a pillar is drawn and collided with as a solid block,
    /// not as four thin slabs.
    fn slab_ns(&self, x: i32, y: i32) -> Edge {
        if self.solid(x, y) || self.solid(x - 1, y) { Edge::Open } else { self.west(x, y) }
    }

    /// The wall slab running east-west on the south side of cell `(x, y)`.
    fn slab_ew(&self, x: i32, y: i32) -> Edge {
        if self.solid(x, y) || self.solid(x, y - 1) { Edge::Open } else { self.south(x, y) }
    }

    /// How far the world is pushed out of true at `at` (a position in
    /// cells), in metres.
    ///
    /// The core only says how far each point of the 2 m lattice has moved.
    /// Everything in between moves by a blend of its four surrounding
    /// lattice points, so walls stay straight between lattice points and
    /// nothing tears. Add the result to a position to get where it really
    /// is; almost everywhere it is zero.
    fn warp(&self, at: Vec2) -> Vec2 {
        let lattice = (at / LATTICE_CELLS).floor();
        let t = at / LATTICE_CELLS - lattice;
        let (x, y) = (lattice.x as i32 * LATTICE_STEP, lattice.y as i32 * LATTICE_STEP);
        let south = self.skew(x, y).lerp(self.skew(x + LATTICE_STEP, y), t.x);
        let north = self.skew(x, y + LATTICE_STEP).lerp(self.skew(x + LATTICE_STEP, y + LATTICE_STEP), t.x);
        south.lerp(north, t.y)
    }
}

/// Spacing of the wall lattice, in cells.
const LATTICE_STEP: i32 = 4;
const LATTICE_CELLS: f32 = LATTICE_STEP as f32;

/// How much of a crawl hole's height a body may fill, and what is too tall
/// to pass a given edge.
pub fn blocks(edge: Edge, body_height: f32, ceiling: f32) -> bool {
    let clearance = edge.clearance_mm((ceiling * 1000.0) as u16) as f32 / 1000.0;
    body_height > clearance
}

/// Cells along one side of a chunk.
const N: i32 = backrooms_core::coord::CHUNK_SIZE;
/// Cells along one side of a [`LocalPlan`]: three chunks, plus one for the
/// far boundary edges.
const SPAN: i32 = 3 * N + 1;
/// Lattice points along one side of a [`LocalPlan`].
const LATTICE_SPAN: i32 = 3 * N / LATTICE_STEP + 1;

/// A ceiling fixture as seen from a [`LocalPlan`].
#[derive(Clone, Copy, Debug)]
pub struct PlanFixture {
    /// South-west cell of the panel, relative to the plan's base. The panel
    /// also covers the cell east of it.
    pub cell: IVec2,
    pub state: LightState,
}

/// One cell of a [`LocalPlan`].
#[derive(Clone, Copy, Default)]
struct PlanCell {
    west: Edge,
    south: Edge,
    solid: bool,
    damp: u8,
    wear: u8,
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
    cells: Vec<PlanCell>,
    /// Lattice displacement in metres, one entry per lattice point.
    skew: Vec<Vec2>,
    pub fixtures: Vec<PlanFixture>,
}

impl LocalPlan {
    pub fn around(cache: &ChunkCache, centre: ChunkPos) -> Self {
        let mut plan = Self {
            base: centre.origin(),
            cells: vec![PlanCell::default(); (SPAN * SPAN) as usize],
            skew: vec![Vec2::ZERO; (LATTICE_SPAN * LATTICE_SPAN) as usize],
            fixtures: Vec::new(),
        };
        for cy in -1..=1 {
            for cx in -1..=1 {
                let chunk = cache.get(centre.offset(cx, cy));
                let corner = IVec2::new(cx, cy) * N;
                // Index 16 of a chunk's edge arrays is its far boundary. It
                // is the same edge as index 0 of the next chunk, so both
                // chunks write the same value there, and on the outer rim it
                // is the only copy.
                for ly in 0..=N {
                    for lx in 0..=N {
                        let (ux, uy) = (lx as usize, ly as usize);
                        let cell = &mut plan.cells[Self::index(corner.x + lx, corner.y + ly).expect("inside the plan")];
                        if ly < N {
                            cell.west = chunk.west[uy][ux];
                        }
                        if lx < N {
                            cell.south = chunk.south[uy][ux];
                        }
                        if lx < N && ly < N {
                            let core = chunk.cells[uy][ux];
                            (cell.solid, cell.damp, cell.wear) = (core.solid, core.damp, core.wear);
                        }
                    }
                }
                for (j, row) in chunk.skew.iter().enumerate() {
                    for (i, skew) in row.iter().enumerate() {
                        let (gx, gy) = ((cx + 1) * 4 + i as i32, (cy + 1) * 4 + j as i32);
                        // The core counts in centimetres.
                        plan.skew[(gy * LATTICE_SPAN + gx) as usize] =
                            Vec2::new(skew.dx as f32, skew.dy as f32) / 100.0;
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

    fn cell(&self, x: i32, y: i32) -> PlanCell {
        Self::index(x, y).map(|i| self.cells[i]).unwrap_or_default()
    }
}

impl Plan for LocalPlan {
    fn west(&self, x: i32, y: i32) -> Edge {
        self.cell(x, y).west
    }

    fn south(&self, x: i32, y: i32) -> Edge {
        self.cell(x, y).south
    }

    fn solid(&self, x: i32, y: i32) -> bool {
        self.cell(x, y).solid
    }

    fn condition(&self, x: i32, y: i32) -> Vec2 {
        let cell = self.cell(x, y);
        Vec2::new(cell.damp as f32, cell.wear as f32) / 255.0
    }

    fn skew(&self, x: i32, y: i32) -> Vec2 {
        let (gx, gy) = (x.div_euclid(LATTICE_STEP) + 4, y.div_euclid(LATTICE_STEP) + 4);
        if (0..LATTICE_SPAN).contains(&gx) && (0..LATTICE_SPAN).contains(&gy) {
            self.skew[(gy * LATTICE_SPAN + gx) as usize]
        } else {
            Vec2::ZERO
        }
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
                            assert_eq!(plan.west(x, y), chunk.west[uy][ux], "west {x},{y}");
                            assert_eq!(plan.south(x, y), chunk.south[uy][ux], "south {x},{y}");
                            assert_eq!(plan.solid(x, y), chunk.cells[uy][ux].solid, "solid {x},{y}");
                        }
                        // The far boundary of the outermost chunks is covered too.
                        let (ux, far) = (ly as usize, 16usize);
                        assert_eq!(plan.west(cx * 16 + 16, cy * 16 + ly), chunk.west[ux][far]);
                        assert_eq!(plan.south(cx * 16 + ly, cy * 16 + 16), chunk.south[far][ux]);
                    }
                    for (j, row) in chunk.skew.iter().enumerate() {
                        for (i, skew) in row.iter().enumerate() {
                            let got = plan.skew(cx * 16 + i as i32 * 4, cy * 16 + j as i32 * 4);
                            assert_eq!(got, Vec2::new(skew.dx as f32, skew.dy as f32) / 100.0);
                        }
                    }
                }
            }
            let expected: usize = (-1..=1)
                .flat_map(|cy| (-1..=1).map(move |cx| (cx, cy)))
                .map(|(cx, cy)| cache.get(centre.offset(cx, cy)).fixtures().len())
                .sum();
            assert_eq!(plan.fixtures.len(), expected);
            // Outside the three-by-three block everything is open floor.
            assert!(plan.west(-17, 0) == Edge::Open && !plan.solid(40, 40));
        }
    }

    /// A plan where one lattice point is pushed 20 cm east and 10 cm north.
    struct Bent;

    impl Plan for Bent {
        fn west(&self, _: i32, _: i32) -> Edge {
            Edge::Open
        }
        fn south(&self, _: i32, _: i32) -> Edge {
            Edge::Open
        }
        fn solid(&self, _: i32, _: i32) -> bool {
            false
        }
        fn condition(&self, _: i32, _: i32) -> Vec2 {
            Vec2::ZERO
        }
        fn skew(&self, x: i32, y: i32) -> Vec2 {
            if (x, y) == (4, 4) { Vec2::new(0.2, 0.1) } else { Vec2::ZERO }
        }
    }

    #[test]
    fn warp_is_exact_at_lattice_points_and_blends_between() {
        assert_eq!(Bent.warp(Vec2::new(4.0, 4.0)), Vec2::new(0.2, 0.1));
        assert_eq!(Bent.warp(Vec2::new(0.0, 4.0)), Vec2::ZERO);
        assert_eq!(Bent.warp(Vec2::new(8.0, 8.0)), Vec2::ZERO);
        // Halfway along a wall to the next lattice point: half the push.
        assert_eq!(Bent.warp(Vec2::new(6.0, 4.0)), Vec2::new(0.1, 0.05));
        // In the middle of a bay: a quarter.
        assert_eq!(Bent.warp(Vec2::new(2.0, 2.0)), Vec2::new(0.05, 0.025));
        // Well away from the bent point nothing moves.
        assert_eq!(Bent.warp(Vec2::new(-3.0, 13.0)), Vec2::ZERO);
    }

    #[test]
    fn bodies_fit_through_what_they_should() {
        let ceiling = 2.6;
        assert!(!blocks(Edge::Open, 1.75, ceiling));
        assert!(blocks(Edge::Wall, 0.6, ceiling));
        assert!(blocks(Edge::Window, 0.6, ceiling));
        // A crawl hole stops anyone standing or crouching, but not crawling.
        assert!(blocks(Edge::Crawl, 1.75, ceiling));
        assert!(blocks(Edge::Crawl, 1.1, ceiling));
        assert!(!blocks(Edge::Crawl, 0.6, ceiling));
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

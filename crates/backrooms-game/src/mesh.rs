//! Turns one chunk of floor plan into triangles.
//!
//! The output is a single mesh per chunk: floor, ceiling, wall slabs (with
//! their windows and crawl holes cut out) and pillars. Positions are in
//! metres relative to the chunk's south-west corner, so the same mesh is
//! valid wherever the render origin is.
//!
//! Each vertex carries four numbers in its colour: red is moisture and
//! green is wear (both from the core), blue is the baked light falling on it
//! (from [`crate::bake`]), and alpha marks plain trim such as the inside of
//! a window opening. The surface shader turns those into what you see;
//! nothing here decides what anything looks like.
//!
//! Everything is laid out on the undistorted plan first. Where the core says
//! the lattice is out of true, each vertex is pushed by [`Plan::warp`] at
//! the last moment, so skewed walls need no special handling anywhere else.

use backrooms_core::Edge;
use backrooms_core::chunk::{CRAWL_HEIGHT_MM, WINDOW_HEAD_MM, WINDOW_SILL_MM};
use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::bake::{Baker, NUDGE, ROWS};
use crate::space::{CELL, Plan, WALL_HALF, plan_to_world};

/// Cells along one side of a chunk.
const N: i32 = backrooms_core::coord::CHUNK_SIZE;
/// Baked light is stored in thousands of lux, which keeps typical values
/// between 0 and 1. The shader multiplies by the same number.
pub const LUX_PER_UNIT: f32 = 1000.0;

const CRAWL_HEIGHT: f32 = CRAWL_HEIGHT_MM as f32 / 1000.0;
const WINDOW_SILL: f32 = WINDOW_SILL_MM as f32 / 1000.0;
const WINDOW_HEAD: f32 = WINDOW_HEAD_MM as f32 / 1000.0;

/// Raw triangle data, kept separate from Bevy's `Mesh` so it can be built on
/// a worker thread and tested without a renderer.
#[derive(Default, Debug)]
pub struct Geometry {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

impl Geometry {
    pub fn into_mesh(self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
            .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// A corner of a quad: where it is on the undistorted plan (in metres), how
/// high, and what the surface is like there.
#[derive(Clone, Copy)]
struct Corner {
    plan: Vec2,
    height: f32,
    /// Moisture and wear, 0 to 1.
    condition: Vec2,
    /// Baked light in lux.
    lux: f32,
}

/// A range of heights, bottom and top, in metres.
type Span = (f32, f32);

/// The parts of a wall piece that are solid, as height ranges in metres, and
/// the opening between them if there is one.
fn profile(edge: Edge, ceiling: f32) -> (Vec<Span>, Option<Span>) {
    match edge {
        Edge::Open => (vec![], None),
        Edge::Wall => (vec![(0.0, ceiling)], None),
        Edge::Window => (vec![(0.0, WINDOW_SILL), (WINDOW_HEAD, ceiling)], Some((WINDOW_SILL, WINDOW_HEAD))),
        Edge::Crawl => (vec![(CRAWL_HEIGHT, ceiling)], Some((0.0, CRAWL_HEIGHT))),
    }
}

/// Collects triangles for one chunk.
///
/// The lifetime `'a` says the builder borrows the plan and may not outlive
/// it; `P: Plan` lets the same code run on a real chunk or a test plan, with
/// no cost at run time (the compiler makes a copy of the code for each).
struct Builder<'a, P: Plan> {
    out: Geometry,
    plan: &'a P,
    heights: [f32; ROWS],
    ceiling: f32,
}

impl<P: Plan> Builder<'_, P> {
    /// Adds a four-sided face. `corners` go around the face in either
    /// direction; the winding is fixed up here so that the face is visible
    /// from the side `facing` points to. (Triangles are only drawn when
    /// their corners appear anticlockwise on screen, so getting this wrong
    /// makes a face invisible. Checking it in one place beats reasoning
    /// about it for every wall direction.)
    ///
    /// `trim` marks plain surfaces such as the inside of an opening, which
    /// the shader paints as woodwork rather than as wallpaper or carpet.
    fn quad(&mut self, corners: [Corner; 4], facing: Vec3, trim: bool) {
        // Out-of-true lattice points drag their surroundings with them.
        let world = corners.map(|c| plan_to_world(c.plan + self.plan.warp(c.plan / CELL), c.height));
        let face = (world[1] - world[0]).cross(world[2] - world[0]);
        let same_way = face.dot(facing) >= 0.0;
        let order: [usize; 4] = if same_way { [0, 1, 2, 3] } else { [3, 2, 1, 0] };
        // On a skewed wall the true normal differs slightly from the one
        // asked for, so it is taken from the face itself.
        let normal = (if same_way { face } else { -face }).try_normalize().unwrap_or(facing);

        let first = self.out.positions.len() as u32;
        for i in order {
            let c = corners[i];
            self.out.positions.push(world[i].to_array());
            self.out.normals.push(normal.to_array());
            self.out.colors.push([c.condition.x, c.condition.y, c.lux / LUX_PER_UNIT, if trim { 0.0 } else { 1.0 }]);
        }
        self.out.indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }

    /// Light at `height`, given the light at each of the baker's heights.
    fn lux_at(&self, rows: &[f32; ROWS], height: f32) -> f32 {
        let step = self.ceiling / (ROWS - 1) as f32;
        let position = (height / step).clamp(0.0, (ROWS - 1) as f32);
        let below = (position.floor() as usize).min(ROWS - 2);
        rows[below] + (rows[below + 1] - rows[below]) * (position - below as f32)
    }

    /// An upright face from plan point `a` to plan point `b`, between two
    /// heights, as a stack of quads with a row of vertices at each height
    /// the baker reports light for. One tall quad could only blend from its
    /// bottom to its top, and would miss the bright band a panel throws high
    /// on a nearby wall.
    #[allow(clippy::too_many_arguments)]
    fn upright(
        &mut self,
        a: Vec2,
        b: Vec2,
        facing: Vec2,
        condition: [Vec2; 2],
        lux: [[f32; ROWS]; 2],
        (low, high): (f32, f32),
        trim: bool,
    ) {
        // The bottom, any baker height strictly inside the range, the top.
        let mut levels = vec![low];
        levels.extend(self.heights.iter().copied().filter(|&h| h > low + 1e-3 && h < high - 1e-3));
        levels.push(high);

        for pair in levels.windows(2) {
            let at = |end: usize, height: f32| Corner {
                plan: [a, b][end],
                height,
                condition: condition[end],
                lux: self.lux_at(&lux[end], height),
            };
            let corners = [at(0, pair[0]), at(1, pair[0]), at(1, pair[1]), at(0, pair[1])];
            self.quad(corners, plan_to_world(facing, 0.0), trim);
        }
    }
}

/// Direct light on the floor at every cell corner of the chunk, separately
/// for each of the four cells that meet there.
///
/// In the open, the four cells around a corner must agree on its brightness
/// or the floor would show a grid of faint steps, so they share the average.
/// But where a wall runs through the corner, the cells on either side of it
/// can be in different rooms, one lit and one dark, and must not be blended.
/// So the light is sampled a little way into each cell, and averaged only
/// among the cells that are connected around the corner without crossing a
/// wall.
///
/// Indexed `[y][x][quadrant]` with quadrants in the order south-west,
/// south-east, north-west, north-east.
fn corner_light<P: Plan>(plan: &P, baker: &Baker<P>) -> Vec<[f32; 4]> {
    const QUADRANTS: [(i32, i32); 4] = [(-1, -1), (0, -1), (-1, 0), (0, 0)];
    let mut out = Vec::with_capacity(((N + 1) * (N + 1)) as usize);
    for y in 0..=N {
        for x in 0..=N {
            let cell = |q: usize| (x + QUADRANTS[q].0, y + QUADRANTS[q].1);
            let mut lux = [0.0f32; 4];
            let mut open = [false; 4];
            for q in 0..4 {
                let (cx, cy) = cell(q);
                open[q] = !plan.solid(cx, cy);
                if open[q] {
                    // From the corner, a short step into this cell.
                    let step = Vec2::new(QUADRANTS[q].0 as f32 + 0.5, QUADRANTS[q].1 as f32 + 0.5) * 2.0 * NUDGE;
                    lux[q] = baker.floor_direct(Vec2::new(x as f32, y as f32) + step);
                }
            }

            // Which neighbouring quadrants are joined (no wall between).
            // The raw core edges are used, so pillar sides count as walls.
            let joined = [
                (0, 1, plan.west(x, y - 1) == Edge::Open),
                (2, 3, plan.west(x, y) == Edge::Open),
                (0, 2, plan.south(x - 1, y) == Edge::Open),
                (1, 3, plan.south(x, y) == Edge::Open),
            ];
            // Give every quadrant a group number, then merge joined groups.
            // With four items, repeating the merge three times is enough for
            // every chain to settle.
            let mut group = [0, 1, 2, 3];
            for _ in 0..3 {
                for &(a, b, is_joined) in &joined {
                    if is_joined && open[a] && open[b] {
                        let low = group[a].min(group[b]);
                        group[a] = low;
                        group[b] = low;
                    }
                }
            }
            let mut shared = [0.0f32; 4];
            for q in 0..4 {
                let members = (0..4).filter(|&m| open[m] && group[m] == group[q]);
                let (sum, count) = members.fold((0.0, 0), |(sum, count), m| (sum + lux[m], count + 1));
                shared[q] = if count > 0 { sum / count as f32 } else { 0.0 };
            }
            out.push(shared);
        }
    }
    out
}

/// Which way a wall slab runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Run {
    /// On the west side of its cell, running north.
    North,
    /// On the south side of its cell, running east.
    East,
}

/// Builds the wall piece on the west (`Run::North`) or south (`Run::East`)
/// side of cell `(x, y)`.
///
/// The two directions are the same shape turned a quarter turn, so the code
/// is written once in terms of "along" the wall and "across" it, and `at`
/// turns that back into east and north.
fn slab<P: Plan>(b: &mut Builder<P>, baker: &Baker<P>, run: Run, x: i32, y: i32, condition: [Vec2; 2]) {
    let plan = b.plan;
    // The piece `step` places further along the wall.
    let kind = |step: i32| match run {
        Run::North => plan.slab_ns(x, y + step),
        Run::East => plan.slab_ew(x + step, y),
    };
    let here = kind(0);
    if here == Edge::Open {
        return;
    }
    // Whether another wall meets this one at the given end (0 or 1), on the
    // given side (-1 or 0, as a cell offset across the wall).
    let meets = |side: i32, end: i32| match run {
        Run::North => plan.slab_ew(x + side, y + end) != Edge::Open,
        Run::East => plan.slab_ns(x + end, y + side) != Edge::Open,
    };
    let (start, line) = match run {
        Run::North => (y as f32, x as f32),
        Run::East => (x as f32, y as f32),
    };
    // A point given as cells along the wall and cells across it.
    let at = |along: f32, across: f32| match run {
        Run::North => Vec2::new(across, along),
        Run::East => Vec2::new(along, across),
    };
    let (along_axis, across_axis) = (at(1.0, 0.0), at(0.0, 1.0));
    let half = WALL_HALF / CELL;

    // Where the wall does not continue, the slab is lengthened by half its
    // thickness. At a corner that fills the little square both walls share;
    // at a doorway it forms the jamb.
    let ends = [kind(-1) == Edge::Open, kind(1) == Edge::Open];
    let from = start - if ends[0] { half } else { 0.0 };
    let to = start + 1.0 + if ends[1] { half } else { 0.0 };
    let (solid, opening) = profile(here, b.ceiling);

    // The two long faces. Light is sampled just off each face. Along an
    // unbroken wall the sample sits exactly at the join between pieces, so
    // neighbouring pieces agree. Where the wall ends, or another wall meets
    // this face, it is pulled back along the wall so that it stays on this
    // face's side of the corner.
    let mut face_lux = [[[0.0; ROWS]; 2]; 2];
    for (index, (side, sign)) in [(-1, -1.0), (0, 1.0)].into_iter().enumerate() {
        let facing = across_axis * sign;
        let off = line + sign * NUDGE;
        let pull = |end: usize| if ends[end] || meets(side, end as i32) { NUDGE } else { 0.0 };
        let lux = [baker.wall(at(start + pull(0), off), facing), baker.wall(at(start + 1.0 - pull(1), off), facing)];
        face_lux[index] = lux;
        let across = line + sign * half;
        for &range in &solid {
            b.upright(at(from, across) * CELL, at(to, across) * CELL, facing, condition, lux, range, false);
        }
    }

    // End caps where the wall stops.
    for (end, along, sign) in [(0, from, -1.0), (1, to, 1.0)] {
        if ends[end] {
            let facing = along_axis * sign;
            let lux = baker.wall(at(along + sign * NUDGE, line), facing);
            let c = [condition[end]; 2];
            for &range in &solid {
                b.upright(
                    at(along, line - half) * CELL,
                    at(along, line + half) * CELL,
                    facing,
                    c,
                    [lux, lux],
                    range,
                    false,
                );
            }
        }
    }

    // The inside of a window or crawl hole: a sill and a head across the
    // wall's thickness, and a jamb wherever the opening stops.
    let Some((low, high)) = opening else {
        return;
    };
    // The lining sees roughly what the two faces see, so it takes their
    // average.
    let lining: [[f32; ROWS]; 2] =
        std::array::from_fn(|end| std::array::from_fn(|row| (face_lux[0][end][row] + face_lux[1][end][row]) / 2.0));
    for (height, facing) in [(low, Vec3::Y), (high, Vec3::NEG_Y)] {
        // The floor is the bottom of a crawl hole, and nothing here reaches
        // the ceiling.
        if height <= 0.0 || height >= b.ceiling {
            continue;
        }
        let corner = |end: usize, across: f32| Corner {
            plan: at(start + end as f32, line + across) * CELL,
            height,
            condition: condition[end],
            lux: b.lux_at(&lining[end], height),
        };
        b.quad([corner(0, -half), corner(1, -half), corner(1, half), corner(0, half)], facing, true);
    }
    for (end, step, sign) in [(0, -1, 1.0), (1, 1, -1.0)] {
        if kind(step) != here {
            let along = start + end as f32;
            let c = [condition[end]; 2];
            let lux = [lining[end]; 2];
            // The jamb faces into the opening.
            b.upright(
                at(along, line - half) * CELL,
                at(along, line + half) * CELL,
                along_axis * sign,
                c,
                lux,
                (low, high),
                true,
            );
        }
    }
}

/// Builds the geometry of the chunk whose south-west cell is the plan's
/// `(0, 0)`. The plan must also cover the neighbouring chunks: walls
/// continuing across the boundary and lights shining in from next door are
/// looked up there.
pub fn chunk_geometry<P: Plan>(plan: &P, baker: &Baker<P>, ceiling: f32) -> Geometry {
    let mut b = Builder { out: Geometry::default(), plan, heights: baker.heights(), ceiling };

    // Surface condition at a cell corner: the average of the four cells that
    // meet there. Sampling at corners (rather than colouring whole cells)
    // lets the GPU blend smoothly across the floor, and because neighbouring
    // chunks compute the same average for a shared corner, there is no seam.
    let at_corner = |x: i32, y: i32| {
        (plan.condition(x - 1, y - 1) + plan.condition(x, y - 1) + plan.condition(x - 1, y) + plan.condition(x, y))
            / 4.0
    };
    let m = |cells: i32| cells as f32 * CELL;

    // Floor and ceiling, one quad per cell.
    let light = corner_light(plan, baker);
    let light_at = |x: i32, y: i32, quadrant: usize| light[(y * (N + 1) + x) as usize][quadrant];
    for y in 0..N {
        for x in 0..N {
            if plan.solid(x, y) {
                continue;
            }
            // The four corners of this cell, each with the light of the
            // quadrant that points back into the cell.
            let around = [(x, y, 3), (x + 1, y, 2), (x + 1, y + 1, 0), (x, y + 1, 1)];
            for (height, facing, total) in [
                (0.0, Vec3::Y, Baker::<P>::floor_total as fn(f32) -> f32),
                (ceiling, Vec3::NEG_Y, Baker::<P>::ceiling_total as fn(f32) -> f32),
            ] {
                let corners = around.map(|(cx, cy, quadrant)| Corner {
                    plan: Vec2::new(m(cx), m(cy)),
                    height,
                    condition: at_corner(cx, cy),
                    lux: total(light_at(cx, cy, quadrant)),
                });
                b.quad(corners, facing, false);
            }
        }
    }

    for y in 0..N {
        for x in 0..N {
            let (here, north, east) = (at_corner(x, y), at_corner(x, y + 1), at_corner(x + 1, y));
            slab(&mut b, baker, Run::North, x, y, [here, north]);
            slab(&mut b, baker, Run::East, x, y, [here, east]);

            // Pillar: the four sides of the cell, each lit from just outside.
            if plan.solid(x, y) {
                let (fx, fy) = (x as f32, y as f32);
                let (w, e, s, n) = (m(x), m(x + 1), m(y), m(y + 1));
                let far_corner = at_corner(x + 1, y + 1);
                let (near, far) = (NUDGE, 1.0 - NUDGE);
                let sides = [
                    (Vec2::new(w, s), Vec2::new(w, n), Vec2::NEG_X, [here, north], [(-near, near), (-near, far)]),
                    (
                        Vec2::new(e, s),
                        Vec2::new(e, n),
                        Vec2::X,
                        [east, far_corner],
                        [(1.0 + near, near), (1.0 + near, far)],
                    ),
                    (Vec2::new(w, s), Vec2::new(e, s), Vec2::NEG_Y, [here, east], [(near, -near), (far, -near)]),
                    (
                        Vec2::new(w, n),
                        Vec2::new(e, n),
                        Vec2::Y,
                        [north, far_corner],
                        [(near, 1.0 + near), (far, 1.0 + near)],
                    ),
                ];
                for (from, to, facing, condition, samples) in sides {
                    let lux = samples.map(|(dx, dy)| baker.wall(Vec2::new(fx + dx, fy + dy), facing));
                    b.upright(from, to, facing, condition, lux, (0.0, ceiling), false);
                }
            }
        }
    }

    b.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::tests::TestPlan;
    use crate::space::{ChunkCache, LocalPlan, PlanFixture};
    use backrooms_core::{ChunkPos, GeneratorVersion, LevelId, LightState};

    const H: f32 = 2.6;
    /// Quads in one upright face.
    const STACK: usize = ROWS - 1;

    fn build(plan: &TestPlan, fixtures: &[PlanFixture]) -> Geometry {
        chunk_geometry(plan, &Baker::new(plan, fixtures, H), H)
    }

    fn lamp(x: i32, y: i32) -> PlanFixture {
        PlanFixture { cell: IVec2::new(x, y), state: LightState::On }
    }

    fn triangles(g: &Geometry) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        g.indices.as_chunks::<3>().0.iter().map(|t| [0, 1, 2].map(|i| Vec3::from_array(g.positions[t[i] as usize])))
    }

    /// Every triangle must be wound so that it faces along its stored normal.
    fn assert_consistent(g: &Geometry) {
        assert_eq!(g.positions.len(), g.normals.len());
        assert_eq!(g.positions.len(), g.colors.len());
        assert_eq!(g.indices.len() % 3, 0);
        for (t, tri) in g.indices.as_chunks::<3>().0.iter().zip(triangles(g)) {
            let face = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
            assert!(face.length() > 1e-6, "degenerate triangle {tri:?}");
            let normal = Vec3::from_array(g.normals[t[0] as usize]);
            assert!(face.normalize().dot(normal) > 0.99, "triangle {tri:?} faces away from its normal {normal}");
        }
        assert!(g.colors.iter().all(|c| c.iter().all(|v| v.is_finite() && *v >= 0.0)));
    }

    /// Baked light (lux) of the floor vertex nearest to a plan position.
    fn floor_lux(g: &Geometry, east: f32, north: f32, towards: Vec2) -> f32 {
        // Several cells share each corner; take the vertex of the cell that
        // lies in the direction `towards` from it.
        let target = plan_to_world(Vec2::new(east, north), 0.0);
        let mut best = None;
        for quad in g.indices.as_chunks::<6>().0 {
            let verts: Vec<usize> = quad.iter().map(|&i| i as usize).collect();
            if g.normals[verts[0]] != [0.0, 1.0, 0.0] {
                continue;
            }
            let centre = verts.iter().map(|&v| Vec3::from_array(g.positions[v])).sum::<Vec3>() / 6.0;
            let wanted = target + plan_to_world(towards, 0.0) * CELL / 2.0;
            if centre.distance(wanted) > 0.05 {
                continue;
            }
            for &v in &verts {
                if Vec3::from_array(g.positions[v]).distance(target) < 1e-4 {
                    best = Some(g.colors[v][2] * LUX_PER_UNIT);
                }
            }
        }
        best.expect("no floor vertex there")
    }

    #[test]
    fn empty_chunk_is_floor_and_ceiling() {
        let g = build(&TestPlan::default(), &[]);
        assert_consistent(&g);
        // 256 cells, two faces each, two triangles per face.
        assert_eq!(g.indices.len() / 3, 256 * 2 * 2);
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &g.positions {
            low = low.min(Vec3::from_array(*p));
            high = high.max(Vec3::from_array(*p));
        }
        // 8 m east (+X), 8 m north (-Z), floor to ceiling.
        assert_eq!(low, Vec3::new(0.0, 0.0, -8.0));
        assert_eq!(high, Vec3::new(8.0, H, 0.0));
    }

    #[test]
    fn a_lone_wall_piece_is_a_closed_slab() {
        let plan = TestPlan { west: vec![(4, 4)], ..Default::default() };
        let g = build(&plan, &[]);
        assert_consistent(&g);
        // Two long faces and two end caps on top of the floor and ceiling.
        assert_eq!(g.indices.len() / 3, 256 * 4 + 4 * STACK * 2);
        let wall: Vec<Vec3> = g
            .positions
            .iter()
            .zip(&g.normals)
            .filter(|(_, n)| n[1] == 0.0)
            .map(|(p, _)| Vec3::from_array(*p))
            .collect();
        let east = wall.iter().map(|p| p.x).fold(f32::MIN, f32::max);
        let west = wall.iter().map(|p| p.x).fold(f32::MAX, f32::min);
        let north = wall.iter().map(|p| -p.z).fold(f32::MIN, f32::max);
        let south = wall.iter().map(|p| -p.z).fold(f32::MAX, f32::min);
        assert_eq!((west, east), (2.0 - WALL_HALF, 2.0 + WALL_HALF));
        // Lengthened by half a thickness at both free ends.
        assert_eq!((south, north), (2.0 - WALL_HALF, 2.5 + WALL_HALF));
        // And it reaches from the floor to the ceiling.
        let top = wall.iter().map(|p| p.y).fold(f32::MIN, f32::max);
        let bottom = wall.iter().map(|p| p.y).fold(f32::MAX, f32::min);
        assert_eq!((bottom, top), (0.0, H));
    }

    #[test]
    fn a_continuing_wall_has_no_caps_between_pieces() {
        let plan = TestPlan { west: vec![(4, 4), (4, 5), (4, 6)], ..Default::default() };
        let g = build(&plan, &[]);
        assert_consistent(&g);
        // Three pieces with two long faces each, but only two end caps.
        assert_eq!(g.indices.len() / 3, 256 * 4 + (3 * 2 + 2) * STACK * 2);
    }

    #[test]
    fn walls_continue_across_the_chunk_boundary_without_a_cap() {
        // The piece at y = 0 continues south into the neighbouring chunk.
        let plan = TestPlan { west: vec![(4, 0), (4, -1)], ..Default::default() };
        let g = build(&plan, &[]);
        assert_consistent(&g);
        // Only the piece inside this chunk is built: two faces and one cap.
        assert_eq!(g.indices.len() / 3, 256 * 4 + 3 * STACK * 2);
    }

    #[test]
    fn a_pillar_replaces_its_floor_with_four_sides() {
        let plan = TestPlan {
            pillars: vec![(8, 8)],
            west: vec![(8, 8), (9, 8)],
            south: vec![(8, 8), (8, 9)],
            ..Default::default()
        };
        let g = build(&plan, &[]);
        assert_consistent(&g);
        // One cell loses floor and ceiling and gains four sides; the wall
        // edges the core puts around a pillar add nothing.
        assert_eq!(g.indices.len() / 3, 255 * 4 + 4 * STACK * 2);
    }

    /// Vertices of upright, non-trim faces: the wall surfaces proper.
    fn wall_surface(g: &Geometry) -> Vec<Vec3> {
        (0..g.positions.len())
            .filter(|&i| g.normals[i][1].abs() < 0.5 && g.colors[i][3] > 0.5)
            .map(|i| Vec3::from_array(g.positions[i]))
            .collect()
    }

    #[test]
    fn a_window_is_a_hole_between_sill_and_head() {
        // Half a metre of wall, a one-metre window, half a metre of wall.
        let plan = TestPlan { west: vec![(4, 3), (4, 6)], west_windows: vec![(4, 4), (4, 5)], ..Default::default() };
        let g = build(&plan, &[]);
        assert_consistent(&g);
        // Over the window's length the wall surface stops at the sill and
        // starts again at the head: nothing in between.
        let surface = wall_surface(&g);
        let over_window: Vec<&Vec3> = surface.iter().filter(|p| -p.z > 2.0 + 1e-3 && -p.z < 3.0 - 1e-3).collect();
        assert!(!over_window.is_empty());
        for p in &over_window {
            assert!(p.y <= WINDOW_SILL + 1e-3 || p.y >= WINDOW_HEAD - 1e-3, "wall surface inside the opening at {p}");
        }
        assert!(over_window.iter().any(|p| (p.y - WINDOW_SILL).abs() < 1e-3));
        assert!(over_window.iter().any(|p| (p.y - WINDOW_HEAD).abs() < 1e-3));
        // The opening is lined with trim: a sill, a head and two jambs.
        let trim: Vec<Vec3> = (0..g.positions.len())
            .filter(|&i| g.colors[i][3] < 0.5)
            .map(|i| Vec3::from_array(g.positions[i]))
            .collect();
        assert!(!trim.is_empty());
        assert!(trim.iter().all(|p| p.y >= WINDOW_SILL - 1e-3 && p.y <= WINDOW_HEAD + 1e-3));
        assert!(trim.iter().all(|p| (p.x - 2.0).abs() <= WALL_HALF + 1e-3), "trim stays within the wall's thickness");
    }

    #[test]
    fn a_crawl_hole_is_open_at_the_floor() {
        let plan = TestPlan { west: vec![(4, 3), (4, 6)], west_crawls: vec![(4, 4), (4, 5)], ..Default::default() };
        let g = build(&plan, &[]);
        assert_consistent(&g);
        // Take the wall faces of the two crawl pieces: they start at the top
        // of the hole, not at the floor.
        let lowest = (0..g.positions.len())
            .filter(|&i| g.normals[i][0].abs() > 0.5 && g.colors[i][3] > 0.5)
            .map(|i| Vec3::from_array(g.positions[i]))
            .filter(|p| -p.z > 2.0 + 0.01 && -p.z < 3.0 - 0.01)
            .map(|p| p.y)
            .fold(f32::MAX, f32::min);
        // (No vertex lies strictly inside the two pieces' length except at
        // their shared middle, at 2.5 m.)
        assert!((lowest - CRAWL_HEIGHT).abs() < 1e-3, "crawl pieces start at {lowest} m");
    }

    #[test]
    fn skew_moves_the_geometry_with_the_lattice() {
        let push = Vec2::new(0.2, 0.1);
        let plan = TestPlan { west: vec![(4, 4), (4, 5)], skews: vec![((4, 4), push)], ..Default::default() };
        let g = build(&plan, &[]);
        assert_consistent(&g);
        // The floor corner on the bent lattice point (2 m, 2 m) has moved
        // with it, and nothing is left behind at the old place.
        let moved = plan_to_world(Vec2::splat(2.0) + push, 0.0);
        let stayed = plan_to_world(Vec2::splat(2.0), 0.0);
        let near = |target: Vec3| g.positions.iter().filter(|p| Vec3::from_array(**p).distance(target) < 1e-4).count();
        assert!(near(moved) >= 4, "floor did not follow the lattice");
        assert_eq!(near(stayed), 0);
        // The wall that starts there is no longer square to the grid.
        let slanted = (0..g.positions.len())
            .filter(|&i| g.colors[i][3] > 0.5 && g.normals[i][1].abs() < 0.5)
            .any(|i| g.normals[i][0].abs() > 0.5 && g.normals[i][2].abs() > 0.01);
        assert!(slanted, "the wall stayed square");
        // Far from the bent point nothing moved.
        assert!(near(plan_to_world(Vec2::new(6.0, 6.0), 0.0)) >= 4);
    }

    #[test]
    fn open_floor_is_lit_smoothly() {
        let g = build(&TestPlan::default(), &[lamp(7, 8)]);
        assert_consistent(&g);
        // The four cells around an open corner agree on its brightness.
        let around = [Vec2::new(1.0, 1.0), Vec2::new(-1.0, 1.0), Vec2::new(1.0, -1.0), Vec2::new(-1.0, -1.0)];
        let values = around.map(|towards| floor_lux(&g, 3.0, 3.0, towards));
        assert!(values.iter().all(|v| (v - values[0]).abs() < 1e-3), "{values:?}");
        // Brighter under the lamp than at the edge of the chunk.
        assert!(floor_lux(&g, 4.0, 4.0, around[0]) > floor_lux(&g, 0.5, 0.5, around[0]) * 1.5);
    }

    #[test]
    fn a_wall_keeps_light_out_of_the_next_room() {
        // A wall across the whole chunk on x = 8 cells, lamp on the west side.
        let plan = TestPlan { west: (-16..32).map(|y| (8, y)).collect(), ..Default::default() };
        let g = build(&plan, &[lamp(3, 8)]);
        assert_consistent(&g);
        // The same floor corner, seen from the cell on each side of the wall.
        let lit = floor_lux(&g, 4.0, 4.0, Vec2::new(-1.0, 1.0));
        let dark = floor_lux(&g, 4.0, 4.0, Vec2::new(1.0, 1.0));
        assert!(lit > 50.0, "lit side has {lit} lux");
        assert!(dark < 1.0, "dark side has {dark} lux");
    }

    #[test]
    fn real_chunks_mesh_cleanly_and_agree_on_shared_corners() {
        let cache = ChunkCache::new(GeneratorVersion::V1, LevelId::ZERO, 256);
        for (x, y) in [(0, 0), (-5, 3), (40, -17), (ChunkPos::MAX, ChunkPos::MAX)] {
            let pos = ChunkPos::new(x, y);
            let plan = LocalPlan::around(&cache, pos);
            let baker = Baker::new(&plan, &plan.fixtures, H);
            let g = chunk_geometry(&plan, &baker, H);
            assert_consistent(&g);
            assert!(g.colors.iter().all(|c| (0.0..=1.0).contains(&c[0]) && (0.0..=1.0).contains(&c[1])));

            // The chunk to the east bakes the shared boundary on its own,
            // from its own neighbourhood. Both must get the same light, or
            // there would be a visible line along every chunk edge.
            let east = LocalPlan::around(&cache, pos.offset(1, 0));
            let east_light = corner_light(&east, &Baker::new(&east, &east.fixtures, H));
            let here_light = corner_light(&plan, &baker);
            for row in 0..=16usize {
                let here = here_light[row * 17 + 16];
                let there = east_light[row * 17];
                for q in 0..4 {
                    assert!((here[q] - there[q]).abs() < 0.01, "seam at row {row}: {here:?} vs {there:?}");
                }
            }
        }
    }
}

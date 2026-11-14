//! Turns one chunk of floor plan into triangles.
//!
//! The output is a single mesh per chunk: floor, ceiling, wall slabs and
//! pillars. Positions are in metres relative to the chunk's south-west
//! corner, so the same mesh is valid wherever the render origin is.
//!
//! Each vertex carries three numbers in its colour: red is moisture and
//! green is wear (both from the core), and blue is the baked light falling
//! on it (from [`crate::bake`]). The surface shader turns those into what
//! you see; nothing here decides what anything looks like.

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

/// Raw triangle data, kept separate from Bevy's `Mesh` so it can be built on
/// a worker thread and tested without a renderer.
#[derive(Default, Debug)]
pub struct Geometry {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

/// A corner of a quad: where it is on the plan, how high, and what the
/// surface is like there.
#[derive(Clone, Copy)]
struct Corner {
    plan: Vec2,
    height: f32,
    /// Moisture and wear, 0 to 1.
    condition: Vec2,
    /// Baked light in lux.
    lux: f32,
}

impl Geometry {
    /// Adds a four-sided face. `corners` go around the face in either
    /// direction; the winding is fixed up here so that the face is visible
    /// from the side `normal` points to. (Triangles are only drawn when their
    /// corners appear anticlockwise on screen, so getting this wrong makes a
    /// face invisible. Checking it in one place beats reasoning about it for
    /// every wall direction.)
    fn quad(&mut self, corners: [Corner; 4], normal: Vec3) {
        let world = corners.map(|c| plan_to_world(c.plan, c.height));
        let facing = (world[1] - world[0]).cross(world[2] - world[0]).dot(normal);
        let order: [usize; 4] = if facing >= 0.0 { [0, 1, 2, 3] } else { [3, 2, 1, 0] };

        let first = self.positions.len() as u32;
        for i in order {
            self.positions.push(world[i].to_array());
            self.normals.push(normal.to_array());
            let c = corners[i];
            self.colors.push([c.condition.x, c.condition.y, c.lux / LUX_PER_UNIT, 1.0]);
        }
        self.indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }

    /// An upright face from plan point `a` to plan point `b`, floor to
    /// ceiling, as a stack of quads with a row of vertices at each height
    /// the baker reports light for. One tall quad could only blend from its
    /// bottom to its top, and would miss the bright band a panel throws high
    /// on a nearby wall.
    #[allow(clippy::too_many_arguments)]
    fn upright(
        &mut self,
        heights: &[f32; ROWS],
        a: Vec2,
        b: Vec2,
        normal: Vec2,
        condition: [Vec2; 2],
        lux: [[f32; ROWS]; 2],
    ) {
        for row in 0..ROWS - 1 {
            let at = |end: usize, row: usize| Corner {
                plan: [a, b][end],
                height: heights[row],
                condition: condition[end],
                lux: lux[end][row],
            };
            self.quad([at(0, row), at(1, row), at(1, row + 1), at(0, row + 1)], plan_to_world(normal, 0.0));
        }
    }

    pub fn into_mesh(self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
            .with_inserted_indices(Indices::U32(self.indices))
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
                (0, 1, !plan.edge_west(x, y - 1)),
                (2, 3, !plan.edge_west(x, y)),
                (0, 2, !plan.edge_south(x - 1, y)),
                (1, 3, !plan.edge_south(x, y)),
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

/// Builds the geometry of the chunk whose south-west cell is the plan's
/// `(0, 0)`. The plan must also cover the neighbouring chunks: walls
/// continuing across the boundary and lights shining in from next door are
/// looked up there.
pub fn chunk_geometry<P: Plan>(plan: &P, baker: &Baker<P>, ceiling: f32) -> Geometry {
    let mut g = Geometry::default();
    let heights = baker.heights();

    // Surface condition at a cell corner: the average of the four cells that
    // meet there. Sampling at corners (rather than colouring whole cells)
    // lets the GPU blend smoothly across the floor, and because neighbouring
    // chunks compute the same average for a shared corner, there is no seam.
    let at_corner = |x: i32, y: i32| {
        (plan.condition(x - 1, y - 1) + plan.condition(x, y - 1) + plan.condition(x - 1, y) + plan.condition(x, y))
            / 4.0
    };
    let m = |cells: i32| cells as f32 * CELL;
    let cells = |x: f32, y: f32| Vec2::new(x, y);

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
            for (height, normal, total) in [
                (0.0, Vec3::Y, Baker::<P>::floor_total as fn(f32) -> f32),
                (ceiling, Vec3::NEG_Y, Baker::<P>::ceiling_total as fn(f32) -> f32),
            ] {
                let corners = around.map(|(cx, cy, quadrant)| Corner {
                    plan: Vec2::new(m(cx), m(cy)),
                    height,
                    condition: at_corner(cx, cy),
                    lux: total(light_at(cx, cy, quadrant)),
                });
                g.quad(corners, normal);
            }
        }
    }

    for y in 0..N {
        for x in 0..N {
            let (ca, cb_n, cb_e) = (at_corner(x, y), at_corner(x, y + 1), at_corner(x + 1, y));
            let (fx, fy) = (x as f32, y as f32);

            // Wall slab on the west side of the cell, running north.
            if plan.wall_ns(x, y) {
                let line = m(x);
                // Where the wall does not continue, the slab is lengthened by
                // half its thickness. At a corner that fills the little square
                // both walls share; at a doorway it forms the jamb.
                let ends_south = !plan.wall_ns(x, y - 1);
                let ends_north = !plan.wall_ns(x, y + 1);
                let south = m(y) - if ends_south { WALL_HALF } else { 0.0 };
                let north = m(y + 1) + if ends_north { WALL_HALF } else { 0.0 };
                let (w, e) = (line - WALL_HALF, line + WALL_HALF);

                // Light is sampled just off each face. Along an unbroken
                // wall the sample sits exactly at the join between pieces,
                // so neighbouring pieces agree. Where the wall ends, or
                // another wall meets this face, it is pulled back along the
                // wall so that it stays on this face's side of the corner.
                for (face, side, normal) in [(w, -1, Vec2::NEG_X), (e, 0, Vec2::X)] {
                    let off = fx + if side < 0 { -NUDGE } else { NUDGE };
                    let pull_south = if ends_south || plan.wall_ew(x + side, y) { NUDGE } else { 0.0 };
                    let pull_north = if ends_north || plan.wall_ew(x + side, y + 1) { NUDGE } else { 0.0 };
                    let lux = [
                        baker.wall(cells(off, fy + pull_south), normal),
                        baker.wall(cells(off, fy + 1.0 - pull_north), normal),
                    ];
                    g.upright(&heights, Vec2::new(face, south), Vec2::new(face, north), normal, [ca, cb_n], lux);
                }
                if ends_south {
                    let lux = baker.wall(cells(fx, fy - NUDGE), Vec2::NEG_Y);
                    g.upright(&heights, Vec2::new(w, south), Vec2::new(e, south), Vec2::NEG_Y, [ca, ca], [lux, lux]);
                }
                if ends_north {
                    let lux = baker.wall(cells(fx, fy + 1.0 + NUDGE), Vec2::Y);
                    g.upright(&heights, Vec2::new(w, north), Vec2::new(e, north), Vec2::Y, [cb_n, cb_n], [lux, lux]);
                }
            }

            // Wall slab on the south side of the cell, running east.
            if plan.wall_ew(x, y) {
                let line = m(y);
                let ends_west = !plan.wall_ew(x - 1, y);
                let ends_east = !plan.wall_ew(x + 1, y);
                let west = m(x) - if ends_west { WALL_HALF } else { 0.0 };
                let east = m(x + 1) + if ends_east { WALL_HALF } else { 0.0 };
                let (s, n) = (line - WALL_HALF, line + WALL_HALF);

                for (face, side, normal) in [(s, -1, Vec2::NEG_Y), (n, 0, Vec2::Y)] {
                    let off = fy + if side < 0 { -NUDGE } else { NUDGE };
                    let pull_west = if ends_west || plan.wall_ns(x, y + side) { NUDGE } else { 0.0 };
                    let pull_east = if ends_east || plan.wall_ns(x + 1, y + side) { NUDGE } else { 0.0 };
                    let lux = [
                        baker.wall(cells(fx + pull_west, off), normal),
                        baker.wall(cells(fx + 1.0 - pull_east, off), normal),
                    ];
                    g.upright(&heights, Vec2::new(west, face), Vec2::new(east, face), normal, [ca, cb_e], lux);
                }
                if ends_west {
                    let lux = baker.wall(cells(fx - NUDGE, fy), Vec2::NEG_X);
                    g.upright(&heights, Vec2::new(west, s), Vec2::new(west, n), Vec2::NEG_X, [ca, ca], [lux, lux]);
                }
                if ends_east {
                    let lux = baker.wall(cells(fx + 1.0 + NUDGE, fy), Vec2::X);
                    g.upright(&heights, Vec2::new(east, s), Vec2::new(east, n), Vec2::X, [cb_e, cb_e], [lux, lux]);
                }
            }

            // Pillar: the four sides of the cell, each lit from just outside.
            if plan.solid(x, y) {
                let (w, e, s, n) = (m(x), m(x + 1), m(y), m(y + 1));
                let cc = at_corner(x + 1, y + 1);
                let (near, far) = (NUDGE, 1.0 - NUDGE);
                let sides = [
                    (Vec2::new(w, s), Vec2::new(w, n), Vec2::NEG_X, [ca, cb_n], [(-near, near), (-near, far)]),
                    (Vec2::new(e, s), Vec2::new(e, n), Vec2::X, [cb_e, cc], [(1.0 + near, near), (1.0 + near, far)]),
                    (Vec2::new(w, s), Vec2::new(e, s), Vec2::NEG_Y, [ca, cb_e], [(near, -near), (far, -near)]),
                    (Vec2::new(w, n), Vec2::new(e, n), Vec2::Y, [cb_n, cc], [(near, 1.0 + near), (far, 1.0 + near)]),
                ];
                for (a, b, normal, condition, samples) in sides {
                    let lux = samples.map(|(dx, dy)| baker.wall(cells(fx + dx, fy + dy), normal));
                    g.upright(&heights, a, b, normal, condition, lux);
                }
            }
        }
    }

    g
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
        let plan = TestPlan { pillars: vec![(8, 8)], west: vec![(8, 8), (9, 8)], south: vec![(8, 8), (8, 9)] };
        let g = build(&plan, &[]);
        assert_consistent(&g);
        // One cell loses floor and ceiling and gains four sides; the wall
        // edges the core puts around a pillar add nothing.
        assert_eq!(g.indices.len() / 3, 255 * 4 + 4 * STACK * 2);
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

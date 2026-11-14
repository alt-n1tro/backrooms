//! Turns one chunk of floor plan into triangles.
//!
//! The output is a single mesh per chunk: floor, ceiling, wall slabs and
//! pillars. Positions are in metres relative to the chunk's south-west
//! corner, so the same mesh is valid wherever the render origin is.
//!
//! Each vertex carries the surface condition from the core in its colour:
//! red is moisture and green is wear. The surface shader turns that into
//! stains; nothing here decides what anything looks like.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::space::{CELL, Plan, WALL_HALF, plan_to_world};

/// Cells along one side of a chunk.
const N: i32 = backrooms_core::coord::CHUNK_SIZE;

/// Raw triangle data, kept separate from Bevy's `Mesh` so it can be built and
/// tested without a renderer.
#[derive(Default, Debug)]
pub struct Geometry {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

/// A corner of a quad: where it is on the plan and how high.
#[derive(Clone, Copy)]
struct Corner {
    plan: Vec2,
    height: f32,
}

fn corner(east: f32, north: f32, height: f32) -> Corner {
    Corner { plan: Vec2::new(east, north), height }
}

impl Geometry {
    /// Adds a four-sided face. `corners` go around the face in either
    /// direction; the winding is fixed up here so that the face is visible
    /// from the side `normal` points to. (Triangles are only drawn when their
    /// corners appear anticlockwise on screen, so getting this wrong makes a
    /// face invisible. Checking it in one place beats reasoning about it for
    /// every wall direction.)
    fn quad(&mut self, corners: [Corner; 4], normal: Vec3, condition: [Vec2; 4]) {
        let world = corners.map(|c| plan_to_world(c.plan, c.height));
        let facing = (world[1] - world[0]).cross(world[2] - world[0]).dot(normal);
        let order: [usize; 4] = if facing >= 0.0 { [0, 1, 2, 3] } else { [3, 2, 1, 0] };

        let first = self.positions.len() as u32;
        for i in order {
            self.positions.push(world[i].to_array());
            self.normals.push(normal.to_array());
            self.colors.push([condition[i].x, condition[i].y, 0.0, 1.0]);
        }
        self.indices.extend([first, first + 1, first + 2, first, first + 2, first + 3]);
    }

    pub fn into_mesh(self) -> Mesh {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
            .with_inserted_indices(Indices::U32(self.indices))
    }
}

/// Builds the geometry of the chunk whose south-west cell is the plan's
/// `(0, 0)`. The plan must also cover one cell beyond the chunk on every
/// side, which is where walls continuing into neighbours are looked up.
pub fn chunk_geometry(plan: &impl Plan, ceiling: f32) -> Geometry {
    let mut g = Geometry::default();

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
    for y in 0..N {
        for x in 0..N {
            if plan.solid(x, y) {
                continue;
            }
            let condition = [at_corner(x, y), at_corner(x + 1, y), at_corner(x + 1, y + 1), at_corner(x, y + 1)];
            for (height, normal) in [(0.0, Vec3::Y), (ceiling, Vec3::NEG_Y)] {
                let corners = [
                    corner(m(x), m(y), height),
                    corner(m(x + 1), m(y), height),
                    corner(m(x + 1), m(y + 1), height),
                    corner(m(x), m(y + 1), height),
                ];
                g.quad(corners, normal, condition);
            }
        }
    }

    // A vertical face from plan point `a` to plan point `b`.
    let upright = |g: &mut Geometry, a: Vec2, b: Vec2, normal: Vec2, ca: Vec2, cb: Vec2| {
        let corners =
            [corner(a.x, a.y, 0.0), corner(b.x, b.y, 0.0), corner(b.x, b.y, ceiling), corner(a.x, a.y, ceiling)];
        g.quad(corners, plan_to_world(normal, 0.0), [ca, cb, cb, ca]);
    };

    for y in 0..N {
        for x in 0..N {
            let (ca, cb_n, cb_e) = (at_corner(x, y), at_corner(x, y + 1), at_corner(x + 1, y));

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
                upright(&mut g, Vec2::new(w, south), Vec2::new(w, north), Vec2::NEG_X, ca, cb_n);
                upright(&mut g, Vec2::new(e, south), Vec2::new(e, north), Vec2::X, ca, cb_n);
                if ends_south {
                    upright(&mut g, Vec2::new(w, south), Vec2::new(e, south), Vec2::NEG_Y, ca, ca);
                }
                if ends_north {
                    upright(&mut g, Vec2::new(w, north), Vec2::new(e, north), Vec2::Y, cb_n, cb_n);
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
                upright(&mut g, Vec2::new(west, s), Vec2::new(east, s), Vec2::NEG_Y, ca, cb_e);
                upright(&mut g, Vec2::new(west, n), Vec2::new(east, n), Vec2::Y, ca, cb_e);
                if ends_west {
                    upright(&mut g, Vec2::new(west, s), Vec2::new(west, n), Vec2::NEG_X, ca, ca);
                }
                if ends_east {
                    upright(&mut g, Vec2::new(east, s), Vec2::new(east, n), Vec2::X, cb_e, cb_e);
                }
            }

            // Pillar: the four sides of the cell.
            if plan.solid(x, y) {
                let (w, e, s, n) = (m(x), m(x + 1), m(y), m(y + 1));
                let cc = at_corner(x + 1, y + 1);
                upright(&mut g, Vec2::new(w, s), Vec2::new(w, n), Vec2::NEG_X, ca, cb_n);
                upright(&mut g, Vec2::new(e, s), Vec2::new(e, n), Vec2::X, cb_e, cc);
                upright(&mut g, Vec2::new(w, s), Vec2::new(e, s), Vec2::NEG_Y, ca, cb_e);
                upright(&mut g, Vec2::new(w, n), Vec2::new(e, n), Vec2::Y, cb_n, cc);
            }
        }
    }

    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::tests::TestPlan;

    const H: f32 = 2.6;

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
    }

    #[test]
    fn empty_chunk_is_floor_and_ceiling() {
        let g = chunk_geometry(&TestPlan::default(), H);
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
        let g = chunk_geometry(&plan, H);
        assert_consistent(&g);
        // Two long faces and two end caps on top of the floor and ceiling.
        assert_eq!(g.indices.len() / 3, 256 * 4 + 4 * 2);
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
    }

    #[test]
    fn a_continuing_wall_has_no_caps_between_pieces() {
        let plan = TestPlan { west: vec![(4, 4), (4, 5), (4, 6)], ..Default::default() };
        let g = chunk_geometry(&plan, H);
        assert_consistent(&g);
        // Three pieces with two long faces each, but only two end caps.
        assert_eq!(g.indices.len() / 3, 256 * 4 + (3 * 2 + 2) * 2);
    }

    #[test]
    fn walls_continue_across_the_chunk_boundary_without_a_cap() {
        // The piece at y = 0 continues south into the neighbouring chunk.
        let plan = TestPlan { west: vec![(4, 0), (4, -1)], ..Default::default() };
        let g = chunk_geometry(&plan, H);
        assert_consistent(&g);
        // Only the piece inside this chunk is built: two faces and one cap.
        assert_eq!(g.indices.len() / 3, 256 * 4 + 3 * 2);
    }

    #[test]
    fn a_pillar_replaces_its_floor_with_four_sides() {
        let plan = TestPlan { pillars: vec![(8, 8)], west: vec![(8, 8), (9, 8)], south: vec![(8, 8), (8, 9)] };
        let g = chunk_geometry(&plan, H);
        assert_consistent(&g);
        // One cell loses floor and ceiling and gains four sides; the wall
        // edges the core puts around a pillar add nothing.
        assert_eq!(g.indices.len() / 3, 255 * 4 + 4 * 2);
    }

    #[test]
    fn real_chunks_mesh_cleanly() {
        use crate::space::ChunkStore;
        use backrooms_core::{ChunkPos, GeneratorVersion, LevelId};
        let mut store = ChunkStore::new(GeneratorVersion::V1, LevelId::ZERO);
        for (x, y) in [(0, 0), (-5, 3), (40, -17), (ChunkPos::MAX, ChunkPos::MAX)] {
            let pos = ChunkPos::new(x, y);
            store.ensure_around(pos);
            let g = chunk_geometry(&store.plan(pos.origin()), H);
            assert_consistent(&g);
            assert!(g.colors.iter().all(|c| (0.0..=1.0).contains(&c[0]) && (0.0..=1.0).contains(&c[1])));
        }
    }
}

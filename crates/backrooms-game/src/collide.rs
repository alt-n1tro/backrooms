//! Keeping the player out of the walls.
//!
//! The player is a circle on the floor plan. Walls are thin rectangles and
//! pillars are squares, exactly as the mesher draws them. Movement is "move,
//! then push out of anything overlapped", which makes the player slide along
//! walls instead of stopping dead.

use bevy::math::Vec2;

use crate::space::{CELL, Plan, WALL_HALF, blocks};

/// Longest distance moved before re-checking for collisions. Must be well
/// under the wall thickness plus the player radius, or a fast player could
/// step clean through a wall between two checks.
const MAX_STEP: f32 = 0.1;

/// The player's body for collision: a circle on the plan, with a height.
/// The height decides what counts as an obstacle: a crawl hole stops a
/// standing body and lets a crawling one through.
#[derive(Clone, Copy, Debug)]
pub struct Body {
    pub radius: f32,
    pub height: f32,
    /// Ceiling height of the level, which is the clearance of an open edge.
    pub ceiling: f32,
}

/// Moves `body` from `from` by `delta`, sliding along obstacles. Positions
/// are plan coordinates relative to the plan's base.
pub fn slide(plan: &impl Plan, from: Vec2, delta: Vec2, body: Body) -> Vec2 {
    let steps = (delta.length() / MAX_STEP).ceil().max(1.0) as u32;
    let step = delta / steps as f32;
    let mut pos = from;
    for _ in 0..steps {
        pos += step;
        // Being pushed out of one wall can push the circle into another (in
        // a corner), so resolve a few times.
        for _ in 0..3 {
            pos = push_out(plan, pos, body);
        }
    }
    pos
}

/// Pushes the circle out of every obstacle near it, once.
fn push_out(plan: &impl Plan, mut pos: Vec2, body: Body) -> Vec2 {
    let radius = body.radius;
    let solid_ns = |x: i32, y: i32| blocks(plan.slab_ns(x, y), body.height, body.ceiling);
    let solid_ew = |x: i32, y: i32| blocks(plan.slab_ew(x, y), body.height, body.ceiling);
    let cell = (pos / CELL).floor();
    let (cx, cy) = (cell.x as i32, cell.y as i32);
    // Anything further than one cell away cannot touch a circle smaller than
    // a cell.
    for y in cy - 1..=cy + 2 {
        for x in cx - 1..=cx + 2 {
            let corner = Vec2::new(x as f32, y as f32) * CELL;
            // A wall is stored as a row of half-metre pieces. If each piece
            // were its own box, a circle sliding along the wall would catch
            // on the corners where two pieces meet and be shoved sideways.
            // So where the wall carries on, the box is stretched a whole
            // cell that way: the circle then always meets a flat face. Where
            // the wall ends, the box gets the same half-thickness cap as the
            // drawn wall.
            let reach = |continues: bool| if continues { CELL } else { WALL_HALF };
            if solid_ns(x, y) {
                let min = corner - Vec2::new(WALL_HALF, reach(solid_ns(x, y - 1)));
                let max = corner + Vec2::new(WALL_HALF, CELL + reach(solid_ns(x, y + 1)));
                pos = push_out_of_box(pos, radius, min, max);
            }
            if solid_ew(x, y) {
                let min = corner - Vec2::new(reach(solid_ew(x - 1, y)), WALL_HALF);
                let max = corner + Vec2::new(CELL + reach(solid_ew(x + 1, y)), WALL_HALF);
                pos = push_out_of_box(pos, radius, min, max);
            }
            if plan.solid(x, y) {
                pos = push_out_of_box(pos, radius, corner, corner + Vec2::splat(CELL));
            }
        }
    }
    pos
}

/// If the circle overlaps the box `min..max`, moves it the shortest way out.
fn push_out_of_box(pos: Vec2, radius: f32, min: Vec2, max: Vec2) -> Vec2 {
    let nearest = pos.clamp(min, max);
    let away = pos - nearest;
    let distance = away.length();
    if distance >= radius {
        return pos;
    }
    if distance > 1e-6 {
        return nearest + away / distance * radius;
    }
    // The centre is inside the box: leave through the nearest side.
    let to_min = pos - min;
    let to_max = max - pos;
    let nearest_side = to_min.x.min(to_max.x).min(to_min.y).min(to_max.y);
    if nearest_side == to_min.x {
        Vec2::new(min.x - radius, pos.y)
    } else if nearest_side == to_max.x {
        Vec2::new(max.x + radius, pos.y)
    } else if nearest_side == to_min.y {
        Vec2::new(pos.x, min.y - radius)
    } else {
        Vec2::new(pos.x, max.y + radius)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use backrooms_core::Edge;

    /// A hand-written floor plan for tests.
    #[derive(Default)]
    pub struct TestPlan {
        /// Cells with a plain wall on their west side.
        pub west: Vec<(i32, i32)>,
        /// Cells with a plain wall on their south side.
        pub south: Vec<(i32, i32)>,
        pub pillars: Vec<(i32, i32)>,
        /// Cells whose west side has a window, or a crawl hole.
        pub west_windows: Vec<(i32, i32)>,
        pub west_crawls: Vec<(i32, i32)>,
        /// Lattice points that are out of true, and by how many metres.
        pub skews: Vec<((i32, i32), Vec2)>,
    }

    impl Plan for TestPlan {
        fn west(&self, x: i32, y: i32) -> Edge {
            if self.west_windows.contains(&(x, y)) {
                Edge::Window
            } else if self.west_crawls.contains(&(x, y)) {
                Edge::Crawl
            } else if self.west.contains(&(x, y)) {
                Edge::Wall
            } else {
                Edge::Open
            }
        }
        fn south(&self, x: i32, y: i32) -> Edge {
            if self.south.contains(&(x, y)) { Edge::Wall } else { Edge::Open }
        }
        fn solid(&self, x: i32, y: i32) -> bool {
            self.pillars.contains(&(x, y))
        }
        fn condition(&self, _: i32, _: i32) -> Vec2 {
            Vec2::ZERO
        }
        fn skew(&self, x: i32, y: i32) -> Vec2 {
            self.skews.iter().find(|(at, _)| *at == (x, y)).map_or(Vec2::ZERO, |&(_, push)| push)
        }
    }

    /// A standing player.
    const R: Body = Body { radius: 0.22, height: 1.75, ceiling: 2.6 };
    const RADIUS: f32 = R.radius;

    #[test]
    fn open_floor_does_not_interfere() {
        let end = slide(&TestPlan::default(), Vec2::new(0.3, 0.3), Vec2::new(5.0, -2.0), R);
        assert!(end.distance(Vec2::new(5.3, -1.7)) < 1e-4);
    }

    #[test]
    fn a_wall_stops_you_and_lets_you_slide() {
        // A long north-south wall on the line x = 4 cells (2 m).
        let plan = TestPlan { west: (-10..10).map(|y| (4, y)).collect(), ..Default::default() };
        let end = slide(&plan, Vec2::new(1.0, 0.25), Vec2::new(3.0, 1.0), R);
        // Stopped at the wall face, but the northward part of the move went through.
        assert!((end.x - (2.0 - WALL_HALF - RADIUS)).abs() < 1e-3, "x = {}", end.x);
        assert!((end.y - 1.25).abs() < 1e-3, "y = {}", end.y);
    }

    #[test]
    fn sliding_along_a_wall_neither_speeds_up_nor_snags() {
        let plan = TestPlan { west: (-40..40).map(|y| (4, y)).collect(), ..Default::default() };
        // Pressed diagonally into the wall for many small frames.
        let mut pos = Vec2::new(1.0, 0.25);
        for _ in 0..600 {
            pos = slide(&plan, pos, Vec2::new(0.02, 0.02), R);
        }
        assert!((pos.x - (2.0 - WALL_HALF - RADIUS)).abs() < 1e-3, "x = {}", pos.x);
        assert!((pos.y - 12.25).abs() < 1e-2, "y = {}", pos.y);
    }

    #[test]
    fn running_fast_does_not_tunnel() {
        let plan = TestPlan { west: (-10..10).map(|y| (4, y)).collect(), ..Default::default() };
        // 30 m in one frame.
        let end = slide(&plan, Vec2::new(1.0, 0.25), Vec2::new(30.0, 0.0), R);
        assert!(end.x < 2.0, "went through the wall to x = {}", end.x);
    }

    #[test]
    fn a_one_metre_doorway_is_passable() {
        // Wall along y = 0 with a two-cell gap at x = 0..2 cells.
        let plan = TestPlan {
            south: (-10..10).filter(|x| !(0..2).contains(x)).map(|x| (x, 0)).collect(),
            ..Default::default()
        };
        let end = slide(&plan, Vec2::new(0.5, -1.0), Vec2::new(0.0, 2.0), R);
        assert!(end.distance(Vec2::new(0.5, 1.0)) < 1e-3, "ended at {end}");
        // Off-centre, the jamb nudges you into the opening rather than snagging.
        let end = slide(&plan, Vec2::new(0.15, -1.0), Vec2::new(0.0, 2.0), R);
        assert!(end.y > 0.9, "snagged on the door jamb at {end}");
    }

    #[test]
    fn corners_hold() {
        // An inside corner: wall on x = 0 and wall on y = 0, player north-east of it.
        let plan = TestPlan {
            west: (0..10).map(|y| (0, y)).collect(),
            south: (0..10).map(|x| (x, 0)).collect(),
            ..Default::default()
        };
        let end = slide(&plan, Vec2::new(1.0, 1.0), Vec2::new(-5.0, -5.0), R);
        let limit = WALL_HALF + RADIUS;
        assert!((end.x - limit).abs() < 1e-3 && (end.y - limit).abs() < 1e-3, "ended at {end}");
    }

    #[test]
    fn pillars_are_solid_blocks() {
        // The core marks a pillar's edges as walls too; that must not make it fatter.
        let plan = TestPlan {
            pillars: vec![(0, 0)],
            west: vec![(0, 0), (1, 0)],
            south: vec![(0, 0), (0, 1)],
            ..Default::default()
        };
        let end = slide(&plan, Vec2::new(-2.0, 0.25), Vec2::new(4.0, 0.0), R);
        assert!((end.x - (0.0 - RADIUS)).abs() < 1e-3, "x = {}", end.x);
        // Walking past it, just clear of the side, is not blocked.
        let end = slide(&plan, Vec2::new(-2.0, 0.5 + RADIUS + 0.01), Vec2::new(4.0, 0.0), R);
        assert!((end.x - 2.0).abs() < 1e-3, "x = {}", end.x);
    }

    #[test]
    fn a_crawl_hole_passes_only_a_crawling_body() {
        // A long wall on x = 4 with a two-cell crawl hole at y = 0..2.
        let plan = TestPlan {
            west: (-10..10).filter(|y| !(0..2).contains(y)).map(|y| (4, y)).collect(),
            west_crawls: vec![(4, 0), (4, 1)],
            ..Default::default()
        };
        let through = |height: f32| {
            let body = Body { height, ..R };
            slide(&plan, Vec2::new(1.0, 0.5), Vec2::new(2.0, 0.0), body).x
        };
        assert!(through(1.75) < 2.0, "walked through a crawl hole");
        assert!(through(1.1) < 2.0, "crouched through a crawl hole");
        assert!((through(0.6) - 3.0).abs() < 1e-3, "could not crawl through");
    }

    #[test]
    fn a_window_stops_everyone() {
        let plan = TestPlan {
            west: (-10..10).filter(|y| !(0..2).contains(y)).map(|y| (4, y)).collect(),
            west_windows: vec![(4, 0), (4, 1)],
            ..Default::default()
        };
        for height in [1.75, 1.1, 0.6] {
            let end = slide(&plan, Vec2::new(1.0, 0.5), Vec2::new(2.0, 0.0), Body { height, ..R });
            assert!(end.x < 2.0, "a body {height} m tall went through a window");
        }
    }

    #[test]
    fn starting_inside_a_wall_gets_you_out() {
        let plan = TestPlan { west: vec![(0, 0)], ..Default::default() };
        let end = slide(&plan, Vec2::new(0.01, 0.25), Vec2::ZERO, R);
        assert!(end.x >= WALL_HALF + RADIUS - 1e-3, "still inside at {end}");
    }
}

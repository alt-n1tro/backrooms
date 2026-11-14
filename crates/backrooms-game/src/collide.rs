//! Keeping the player out of the walls.
//!
//! The player is a circle on the floor plan. Walls are thin rectangles and
//! pillars are squares, exactly as the mesher draws them. Movement is "move,
//! then push out of anything overlapped", which makes the player slide along
//! walls instead of stopping dead.

use backrooms_core::Edge;
use bevy::math::{IVec2, Vec2};

#[cfg(test)]
use crate::space::PlanStair;
use crate::space::{CELL, Plan, WALL_HALF, blocks};

/// Longest distance moved before re-checking for collisions. Must be well
/// under the wall thickness plus the player radius, or a fast player could
/// step clean through a wall between two checks.
const MAX_STEP: f32 = 0.1;

/// The player's body for collision: a circle on the floor, with a height.
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
/// are real positions in metres relative to the plan's base: where things
/// are drawn, skew included. The player never has to be converted to some
/// other space, so being near a skewed wall changes nothing about how they
/// move.
pub fn slide(plan: &impl Plan, from: Vec2, delta: Vec2, body: Body) -> Vec2 {
    let steps = (delta.length() / MAX_STEP).ceil().max(1.0) as u32;
    let step = delta / steps as f32;
    let mut pos = from;
    for _ in 0..steps {
        pos += step;
        // Being pushed out of one wall can push the circle into another (in
        // a corner), so resolve a few times.
        for _ in 0..4 {
            match deepest_overlap(plan, pos, body) {
                Some(push) => pos += push,
                None => break,
            }
        }
    }
    pos
}

/// Where a point of the plan (in cells) really is, in metres.
fn real(plan: &impl Plan, x: i32, y: i32) -> Vec2 {
    let at = Vec2::new(x as f32, y as f32);
    at * CELL + plan.warp(at)
}

/// The move that takes the circle out of whichever obstacle it is deepest
/// inside, or `None` if it touches nothing.
///
/// A wall is a row of half-metre pieces. Resolving the *deepest* overlap
/// first matters: sliding along a wall, the circle overlaps the piece beside
/// it squarely and the next piece's end only slightly. Dealing with the
/// square one first pushes it straight out and clears both; starting with
/// the other would shove it sideways along the wall.
fn deepest_overlap(plan: &impl Plan, pos: Vec2, body: Body) -> Option<Vec2> {
    let solid_ns = |x: i32, y: i32| blocks(plan.slab_ns(x, y), body.height, body.ceiling);
    let solid_ew = |x: i32, y: i32| blocks(plan.slab_ew(x, y), body.height, body.ceiling);
    let cell = (pos / CELL).floor();
    let (cx, cy) = (cell.x as i32, cell.y as i32);
    let mut best: Option<(f32, Vec2)> = None;
    // Offers one obstacle: a line from `a` to `b`, `half` thick each side.
    let mut offer = |a: Vec2, b: Vec2, half: f32| {
        let along = b - a;
        let t = ((pos - a).dot(along) / along.length_squared()).clamp(0.0, 1.0);
        let away = pos - (a + along * t);
        let distance = away.length();
        let depth = body.radius + half - distance;
        if depth > 0.0 && best.is_none_or(|(deepest, _)| depth > deepest) {
            // Dead centre on the line there is no "away"; leave sideways.
            let direction = if distance > 1e-6 { away / distance } else { along.perp().normalize() };
            best = Some((depth, direction * depth));
        }
    };
    // Skew moves things by less than a cell, so two cells either way covers
    // everything a circle smaller than a cell could touch.
    for y in cy - 2..=cy + 2 {
        for x in cx - 2..=cx + 2 {
            if solid_ns(x, y) {
                offer(real(plan, x, y), real(plan, x, y + 1), WALL_HALF);
            }
            if solid_ew(x, y) {
                offer(real(plan, x, y), real(plan, x + 1, y), WALL_HALF);
            }
            if plan.solid(x, y) {
                let corners = [real(plan, x, y), real(plan, x + 1, y), real(plan, x + 1, y + 1), real(plan, x, y + 1)];
                for i in 0..4 {
                    offer(corners[i], corners[(i + 1) % 4], 0.0);
                }
            }
        }
    }
    best.map(|(_, push)| push)
}

/// A doorway of a special kind that the player has just walked through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crossing {
    pub edge: Edge,
    /// The cell whose west or south side the piece is on, relative to the
    /// plan's base.
    pub cell: IVec2,
}

/// Whether moving from `from` to `to` passed through a portal or a phasing
/// wall piece: the two positions are on opposite sides of its line, within
/// its length.
pub fn crossed(plan: &impl Plan, from: Vec2, to: Vec2) -> Option<Crossing> {
    let special = |edge: Edge| matches!(edge, Edge::Portal | Edge::Phase);
    let through = |a: Vec2, b: Vec2| {
        let along = b - a;
        let (before, after) = (along.perp_dot(from - a), along.perp_dot(to - a));
        let t = (to - a).dot(along) / along.length_squared();
        before * after < 0.0 && (0.0..=1.0).contains(&t)
    };
    let cell = (to / CELL).floor();
    let (cx, cy) = (cell.x as i32, cell.y as i32);
    for y in cy - 2..=cy + 2 {
        for x in cx - 2..=cx + 2 {
            let (ns, ew) = (plan.slab_ns(x, y), plan.slab_ew(x, y));
            if special(ns) && through(real(plan, x, y), real(plan, x, y + 1)) {
                return Some(Crossing { edge: ns, cell: IVec2::new(x, y) });
            }
            if special(ew) && through(real(plan, x, y), real(plan, x + 1, y)) {
                return Some(Crossing { edge: ew, cell: IVec2::new(x, y) });
            }
        }
    }
    None
}

#[cfg(test)]
pub mod tests {
    use super::*;

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
        /// Cells whose west side is a portal, or a wall that is not there.
        pub west_portals: Vec<(i32, i32)>,
        pub west_phases: Vec<(i32, i32)>,
        pub stairs: Vec<PlanStair>,
        /// Lattice points that are out of true, and by how many metres.
        pub skews: Vec<((i32, i32), Vec2)>,
    }

    impl Plan for TestPlan {
        fn west(&self, x: i32, y: i32) -> Edge {
            if self.west_portals.contains(&(x, y)) {
                Edge::Portal
            } else if self.west_phases.contains(&(x, y)) {
                Edge::Phase
            } else if self.west_windows.contains(&(x, y)) {
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
        fn stairs(&self) -> &[PlanStair] {
            &self.stairs
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
    fn a_skewed_wall_is_solid_where_it_is_drawn() {
        // A wall on x = 4 from y = 4 to y = 8, with its south end (the
        // lattice point 4,4) pushed 0.25 m east. It now slants.
        let plan = TestPlan {
            west: (4..8).map(|y| (4, y)).collect(),
            skews: vec![((4, 4), Vec2::new(0.25, 0.0))],
            ..Default::default()
        };
        // Walk east into it at several places. Wherever you end up, you
        // are touching the wall as drawn: the slanted line from (2.25, 2)
        // to (2, 4), not the straight one the plan has.
        let (a, b) = (Vec2::new(2.25, 2.0), Vec2::new(2.0, 4.0));
        for north in [2.2, 2.8, 3.4] {
            let end = slide(&plan, Vec2::new(1.0, north), Vec2::new(3.0, 0.0), R);
            let along = b - a;
            let t = ((end - a).dot(along) / along.length_squared()).clamp(0.0, 1.0);
            let gap = end.distance(a + along * t);
            assert!((gap - (WALL_HALF + RADIUS)).abs() < 0.01, "from north {north}: {gap} m from the wall at {end}");
            assert!(end.x < 2.25, "went through at {end}");
        }
    }

    #[test]
    fn standing_still_near_skew_does_not_move_you() {
        let plan = TestPlan {
            west: (4..8).map(|y| (4, y)).collect(),
            skews: vec![((4, 4), Vec2::new(0.25, 0.1)), ((4, 8), Vec2::new(-0.2, 0.15))],
            ..Default::default()
        };
        let mut pos = Vec2::new(1.4, 2.7);
        for _ in 0..600 {
            pos = slide(&plan, pos, Vec2::ZERO, R);
        }
        assert_eq!(pos, Vec2::new(1.4, 2.7));
    }

    #[test]
    fn portals_and_phasing_walls_let_you_through_and_say_so() {
        let wall: Vec<(i32, i32)> = (-10..10).filter(|y| !(0..2).contains(y)).map(|y| (4, y)).collect();
        for (edge, plan) in [
            (Edge::Portal, TestPlan { west: wall.clone(), west_portals: vec![(4, 0), (4, 1)], ..Default::default() }),
            (Edge::Phase, TestPlan { west: wall.clone(), west_phases: vec![(4, 0), (4, 1)], ..Default::default() }),
        ] {
            let (from, mid) = (Vec2::new(1.5, 0.3), Vec2::new(1.9, 0.3));
            // Up to the line: nothing yet.
            assert_eq!(crossed(&plan, from, mid), None);
            let to = slide(&plan, mid, Vec2::new(0.3, 0.0), R);
            assert!(to.x > 2.0, "{edge:?} blocked the way");
            assert_eq!(crossed(&plan, mid, to), Some(Crossing { edge, cell: IVec2::new(4, 0) }));
            // Carrying on does not report it again; coming back does.
            assert_eq!(crossed(&plan, to, to + Vec2::new(0.3, 0.0)), None);
            assert!(crossed(&plan, to, mid).is_some());
            // Passing the ordinary wall beside it reports nothing (and is
            // not possible anyway).
            assert_eq!(crossed(&plan, Vec2::new(1.9, 2.3), Vec2::new(2.1, 2.3)), None);
        }
    }

    #[test]
    fn starting_inside_a_wall_gets_you_out() {
        let plan = TestPlan { west: vec![(0, 0)], ..Default::default() };
        let end = slide(&plan, Vec2::new(0.01, 0.25), Vec2::ZERO, R);
        assert!(end.x >= WALL_HALF + RADIUS - 1e-3, "still inside at {end}");
    }
}

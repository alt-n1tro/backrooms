//! Lighting computed once per chunk instead of once per pixel per frame.
//!
//! A steady fluorescent panel never changes, so the light it throws on the
//! floor and walls can be worked out when the chunk is built and stored in
//! the mesh. The GPU then only multiplies a colour by a number, however many
//! fixtures there are. It also buys what live lights could not afford:
//! walls block light, so a room with dead tubes is properly dark.
//!
//! The model is plain photometry. A panel is treated as two small sources
//! side by side, each sending out half the panel's lumens, brightest straight
//! down. Light arriving at a point is `intensity * cos(angle at the panel) *
//! cos(angle at the surface) / distance^2`, counted only if a straight line
//! from the source to the point crosses no wall. Light that has bounced off
//! other surfaces is approximated as a share of the direct light nearby.
//!
//! Positions in this module are in cells, relative to the plan's base.
//! Light is measured in lux (lumens per square metre).

use std::f32::consts::PI;

use backrooms_core::{Edge, LightState};
use bevy::math::Vec2;

use crate::space::{CELL, Plan, PlanFixture};

/// Luminous output of one lit panel, in lumens.
pub const LUMENS: f32 = 5200.0;
/// Beyond this many cells (7 m) a panel is not counted. Its contribution
/// fades to zero at this distance rather than being cut off.
const RANGE: f32 = 14.0;
/// Bounced light reaching a floor or wall, as a share of the direct light on
/// the floor there.
const BOUNCE: f32 = 0.22;
/// The ceiling gets no direct light at all (the panels face away from it):
/// everything it receives has bounced, mostly off the walls and floor below.
const CEILING_BOUNCE: f32 = 0.38;
/// A trace of light everywhere, so that a blacked-out area is very dark
/// rather than a featureless black screen.
const AMBIENT: f32 = 0.35;
/// Half the distance between the two sources of a panel, in cells.
const HALF_SPREAD: f32 = 0.45;
/// How far a sample point is moved off a wall or corner into the open, in
/// cells. It has to clear the wall's half-thickness.
pub const NUDGE: f32 = 0.15;
/// Number of heights at which a wall is lit, floor to ceiling.
pub const ROWS: usize = 4;

/// How much light gets through each kind of wall piece, as a fraction.
fn passes(edge: Edge) -> f32 {
    match edge {
        Edge::Open => 1.0,
        Edge::Wall => 0.0,
        // The opening is rather less than half the wall's height.
        Edge::Window => 0.4,
        // A low hole lets a little light spill along the floor.
        Edge::Crawl => 0.12,
    }
}

/// The fraction of light that survives the straight line from `a` to `b`:
/// 1.0 across open floor, 0.0 if a wall is in the way, something in between
/// through a window.
///
/// Walls stand on the grid lines, so it is enough to visit each grid line
/// the segment crosses and ask what stands at the crossing.
pub fn transmission(plan: &impl Plan, a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    let mut through = 1.0;

    // Lines of constant x strictly between the endpoints.
    let (low, high) = (a.x.min(b.x), a.x.max(b.x));
    let mut k = low.floor() as i32 + 1;
    while (k as f32) < high {
        let y = a.y + d.y * (k as f32 - a.x) / d.x;
        let j = y.floor() as i32;
        if plan.solid(k, j) || plan.solid(k - 1, j) {
            return 0.0;
        }
        through *= passes(plan.slab_ns(k, j));
        if through == 0.0 {
            return 0.0;
        }
        k += 1;
    }

    // Lines of constant y.
    let (low, high) = (a.y.min(b.y), a.y.max(b.y));
    let mut k = low.floor() as i32 + 1;
    while (k as f32) < high {
        let x = a.x + d.x * (k as f32 - a.y) / d.y;
        let i = x.floor() as i32;
        if plan.solid(i, k) || plan.solid(i, k - 1) {
            return 0.0;
        }
        through *= passes(plan.slab_ew(i, k));
        if through == 0.0 {
            return 0.0;
        }
        k += 1;
    }
    through
}

/// Computes baked light for one plan.
pub struct Baker<'a, P: Plan> {
    plan: &'a P,
    /// Position of every source, in cells.
    sources: Vec<Vec2>,
    /// Ceiling height in metres.
    ceiling: f32,
}

impl<'a, P: Plan> Baker<'a, P> {
    /// Only steadily lit fixtures are baked. Dead ones give no light, and
    /// flickering ones change from moment to moment, so the game lights
    /// those live.
    pub fn new(plan: &'a P, fixtures: &[PlanFixture], ceiling: f32) -> Self {
        let sources = fixtures
            .iter()
            .filter(|f| f.state == LightState::On)
            .flat_map(|f| {
                // The panel covers two cells; its middle is on the line
                // between them.
                let middle = Vec2::new(f.cell.x as f32 + 1.0, f.cell.y as f32 + 0.5);
                [middle - Vec2::X * HALF_SPREAD, middle + Vec2::X * HALF_SPREAD]
            })
            .collect();
        Self { plan, sources, ceiling }
    }

    /// The heights, in metres, at which [`Baker::wall`] reports light.
    pub fn heights(&self) -> [f32; ROWS] {
        std::array::from_fn(|row| self.ceiling * row as f32 / (ROWS - 1) as f32)
    }

    /// Calls `each` for every source that lights the point `at`, with the
    /// horizontal offset from the point to the source in metres and the
    /// source's weight (which fades out towards the edge of its range).
    fn sources_lighting(&self, at: Vec2, mut each: impl FnMut(Vec2, f32)) {
        for &source in &self.sources {
            let offset = source - at;
            let reach = offset.length_squared() / (RANGE * RANGE);
            if reach >= 1.0 {
                continue;
            }
            let through = transmission(self.plan, at, source);
            if through > 0.0 {
                each(offset * CELL, through * (1.0 - reach) * (1.0 - reach));
            }
        }
    }

    /// Direct light on the floor at `at`, in lux.
    pub fn floor_direct(&self, at: Vec2) -> f32 {
        // Each source is half a panel, and a flat diffuse emitter of flux F
        // has intensity F / pi straight ahead.
        let intensity = LUMENS / 2.0 / PI;
        let h = self.ceiling;
        let mut lux = 0.0;
        self.sources_lighting(at, |offset, weight| {
            let d2 = offset.length_squared() + h * h;
            // Both cosines are h / d here: the panel faces straight down and
            // the floor faces straight up.
            lux += weight * intensity * h * h / (d2 * d2);
        });
        lux
    }

    /// Total light on the floor, given the direct light there.
    pub fn floor_total(direct: f32) -> f32 {
        direct * (1.0 + BOUNCE) + AMBIENT
    }

    /// Total light on the ceiling, given the direct light on the floor below.
    pub fn ceiling_total(floor_direct: f32) -> f32 {
        floor_direct * CEILING_BOUNCE + AMBIENT
    }

    /// Total light on a wall at `at` facing `normal`, at each of
    /// [`Baker::heights`], in lux.
    pub fn wall(&self, at: Vec2, normal: Vec2) -> [f32; ROWS] {
        let intensity = LUMENS / 2.0 / PI;
        let heights = self.heights();
        let mut lux = [0.0; ROWS];
        let mut floor = 0.0;
        self.sources_lighting(at, |offset, weight| {
            let h = self.ceiling;
            let d2 = offset.length_squared() + h * h;
            floor += weight * intensity * h * h / (d2 * d2);

            // How squarely the wall faces the source, before dividing by
            // distance. Sources behind the wall's face give nothing.
            let facing = offset.dot(normal);
            if facing <= 0.0 {
                return;
            }
            for (row, &height) in heights.iter().enumerate() {
                let drop = h - height;
                let d2 = offset.length_squared() + drop * drop;
                // cos at the panel = drop / d, cos at the wall = facing / d.
                lux[row] += weight * intensity * drop * facing / (d2 * d2);
            }
        });
        lux.map(|direct| direct + floor * BOUNCE + AMBIENT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::tests::TestPlan;
    use bevy::math::IVec2;

    const H: f32 = 2.6;

    fn lit(plan: &TestPlan, a: Vec2, b: Vec2) -> bool {
        transmission(plan, a, b) > 0.0
    }

    #[test]
    fn windows_and_crawl_holes_let_some_light_through() {
        let wall = |extra: fn(&mut TestPlan)| {
            let mut plan = TestPlan { west: vec![(4, -1), (4, 2)], ..Default::default() };
            extra(&mut plan);
            transmission(&plan, Vec2::new(1.5, 0.5), Vec2::new(7.5, 0.5))
        };
        let open = wall(|_| {});
        let window = wall(|plan| plan.west_windows = vec![(4, 0), (4, 1)]);
        let crawl = wall(|plan| plan.west_crawls = vec![(4, 0), (4, 1)]);
        let solid = wall(|plan| plan.west.extend([(4, 0), (4, 1)]));
        assert_eq!(open, 1.0);
        assert_eq!(solid, 0.0);
        assert!(solid < crawl && crawl < window && window < open, "{crawl} {window}");
    }

    fn fixture(x: i32, y: i32, state: LightState) -> PlanFixture {
        PlanFixture { cell: IVec2::new(x, y), state }
    }

    #[test]
    fn open_floor_is_visible_and_walls_block() {
        let open = TestPlan::default();
        assert!(lit(&open, Vec2::new(0.5, 0.5), Vec2::new(9.5, -7.5)));

        // A wall on the line x = 4, from y = 0 to y = 2.
        let wall = TestPlan { west: vec![(4, 0), (4, 1)], ..Default::default() };
        assert!(!lit(&wall, Vec2::new(1.5, 0.5), Vec2::new(7.5, 1.5)));
        assert!(!lit(&wall, Vec2::new(7.5, 1.5), Vec2::new(1.5, 0.5)));
        // Past its end the view is clear.
        assert!(lit(&wall, Vec2::new(1.5, 2.5), Vec2::new(7.5, 2.5)));
        // Running alongside it is clear too.
        assert!(lit(&wall, Vec2::new(3.5, -3.0), Vec2::new(3.5, 5.0)));

        let wall = TestPlan { south: vec![(0, 3)], ..Default::default() };
        assert!(!lit(&wall, Vec2::new(0.5, 0.5), Vec2::new(0.5, 5.5)));
        assert!(lit(&wall, Vec2::new(1.5, 0.5), Vec2::new(1.5, 5.5)));
    }

    #[test]
    fn pillars_block() {
        let plan = TestPlan {
            pillars: vec![(4, 4)],
            west: vec![(4, 4), (5, 4)],
            south: vec![(4, 4), (4, 5)],
            ..Default::default()
        };
        assert!(!lit(&plan, Vec2::new(1.5, 4.5), Vec2::new(8.5, 4.5)));
        assert!(!lit(&plan, Vec2::new(4.5, 1.5), Vec2::new(4.5, 8.5)));
        assert!(lit(&plan, Vec2::new(1.5, 5.5), Vec2::new(8.5, 5.5)));
    }

    #[test]
    fn light_is_brightest_under_the_panel_and_fades() {
        let plan = TestPlan::default();
        let fixtures = [fixture(0, 0, LightState::On)];
        let baker = Baker::new(&plan, &fixtures, H);
        let under = baker.floor_direct(Vec2::new(1.0, 0.5));
        // Straight below a 5200 lm panel 2.6 m up: about 240 lux.
        assert!((200.0..280.0).contains(&under), "{under} lux under the panel");
        let near = baker.floor_direct(Vec2::new(5.0, 0.5));
        let far = baker.floor_direct(Vec2::new(11.0, 0.5));
        assert!(under > near && near > far && far > 0.0, "{under} > {near} > {far}");
        assert_eq!(baker.floor_direct(Vec2::new(16.0, 0.5)), 0.0);
    }

    #[test]
    fn dead_and_flickering_fixtures_are_not_baked() {
        let plan = TestPlan::default();
        let fixtures = [fixture(0, 0, LightState::Dead), fixture(4, 4, LightState::Flicker)];
        let baker = Baker::new(&plan, &fixtures, H);
        assert_eq!(baker.floor_direct(Vec2::new(1.0, 0.5)), 0.0);
    }

    #[test]
    fn a_wall_casts_a_shadow() {
        // Panel at cells 0..2, a long wall on x = 6 between it and the east.
        let plan = TestPlan { west: (-20..20).map(|y| (6, y)).collect(), ..Default::default() };
        let fixtures = [fixture(0, 0, LightState::On)];
        let baker = Baker::new(&plan, &fixtures, H);
        assert!(baker.floor_direct(Vec2::new(5.5, 0.5)) > 20.0);
        assert_eq!(baker.floor_direct(Vec2::new(6.5, 0.5)), 0.0);
    }

    #[test]
    fn walls_are_lit_from_the_front_only() {
        let plan = TestPlan::default();
        let fixtures = [fixture(0, 0, LightState::On)];
        let baker = Baker::new(&plan, &fixtures, H);
        // A point 2 m east of the panel, on a wall facing west (towards it).
        let facing = baker.wall(Vec2::new(5.0, 0.5), Vec2::NEG_X);
        let away = baker.wall(Vec2::new(5.0, 0.5), Vec2::X);
        for row in 0..ROWS - 1 {
            assert!(facing[row] > away[row] * 1.5, "row {row}: {} vs {}", facing[row], away[row]);
        }
        // At ceiling height the panel is edge-on and gives nothing directly,
        // so both sides are left with the same bounced light.
        assert!((facing[ROWS - 1] - away[ROWS - 1]).abs() < 1e-3);
        // The side facing away still gets bounce, never blackness.
        assert!(away.iter().all(|&lux| lux > AMBIENT));
    }

    #[test]
    fn heights_run_from_floor_to_ceiling() {
        let plan = TestPlan::default();
        let heights = Baker::new(&plan, &[], H).heights();
        assert_eq!(heights[0], 0.0);
        assert_eq!(heights[ROWS - 1], H);
    }
}

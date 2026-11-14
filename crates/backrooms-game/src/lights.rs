//! Fluorescent ceiling fixtures.
//!
//! Where the fixtures are, which are dead and which flicker comes from the
//! core. What a flicker looks like from moment to moment is decided here, as
//! a pure function of the fixture's seed and the clock.
//!
//! The light of a steady panel is baked into the chunk mesh (see
//! [`crate::bake`]), so a steady fixture is only a glowing rectangle here.
//! Flickering fixtures cannot be baked, because they change; each of those
//! gets a real light that the GPU evaluates every frame. They are a few
//! percent of all fixtures, which is what makes this affordable.

use backrooms_core::LightState;
use backrooms_core::hash::Key;
use bevy::prelude::*;

use crate::bake::LUMENS;

/// How the light of a flickering fixture is computed.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum LightKind {
    /// A glowing rectangle the size of the panel. Soft, correct, costlier.
    Rect,
    /// A single point under the panel. Cheaper, for weak GPUs.
    Point,
}

/// A point light throws its output in every direction, a panel only
/// downwards, so the point version needs more to light the floor equally.
const POINT_BOOST: f32 = 7.0;
/// Beyond this distance a flickering fixture lights nothing. Shorter is
/// faster.
const RANGE: f32 = 6.0;
/// The panel is 1.0 m x 0.5 m; the glowing part is a little smaller.
pub const PANEL: Vec2 = Vec2::new(0.92, 0.42);
/// Slightly cool white, like a cheap tube.
const TUBE: Color = Color::srgb(1.0, 0.97, 0.86);

/// A ceiling fixture. Its child entity (if any) is the light it casts.
#[derive(Component)]
pub struct FixtureLight {
    pub state: LightState,
    pub seed: u32,
    /// Current output, 0.0 to 1.0. Read by the hum.
    pub level: f32,
}

/// The three looks a panel can have, shared by all fixtures.
#[derive(Resource)]
pub struct PanelMaterials {
    pub mesh: Handle<Mesh>,
    pub lit: Handle<StandardMaterial>,
    pub dim: Handle<StandardMaterial>,
    pub dark: Handle<StandardMaterial>,
}

pub struct LightsPlugin;

impl Plugin for LightsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(Update, flicker);
    }
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let glow = |strength: f32| StandardMaterial {
        base_color: Color::srgb(0.85, 0.85, 0.8),
        emissive: TUBE.to_linear() * strength,
        ..default()
    };
    commands.insert_resource(PanelMaterials {
        mesh: meshes.add(Rectangle::new(PANEL.x, PANEL.y)),
        lit: materials.add(glow(900.0)),
        dim: materials.add(glow(120.0)),
        dark: materials.add(StandardMaterial { base_color: Color::srgb(0.33, 0.33, 0.3), ..default() }),
    });
}

/// Spawns one fixture as a child of `chunk`, at `local` (metres from the
/// chunk's corner, at ceiling height).
pub fn spawn_fixture(
    commands: &mut Commands,
    chunk: Entity,
    local: Vec3,
    state: LightState,
    seed: u32,
    kind: LightKind,
    panels: &PanelMaterials,
) {
    let lit = state != LightState::Dead;
    let fixture = commands
        .spawn((
            FixtureLight { state, seed, level: if lit { 1.0 } else { 0.0 } },
            Mesh3d(panels.mesh.clone()),
            MeshMaterial3d(if lit { panels.lit.clone() } else { panels.dark.clone() }),
            // A `Rectangle` mesh faces +Z; a quarter turn about X points it
            // down. It hangs a centimetre below the ceiling so the two
            // surfaces do not fight over the same pixels.
            Transform::from_translation(local - Vec3::Y * 0.01)
                .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            ChildOf(chunk),
        ))
        .id();
    // Only a flickering tube needs a live light; see the module docs.
    if state != LightState::Flicker {
        return;
    }
    // The light is a child of the panel, a few centimetres in front of it
    // (the panel's "front" is down). In the panel's own frame that is +Z.
    let below = Transform::from_xyz(0.0, 0.0, 0.03);
    match kind {
        LightKind::Rect => {
            // A RectLight shines along its own -Z, so it is turned to face
            // the same way as the panel.
            let facing = below.with_rotation(Quat::from_rotation_y(std::f32::consts::PI));
            let light = RectLight { color: TUBE, intensity: LUMENS, width: PANEL.x, height: PANEL.y, range: RANGE };
            commands.spawn((light, facing, ChildOf(fixture)));
        }
        LightKind::Point => {
            let light =
                PointLight { color: TUBE, intensity: LUMENS * POINT_BOOST, range: RANGE, radius: 0.2, ..default() };
            commands.spawn((light, Transform::from_xyz(0.0, 0.0, 0.25), ChildOf(fixture)));
        }
    }
}

/// Output of a flickering tube at `seconds`, from 0.0 (out) to 1.0.
///
/// A failing tube mostly burns steadily but slightly dim, and now and then
/// has a fit of stuttering. Time is cut into slots; whether a slot is a fit,
/// and each on/off within it, is a hash of the fixture's seed and the slot
/// number. Nothing is remembered between frames, so the pattern is the same
/// whenever you come back to it.
pub fn flicker_level(seed: u32, seconds: f32) -> f32 {
    let key = Key::new(seed as u64);
    // Each fixture runs on its own clock, so neighbours never flicker in step.
    let t = seconds + key.with(0).roll().below(10_000) as f32 / 100.0;
    let slot = (t / 1.7) as u64;
    let fit = key.with(1).with(slot).roll().below(100) < 38;
    if !fit {
        // Steady, with a faint mains shimmer.
        return 0.86 + 0.04 * (t * 37.0).sin();
    }
    let tick = (t * 24.0) as u64;
    match key.with(2).with(tick).roll().below(100) {
        0..45 => 0.0,
        45..70 => 0.25,
        _ => 1.0,
    }
}

fn flicker(
    time: Res<Time>,
    panels: Res<PanelMaterials>,
    mut fixtures: Query<(&mut FixtureLight, &mut MeshMaterial3d<StandardMaterial>, &Children)>,
    mut rects: Query<&mut RectLight>,
    mut points: Query<&mut PointLight>,
) {
    for (mut fixture, mut material, children) in &mut fixtures {
        if fixture.state != LightState::Flicker {
            continue;
        }
        let level = flicker_level(fixture.seed, time.elapsed_secs());
        fixture.level = level;
        let look = if level > 0.6 {
            &panels.lit
        } else if level > 0.1 {
            &panels.dim
        } else {
            &panels.dark
        };
        // Only write when the look changes: assigning marks the component as
        // changed, which makes the renderer redo work for this entity.
        if material.0 != *look {
            material.0 = look.clone();
        }
        for child in children.iter() {
            if let Ok(mut light) = rects.get_mut(child) {
                light.intensity = LUMENS * level;
            }
            if let Ok(mut light) = points.get_mut(child) {
                light.intensity = LUMENS * POINT_BOOST * level;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flicker_is_a_pure_function_of_seed_and_time() {
        assert_eq!(flicker_level(77, 12.34), flicker_level(77, 12.34));
    }

    #[test]
    fn flicker_stays_in_range_and_actually_flickers() {
        let (mut dark, mut bright) = (0, 0);
        for i in 0..6000 {
            let level = flicker_level(4242, i as f32 / 60.0);
            assert!((0.0..=1.0).contains(&level));
            dark += (level < 0.1) as u32;
            bright += (level > 0.6) as u32;
        }
        // Mostly on, sometimes out.
        assert!(bright > 3000, "bright for {bright} of 6000 frames");
        assert!(dark > 200, "dark for {dark} of 6000 frames");
    }

    #[test]
    fn fixtures_do_not_flicker_in_step() {
        let same = (0..600).filter(|&i| flicker_level(1, i as f32 / 60.0) == flicker_level(2, i as f32 / 60.0)).count();
        assert!(same < 300, "{same} of 600 frames identical");
    }
}

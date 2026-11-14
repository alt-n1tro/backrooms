//! The player: a camera at eye height that walks on the floor plan.

use std::f32::consts::FRAC_PI_2;

use bevy::anti_alias::fxaa::Fxaa;
use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

use crate::collide::slide;
use crate::space::{ChunkStore, RenderOrigin, plan_to_world, world_to_plan};
use crate::world::ViewDistance;

const EYE_HEIGHT: f32 = 1.62;
const RADIUS: f32 = 0.22;
const WALK_SPEED: f32 = 1.5;
const RUN_SPEED: f32 = 4.2;
/// How quickly the player reaches the wanted speed; higher is snappier.
const RESPONSE: f32 = 9.0;
/// Radians turned per pixel of mouse movement.
const SENSITIVITY: f32 = 0.0022;

/// The haze that distance fades into: the yellow of far-off lit wallpaper.
const HAZE: Color = Color::srgb(0.20, 0.16, 0.05);

#[derive(Component, Default)]
pub struct Player {
    /// Current speed over the floor, in plan metres per second.
    velocity: Vec2,
}

/// Where the player starts and which way they face.
#[derive(Resource, Clone, Copy)]
pub struct Spawn {
    /// Plan position relative to the render origin.
    pub plan: Vec2,
    /// Degrees anticlockwise from north.
    pub yaw: f32,
    /// Degrees above the horizon.
    pub pitch: f32,
}

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(Update, (grab_cursor, look, walk).chain());
    }
}

fn setup(mut commands: Commands, spawn: Res<Spawn>, view: Res<ViewDistance>) {
    let reach = view.metres();
    commands.spawn((
        Player::default(),
        Camera3d::default(),
        Projection::from(PerspectiveProjection {
            fov: 74.0_f32.to_radians(),
            near: 0.04,
            far: reach + 20.0,
            ..default()
        }),
        // In Bevy's YXZ order the first angle is yaw about the up axis and
        // the second is pitch. With no rotation the camera looks along -Z,
        // which is north.
        Transform::from_translation(plan_to_world(spawn.plan, EYE_HEIGHT)).with_rotation(Quat::from_euler(
            EulerRot::YXZ,
            spawn.yaw.to_radians(),
            spawn.pitch.to_radians(),
            0.0,
        )),
        // Exposure for a dim interior. Without it, physically sized lights
        // would look like a cave: the default suits a much brighter room.
        Exposure { ev100: 6.4 },
        Tonemapping::TonyMcMapface,
        Bloom { intensity: 0.12, ..Bloom::NATURAL },
        // The fog starts well short of the last loaded chunk and is complete
        // before it, so chunks appear and disappear inside the haze.
        DistanceFog { color: HAZE, falloff: FogFalloff::Linear { start: reach * 0.35, end: reach - 4.0 }, ..default() },
        // Fast edge smoothing. Multisampling is switched off because it is
        // expensive on integrated graphics.
        Msaa::Off,
        Fxaa::default(),
    ));
}

/// Click to capture the mouse, Escape to release it.
fn grab_cursor(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut cursor: Single<&mut CursorOptions>,
) {
    if mouse.just_pressed(MouseButton::Left) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

fn look(
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions>,
    mut camera: Single<&mut Transform, With<Player>>,
) {
    if cursor.grab_mode == CursorGrabMode::None || motion.delta == Vec2::ZERO {
        return;
    }
    let (yaw, pitch, _) = camera.rotation.to_euler(EulerRot::YXZ);
    let yaw = yaw - motion.delta.x * SENSITIVITY;
    // Stop just short of straight up and down, where "left" and "right"
    // stop meaning anything.
    let pitch = (pitch - motion.delta.y * SENSITIVITY).clamp(-FRAC_PI_2 + 0.02, FRAC_PI_2 - 0.02);
    camera.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
}

fn walk(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    origin: Res<RenderOrigin>,
    mut store: ResMut<ChunkStore>,
    player: Single<(&mut Transform, &mut Player)>,
) {
    let (mut transform, mut player) = player.into_inner();
    // A long frame (a stall, a dragged window) must not turn into a leap.
    let dt = time.delta_secs().min(0.1);

    // Which way "forward" is on the floor, ignoring where the eyes point
    // vertically.
    let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
    let forward = Vec2::new(-yaw.sin(), yaw.cos());
    let right = Vec2::new(yaw.cos(), yaw.sin());

    let axis = |plus: &[KeyCode], minus: &[KeyCode]| {
        keys.any_pressed(plus.iter().copied()) as i32 as f32 - keys.any_pressed(minus.iter().copied()) as i32 as f32
    };
    let wish = forward * axis(&[KeyCode::KeyW, KeyCode::ArrowUp], &[KeyCode::KeyS, KeyCode::ArrowDown])
        + right * axis(&[KeyCode::KeyD, KeyCode::ArrowRight], &[KeyCode::KeyA, KeyCode::ArrowLeft]);
    let speed = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) { RUN_SPEED } else { WALK_SPEED };
    let target = wish.normalize_or_zero() * speed;

    // Ease towards the wanted velocity. The exponential makes the easing
    // behave the same at any frame rate.
    let blend = 1.0 - (-RESPONSE * dt).exp();
    player.velocity = player.velocity.lerp(target, blend);

    let from = world_to_plan(transform.translation);
    store.ensure_around(origin.cell_at(from).chunk());
    let to = slide(&store.plan(origin.cell), from, player.velocity * dt, RADIUS);
    // What actually happened is the velocity: walking into a wall stops you.
    if dt > 0.0 {
        player.velocity = (to - from) / dt;
    }
    transform.translation = plan_to_world(to, EYE_HEIGHT);
}

//! The player: a camera at eye height that walks, sprints, jumps, crouches
//! and crawls on the floor plan.

use std::f32::consts::FRAC_PI_2;

use bevy::anti_alias::contrast_adaptive_sharpening::ContrastAdaptiveSharpening;
use bevy::anti_alias::fxaa::Fxaa;
use bevy::camera::{Exposure, RenderTarget};
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

use crate::collide::{Body, slide};
use crate::screen::Screen;
use crate::space::{CELL, ChunkCache, LocalPlan, Plan, RenderOrigin, plan_to_world, world_to_plan};
use crate::world::{Anchored, Ceiling, ViewDistance};

const RADIUS: f32 = 0.22;
const WALK_SPEED: f32 = 1.7;
const SPRINT_SPEED: f32 = 5.2;
/// How quickly the player reaches the wanted speed; higher is snappier.
const RESPONSE: f32 = 11.0;
/// Radians turned per pixel of mouse movement at sensitivity 1.
const RADIANS_PER_PIXEL: f32 = 0.0030;
/// Two presses of W within this many seconds start a sprint.
const DOUBLE_TAP: f32 = 0.3;
/// Upward speed at take-off, in metres per second. With normal gravity this
/// lifts the feet about 0.45 m.
const JUMP_SPEED: f32 = 3.0;
const GRAVITY: f32 = 9.81;
const FOV: f32 = 74.0;
/// Extra field of view at a full sprint, in degrees: a cheap, effective cue
/// for speed.
const SPRINT_FOV: f32 = 6.0;

/// The haze that distance fades into: the yellow of far-off lit wallpaper.
const HAZE: Color = Color::srgb(0.20, 0.16, 0.05);

/// How the player is holding themselves.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Posture {
    /// On hands and knees: slow, but fits through a crawl hole.
    Crawling,
    Crouching,
    #[default]
    Standing,
}

impl Posture {
    /// Height of the body, floor to top of head, in metres.
    pub const fn body_height(self) -> f32 {
        match self {
            Self::Standing => 1.75,
            Self::Crouching => 1.1,
            Self::Crawling => 0.6,
        }
    }

    /// Height of the eyes above the floor, in metres.
    pub const fn eye_height(self) -> f32 {
        match self {
            Self::Standing => 1.62,
            Self::Crouching => 0.98,
            Self::Crawling => 0.42,
        }
    }

    /// Unhurried speed in this posture, in metres per second.
    pub const fn speed(self) -> f32 {
        match self {
            Self::Standing => WALK_SPEED,
            Self::Crouching => 1.0,
            Self::Crawling => 0.6,
        }
    }
}

/// The posture the keys ask for this frame, given the one asked for before.
///
/// C toggles crouching, X toggles crawling, and Space gets you back on your
/// feet. Kept as a plain function so the rules can be tested.
fn wanted_posture(wanted: Posture, crouch: bool, crawl: bool, rise: bool) -> Posture {
    if rise {
        Posture::Standing
    } else if crawl {
        if wanted == Posture::Crawling { Posture::Standing } else { Posture::Crawling }
    } else if crouch {
        if wanted == Posture::Crouching { Posture::Standing } else { Posture::Crouching }
    } else {
        wanted
    }
}

#[derive(Component, Default)]
pub struct Player {
    /// Current speed over the floor, in plan metres per second.
    velocity: Vec2,
    /// Height of the feet above the floor, in metres.
    height: f32,
    /// Upward speed, in metres per second.
    rising: f32,
    /// True from a double-tap of W until W is released.
    sprinting: bool,
    /// When W was last pressed, in seconds since the start.
    last_tap: f32,
    /// How far into a sprint the camera is, 0 to 1, for the field of view.
    rush: f32,
    /// How the body is held now, and what the player has asked for. They
    /// differ while there is no room to get up.
    posture: Posture,
    wanted: Posture,
    /// Where the eyes are now, easing towards the posture's eye height.
    eye: f32,
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

/// Multiplier on mouse-look speed.
#[derive(Resource, Clone, Copy)]
pub struct MouseSensitivity(pub f32);

/// The floor plan around the player, for collision. Rebuilt only when the
/// player crosses into another chunk.
#[derive(Resource, Default)]
pub struct Surroundings(Option<LocalPlan>);

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Surroundings>()
            .add_systems(Startup, setup)
            .add_systems(Update, (grab_cursor, look, walk).chain());
    }
}

fn setup(mut commands: Commands, spawn: Res<Spawn>, view: Res<ViewDistance>, screen: Res<Screen>) {
    let reach = view.metres();
    commands.spawn((
        // No press of W has happened yet, so none can be half of a double-tap.
        Player { last_tap: f32::NEG_INFINITY, eye: Posture::Standing.eye_height(), ..default() },
        Anchored,
        Camera3d::default(),
        // The view is drawn into an off-screen picture, which `screen` then
        // stretches over the window. That lets a huge window be filled
        // without paying for every one of its pixels.
        RenderTarget::Image(screen.image.clone().into()),
        Projection::from(PerspectiveProjection { fov: FOV.to_radians(), near: 0.04, far: reach + 20.0, ..default() }),
        // In Bevy's YXZ order the first angle is yaw about the up axis and
        // the second is pitch. With no rotation the camera looks along -Z,
        // which is north.
        Transform::from_translation(plan_to_world(spawn.plan, Posture::Standing.eye_height()))
            .with_rotation(Quat::from_euler(EulerRot::YXZ, spawn.yaw.to_radians(), spawn.pitch.to_radians(), 0.0)),
        // Exposure for a dim interior. Without it, physically sized lights
        // would look like a cave: the default suits a much brighter room.
        Exposure { ev100: 6.4 },
        Tonemapping::TonyMcMapface,
        Bloom { intensity: 0.12, ..Bloom::NATURAL },
        // The fog starts well short of the last loaded chunk and is complete
        // before it, so chunks appear and disappear inside the haze.
        DistanceFog { color: HAZE, falloff: FogFalloff::Linear { start: reach * 0.35, end: reach - 4.0 }, ..default() },
        // Draw depth only first, then colour. With the depth known up
        // front, the costly surface shader runs once per pixel, for the
        // nearest surface, instead of also for every wall hidden behind it.
        DepthPrepass,
        // Fast edge smoothing. Multisampling is switched off because it is
        // expensive on integrated graphics.
        Msaa::Off,
        Fxaa::default(),
        // Restores some of the crispness lost when the picture is stretched.
        ContrastAdaptiveSharpening { enabled: true, sharpening_strength: 0.35, denoise: false },
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
    sensitivity: Res<MouseSensitivity>,
    cursor: Single<&CursorOptions>,
    mut camera: Single<&mut Transform, With<Player>>,
) {
    if cursor.grab_mode == CursorGrabMode::None || motion.delta == Vec2::ZERO {
        return;
    }
    // The raw movement is applied in full, the frame it arrives: no
    // smoothing and no acceleration, which are what make a mouse feel soft.
    let turn = motion.delta * RADIANS_PER_PIXEL * sensitivity.0;
    let (yaw, pitch, _) = camera.rotation.to_euler(EulerRot::YXZ);
    // Stop just short of straight up and down, where "left" and "right"
    // stop meaning anything.
    let pitch = (pitch - turn.y).clamp(-FRAC_PI_2 + 0.02, FRAC_PI_2 - 0.02);
    camera.rotation = Quat::from_euler(EulerRot::YXZ, yaw - turn.x, pitch, 0.0);
}

/// Whether this press of W completes a double-tap. Kept as a plain function
/// so the timing rule can be tested.
fn is_double_tap(now: f32, last_tap: f32) -> bool {
    now - last_tap <= DOUBLE_TAP
}

/// One step of a jump: returns the new height and upward speed.
fn fall(height: f32, rising: f32, dt: f32) -> (f32, f32) {
    let rising = rising - GRAVITY * dt;
    let height = height + rising * dt;
    if height <= 0.0 { (0.0, 0.0) } else { (height, rising) }
}

pub fn walk(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    origin: Res<RenderOrigin>,
    ceiling: Res<Ceiling>,
    cache: Res<ChunkCache>,
    mut surroundings: ResMut<Surroundings>,
    player: Single<(&mut Transform, &mut Player, &mut Projection)>,
) {
    let (mut transform, mut player, mut projection) = player.into_inner();
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

    // Sprint: tap W twice quickly and keep it held, or hold Shift.
    if keys.just_pressed(KeyCode::KeyW) {
        let now = time.elapsed_secs();
        if is_double_tap(now, player.last_tap) {
            player.sprinting = true;
        }
        player.last_tap = now;
    }
    if !keys.pressed(KeyCode::KeyW) {
        player.sprinting = false;
    }
    // Only someone on their feet can sprint.
    let sprint = player.posture == Posture::Standing
        && (player.sprinting || keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]));
    let target = wish.normalize_or_zero() * if sprint { SPRINT_SPEED } else { player.posture.speed() };

    // Ease towards the wanted velocity. The exponential makes the easing
    // behave the same at any frame rate.
    let blend = 1.0 - (-RESPONSE * dt).exp();
    player.velocity = player.velocity.lerp(target, blend);

    // Space jumps when standing; when down low it means "get up" instead.
    let space = keys.just_pressed(KeyCode::Space);
    let low = player.posture != Posture::Standing;
    player.wanted =
        wanted_posture(player.wanted, keys.just_pressed(KeyCode::KeyC), keys.just_pressed(KeyCode::KeyX), space && low);
    if space && !low && player.height == 0.0 {
        player.rising = JUMP_SPEED;
    }
    if player.height > 0.0 || player.rising > 0.0 {
        (player.height, player.rising) = fall(player.height, player.rising, dt);
    }

    // Collide against the plan around the player. It is a snapshot of three
    // by three chunks, refreshed when the player leaves the middle one.
    let from = world_to_plan(transform.translation);
    let chunk = origin.cell_at(from).chunk();
    if surroundings.0.as_ref().is_none_or(|plan| plan.base != chunk.origin()) {
        surroundings.0 = Some(LocalPlan::around(&cache, chunk));
    }
    let plan = surroundings.0.as_ref().expect("set just above");
    // The snapshot measures from its own corner, not from the render origin.
    let corner = origin.plan_of(plan.base);
    // Where the lattice is out of true, what is drawn has been pushed off
    // the plan. Collision is done on the undistorted plan, so the player's
    // real position is pulled back onto it first and pushed out again after.
    // Almost everywhere both steps change nothing.
    let on_plan = plan.unwarp((from - corner) / CELL) * CELL;
    let body = |posture: Posture| Body { radius: RADIUS, height: posture.body_height(), ceiling: ceiling.0 };

    // Change posture. Getting lower always works; getting up needs room,
    // and until there is room the request simply waits.
    if player.wanted != player.posture && player.height == 0.0 {
        let fits = |posture: Posture| slide(plan, on_plan, Vec2::ZERO, body(posture)).distance(on_plan) < 1e-3;
        if player.wanted < player.posture || fits(player.wanted) {
            player.posture = player.wanted;
        } else if player.posture == Posture::Crawling && fits(Posture::Crouching) {
            // No room to stand, but enough to get off the floor.
            player.posture = Posture::Crouching;
        }
    }

    let moved = slide(plan, on_plan, player.velocity * dt, body(player.posture));
    let to = moved + plan.warp(moved / CELL) + corner;
    // What actually happened is the velocity: walking into a wall stops you.
    if dt > 0.0 {
        player.velocity = (to - from) / dt;
    }
    // The eyes glide to their new height rather than snapping.
    let eye = player.posture.eye_height();
    player.eye += (eye - player.eye) * (1.0 - (-10.0 * dt).exp());
    transform.translation = plan_to_world(to, player.eye + player.height);

    // Widen the view a little while actually moving fast.
    let fast = sprint && player.velocity.length() > WALK_SPEED * 1.5;
    player.rush += ((fast as i32 as f32) - player.rush) * (1.0 - (-6.0 * dt).exp());
    if let Projection::Perspective(perspective) = &mut *projection {
        let fov = (FOV + SPRINT_FOV * player.rush).to_radians();
        // Assigning marks the component as changed even if the value is the
        // same, so only write when it really moves.
        if (perspective.fov - fov).abs() > 1e-4 {
            perspective.fov = fov;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_quick_taps_sprint_and_two_slow_ones_do_not() {
        assert!(is_double_tap(10.2, 10.0));
        assert!(!is_double_tap(10.5, 10.0));
        // The very first press of the session is not a double-tap.
        assert!(!is_double_tap(5.0, 0.0));
    }

    #[test]
    fn a_jump_goes_up_about_half_a_metre_and_lands() {
        let (mut height, mut rising) = (0.0, JUMP_SPEED);
        let (mut peak, mut steps) = (0.0f32, 0);
        loop {
            (height, rising) = fall(height, rising, 1.0 / 120.0);
            peak = peak.max(height);
            steps += 1;
            if height == 0.0 {
                break;
            }
            assert!(steps < 1000, "never landed");
        }
        assert!((0.4..0.5).contains(&peak), "peak {peak} m");
        // Up and down in a little over half a second.
        let seconds = steps as f32 / 120.0;
        assert!((0.5..0.7).contains(&seconds), "airborne for {seconds} s");
        assert_eq!(rising, 0.0);
        // The head (eyes plus a little) stays under a 2.6 m ceiling.
        assert!(Posture::Standing.body_height() + peak < 2.6);
    }

    #[test]
    fn posture_keys_toggle_and_space_gets_you_up() {
        use Posture::*;
        // C and X each toggle their own posture.
        assert_eq!(wanted_posture(Standing, true, false, false), Crouching);
        assert_eq!(wanted_posture(Crouching, true, false, false), Standing);
        assert_eq!(wanted_posture(Standing, false, true, false), Crawling);
        assert_eq!(wanted_posture(Crawling, false, true, false), Standing);
        // Straight from one low posture to the other.
        assert_eq!(wanted_posture(Crouching, false, true, false), Crawling);
        assert_eq!(wanted_posture(Crawling, true, false, false), Crouching);
        // Space always means "up"; no key means no change.
        assert_eq!(wanted_posture(Crawling, false, false, true), Standing);
        assert_eq!(wanted_posture(Crouching, false, false, false), Crouching);
    }

    #[test]
    fn postures_are_ordered_and_only_crawling_fits_a_crawl_hole() {
        use backrooms_core::chunk::CRAWL_HEIGHT_MM;
        assert!(Posture::Crawling < Posture::Crouching && Posture::Crouching < Posture::Standing);
        let hole = CRAWL_HEIGHT_MM as f32 / 1000.0;
        assert!(Posture::Crawling.body_height() <= hole);
        assert!(Posture::Crouching.body_height() > hole);
        for posture in [Posture::Crawling, Posture::Crouching, Posture::Standing] {
            assert!(posture.eye_height() < posture.body_height());
        }
    }

    #[test]
    fn jump_height_does_not_depend_on_frame_rate() {
        let peak_at = |fps: f32| {
            let (mut height, mut rising, mut peak) = (0.0, JUMP_SPEED, 0.0f32);
            loop {
                (height, rising) = fall(height, rising, 1.0 / fps);
                peak = peak.max(height);
                if height == 0.0 {
                    return peak;
                }
            }
        };
        assert!((peak_at(30.0) - peak_at(144.0)).abs() < 0.06);
    }
}

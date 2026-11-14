//! What reaches the window, and how fast.
//!
//! The 3D view is not drawn straight into the window. It is drawn into an
//! off-screen picture, usually smaller than the window, and that picture is
//! stretched over the window with the HUD drawn on top at full sharpness.
//! The cost of a frame is very nearly proportional to the number of pixels
//! shaded, so filling a 2880 x 1800 screen from a picture two thirds that
//! size in each direction takes well under half the work.
//!
//! This module also owns the frame-rate cap and the fullscreen toggle.

use std::time::{Duration, Instant};

use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureFormat};
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};

/// How large the off-screen picture is compared with the window.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RenderScale {
    /// As large as possible without exceeding a pixel budget, which is
    /// itself adjusted while playing to hold the target frame rate.
    Auto,
    /// A fixed fraction of the window's width and height.
    Fixed(f32),
}

/// Pixels the automatic scale starts with: a little over 1920 x 1080.
const PIXEL_BUDGET: f32 = 2_100_000.0;
/// The budget never drops below this (about 1280 x 720): softer than that
/// is not worth the frames.
const MIN_BUDGET: f32 = 900_000.0;
/// Nor does it rise above a 4K picture.
const MAX_BUDGET: f32 = 8_300_000.0;
/// The frame rate the automatic scale tries to hold.
const TARGET_FPS: f32 = 60.0;

/// The pixel budget to use next, given the current one and the frame rate
/// it produced.
///
/// Below the target, the budget is cut in proportion to the shortfall, since
/// frame time is close to proportional to pixels drawn. Well above the
/// target, it creeps up. In between it is left alone: that dead band is what
/// stops the picture from pulsing between two sizes.
pub fn next_budget(budget: f32, fps: f32) -> f32 {
    let next = if fps < TARGET_FPS * 0.95 {
        // Aim a little past the target so one step is usually enough.
        budget * (fps / (TARGET_FPS * 1.04)).clamp(0.6, 0.97)
    } else if fps > TARGET_FPS * 1.25 {
        budget * 1.08
    } else {
        budget
    };
    next.clamp(MIN_BUDGET, MAX_BUDGET)
}

impl RenderScale {
    /// Size of the off-screen picture for a window of `window` pixels,
    /// given the current pixel `budget` (which only `Auto` uses).
    pub fn target_size(self, window: UVec2, budget: f32) -> UVec2 {
        let scale = match self {
            Self::Fixed(scale) => scale,
            Self::Auto => (budget / (window.x as f32 * window.y as f32).max(1.0)).sqrt().min(1.0),
        };
        // Whole pixels, and never nothing.
        (window.as_vec2() * scale).round().as_uvec2().max(UVec2::splat(16))
    }
}

/// The off-screen picture the 3D camera draws into.
#[derive(Resource)]
pub struct Screen {
    pub image: Handle<Image>,
    pub scale: RenderScale,
    /// Current size of the picture, for the HUD.
    pub size: UVec2,
    /// Pixels the automatic scale currently allows.
    budget: f32,
    /// Whether the budget follows the frame rate.
    adaptive: bool,
}

/// Highest frame rate to run at, or `None` for no limit.
#[derive(Resource, Clone, Copy)]
pub struct FrameCap(pub Option<f64>);

pub struct ScreenPlugin {
    pub scale: RenderScale,
    pub initial_window: UVec2,
    /// Let the automatic scale follow the frame rate. Off for screenshots,
    /// which should not depend on how fast the machine happens to be.
    pub adaptive: bool,
}

impl Plugin for ScreenPlugin {
    fn build(&self, app: &mut App) {
        let size = self.scale.target_size(self.initial_window, PIXEL_BUDGET);
        // `new_target_texture` makes an image the GPU can both draw into and
        // read from. The sRGB format matches what a window would have been.
        let image = Image::new_target_texture(size.x, size.y, TextureFormat::Bgra8UnormSrgb, None);
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        let adaptive = self.adaptive && self.scale == RenderScale::Auto;
        app.insert_resource(Screen { image, scale: self.scale, size, budget: PIXEL_BUDGET, adaptive })
            .add_systems(Startup, setup)
            .add_systems(Update, (adapt_to_frame_rate, fit_to_window, toggle_fullscreen).chain())
            .add_systems(Last, limit_frame_rate);
    }
}

fn setup(mut commands: Commands, screen: Res<Screen>) {
    // A second camera, drawing only the interface, straight to the window.
    commands.spawn((Camera2d, IsDefaultUiCamera, Msaa::Off));
    // The 3D picture, stretched over the whole window, behind everything.
    commands.spawn((
        ImageNode::new(screen.image.clone()),
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        GlobalZIndex(-1),
    ));
}

/// Frame-rate bookkeeping for [`adapt_to_frame_rate`].
#[derive(Default)]
struct Pace {
    frames: u32,
    seconds: f32,
    /// Measurements to throw away: the first seconds of a run and the
    /// moment after a resize are full of one-off stalls.
    skip: u32,
    started: bool,
}

/// Once a second, compares the frame rate with the target and adjusts how
/// many pixels the automatic scale may draw.
fn adapt_to_frame_rate(time: Res<Time<Real>>, mut screen: ResMut<Screen>, mut pace: Local<Pace>) {
    if !screen.adaptive {
        return;
    }
    if !pace.started {
        (pace.started, pace.skip) = (true, 3);
    }
    pace.frames += 1;
    pace.seconds += time.delta_secs();
    if pace.seconds < 1.0 {
        return;
    }
    let fps = pace.frames as f32 / pace.seconds;
    (pace.frames, pace.seconds) = (0, 0.0);
    if pace.skip > 0 {
        pace.skip -= 1;
        return;
    }
    let next = next_budget(screen.budget, fps);
    if next != screen.budget {
        screen.budget = next;
        pace.skip = 1;
    }
}

/// Keeps the off-screen picture in step with the window's size.
fn fit_to_window(
    window: Single<&Window, With<PrimaryWindow>>,
    mut screen: ResMut<Screen>,
    mut images: ResMut<Assets<Image>>,
) {
    let wanted = screen.scale.target_size(window.physical_size(), screen.budget);
    if wanted == screen.size {
        return;
    }
    if let Some(mut image) = images.get_mut(&screen.image) {
        image.resize(Extent3d { width: wanted.x, height: wanted.y, depth_or_array_layers: 1 });
        screen.size = wanted;
    }
}

/// F11 (or F, for laptops whose F-keys need Fn) switches between a window
/// and the whole screen.
fn toggle_fullscreen(keys: Res<ButtonInput<KeyCode>>, mut window: Single<&mut Window, With<PrimaryWindow>>) {
    if keys.any_just_pressed([KeyCode::F11, KeyCode::KeyF]) {
        window.mode = match window.mode {
            WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            _ => WindowMode::Windowed,
        };
    }
}

/// Sleeps out the rest of the frame if it finished early.
///
/// The game presents frames without waiting for the display (which keeps
/// mouse look immediate), so without a cap it would run as fast as the GPU
/// allows and turn a laptop into a heater for no visible gain.
fn limit_frame_rate(cap: Res<FrameCap>, mut frame_started: Local<Option<Instant>>) {
    if let (Some(fps), Some(started)) = (cap.0, *frame_started) {
        let budget = Duration::from_secs_f64(1.0 / fps);
        if let Some(spare) = budget.checked_sub(started.elapsed()) {
            std::thread::sleep(spare);
        }
    }
    *frame_started = Some(Instant::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_windows_are_drawn_at_full_size() {
        assert_eq!(RenderScale::Auto.target_size(UVec2::new(1600, 900), PIXEL_BUDGET), UVec2::new(1600, 900));
        assert_eq!(RenderScale::Auto.target_size(UVec2::new(1920, 1080), PIXEL_BUDGET), UVec2::new(1920, 1080));
    }

    #[test]
    fn large_screens_are_held_to_the_budget_and_keep_their_shape() {
        let window = UVec2::new(2880, 1800);
        let size = RenderScale::Auto.target_size(window, PIXEL_BUDGET);
        assert!(size.x * size.y <= PIXEL_BUDGET as u32 + 4000, "{size}");
        assert!(size.x * size.y > 2_000_000, "{size}");
        let (wide, narrow) = (window.x as f32 / window.y as f32, size.x as f32 / size.y as f32);
        assert!((wide - narrow).abs() < 0.01);
    }

    #[test]
    fn a_slow_frame_rate_shrinks_the_picture_to_reach_the_target() {
        // 44 fps at the starting budget, with frame time proportional to
        // pixels: one adjustment should land at or just above 60.
        let budget = next_budget(PIXEL_BUDGET, 44.0);
        let expected_fps = 44.0 * PIXEL_BUDGET / budget;
        assert!((60.0..66.0).contains(&expected_fps), "would run at {expected_fps} fps");
        // And having got there, it stays put.
        assert_eq!(next_budget(budget, expected_fps), budget);
    }

    #[test]
    fn spare_speed_grows_the_picture_and_the_budget_has_limits() {
        assert!(next_budget(PIXEL_BUDGET, 120.0) > PIXEL_BUDGET);
        assert_eq!(next_budget(PIXEL_BUDGET, 60.0), PIXEL_BUDGET);
        assert_eq!(next_budget(PIXEL_BUDGET, 70.0), PIXEL_BUDGET);
        assert_eq!(next_budget(MIN_BUDGET, 10.0), MIN_BUDGET);
        assert_eq!(next_budget(MAX_BUDGET, 500.0), MAX_BUDGET);
        // A dreadful frame rate still only takes one measured step at a time.
        assert!(next_budget(PIXEL_BUDGET, 5.0) >= PIXEL_BUDGET * 0.6);
    }

    #[test]
    fn a_fixed_scale_is_used_as_given() {
        assert_eq!(RenderScale::Fixed(0.5).target_size(UVec2::new(2880, 1800), PIXEL_BUDGET), UVec2::new(1440, 900));
        assert_eq!(RenderScale::Fixed(1.0).target_size(UVec2::new(2880, 1800), PIXEL_BUDGET), UVec2::new(2880, 1800));
        // A minimised window must not ask for an empty picture.
        assert_eq!(RenderScale::Fixed(0.5).target_size(UVec2::ZERO, PIXEL_BUDGET), UVec2::splat(16));
    }
}

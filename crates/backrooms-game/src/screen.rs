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
    /// As large as possible without exceeding a pixel budget.
    Auto,
    /// A fixed fraction of the window's width and height.
    Fixed(f32),
}

/// Pixels the automatic scale allows: a little over 1920 x 1080.
const PIXEL_BUDGET: f32 = 2_100_000.0;

impl RenderScale {
    /// Size of the off-screen picture for a window of `window` pixels.
    pub fn target_size(self, window: UVec2) -> UVec2 {
        let scale = match self {
            Self::Fixed(scale) => scale,
            Self::Auto => (PIXEL_BUDGET / (window.x as f32 * window.y as f32).max(1.0)).sqrt().min(1.0),
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
}

/// Highest frame rate to run at, or `None` for no limit.
#[derive(Resource, Clone, Copy)]
pub struct FrameCap(pub Option<f64>);

pub struct ScreenPlugin {
    pub scale: RenderScale,
    pub initial_window: UVec2,
}

impl Plugin for ScreenPlugin {
    fn build(&self, app: &mut App) {
        let size = self.scale.target_size(self.initial_window);
        // `new_target_texture` makes an image the GPU can both draw into and
        // read from. The sRGB format matches what a window would have been.
        let image = Image::new_target_texture(size.x, size.y, TextureFormat::Bgra8UnormSrgb, None);
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(image);
        app.insert_resource(Screen { image, scale: self.scale, size })
            .add_systems(Startup, setup)
            .add_systems(Update, (fit_to_window, toggle_fullscreen))
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

/// Keeps the off-screen picture in step with the window's size.
fn fit_to_window(
    window: Single<&Window, With<PrimaryWindow>>,
    mut screen: ResMut<Screen>,
    mut images: ResMut<Assets<Image>>,
) {
    let wanted = screen.scale.target_size(window.physical_size());
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
        assert_eq!(RenderScale::Auto.target_size(UVec2::new(1600, 900)), UVec2::new(1600, 900));
        assert_eq!(RenderScale::Auto.target_size(UVec2::new(1920, 1080)), UVec2::new(1920, 1080));
    }

    #[test]
    fn large_screens_are_held_to_the_budget_and_keep_their_shape() {
        let window = UVec2::new(2880, 1800);
        let size = RenderScale::Auto.target_size(window);
        assert!(size.x * size.y <= PIXEL_BUDGET as u32 + 4000, "{size}");
        assert!(size.x * size.y > 2_000_000, "{size}");
        let (wide, narrow) = (window.x as f32 / window.y as f32, size.x as f32 / size.y as f32);
        assert!((wide - narrow).abs() < 0.01);
    }

    #[test]
    fn a_fixed_scale_is_used_as_given() {
        assert_eq!(RenderScale::Fixed(0.5).target_size(UVec2::new(2880, 1800)), UVec2::new(1440, 900));
        assert_eq!(RenderScale::Fixed(1.0).target_size(UVec2::new(2880, 1800)), UVec2::new(2880, 1800));
        // A minimised window must not ask for an empty picture.
        assert_eq!(RenderScale::Fixed(0.5).target_size(UVec2::ZERO), UVec2::splat(16));
    }
}

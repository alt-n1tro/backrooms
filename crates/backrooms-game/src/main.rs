//! Backrooms: walk the deterministic world.
//!
//! This crate is the renderer and game. Everything about *what the world is*
//! comes from `backrooms-core`; this crate only decides how to show it, and
//! is the one place floating-point numbers are allowed.

use std::path::PathBuf;
use std::process::ExitCode;

use backrooms_core::{Address, CellPos, GeneratorVersion, LevelId, level_info};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{PresentMode, WindowResolution};
use clap::Parser;

mod audio;
mod bake;
mod collide;
mod hud;
mod lights;
mod mesh;
mod player;
mod screen;
mod space;
mod surface;
mod world;

use screen::RenderScale;
use space::{CELL, ChunkCache, RenderOrigin};

/// Explore the Backrooms.
///
/// Click to look around. W A S D to walk; tap W twice (or hold Shift) to
/// sprint; Space to jump. F11 for fullscreen, F3 for frame rate, Escape to
/// free the mouse.
#[derive(Parser)]
#[command(version, about)]
struct Args {
    /// Start at an address such as backrooms://v1/0/120/-45.
    #[arg(long, conflicts_with_all = ["x", "y", "level", "random"])]
    address: Option<Address>,
    /// Level id to start on.
    #[arg(long, default_value_t = 0)]
    level: u8,
    /// Cell x to start at (east is positive).
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    x: i32,
    /// Cell y to start at (north is positive).
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    y: i32,
    /// Start somewhere random.
    #[arg(long)]
    random: bool,
    /// How far to draw, in chunks of 8 m.
    #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(i32).range(2..=12))]
    view: i32,
    /// How flickering fixtures cast light. `point` is slightly cheaper.
    #[arg(long, value_enum, default_value = "rect")]
    lights: lights::LightKind,
    /// Size of the 3D picture relative to the window: `auto` (hold it to
    /// about 1080p worth of pixels) or a number from 0.25 to 1. Lower is
    /// faster and softer; 1 is native.
    #[arg(long, default_value = "auto", value_parser = parse_scale)]
    render_scale: RenderScale,
    /// Highest frame rate to run at; 0 for no limit.
    #[arg(long, default_value_t = 120, value_parser = clap::value_parser!(u32).range(0..=1000))]
    fps: u32,
    /// Wait for the display before showing each frame. Removes any tearing
    /// at the cost of slower-feeling mouse look.
    #[arg(long)]
    vsync: bool,
    /// Mouse-look speed; 1 is the default, 2 is twice as fast.
    #[arg(long, default_value_t = 1.0)]
    sensitivity: f32,
    /// Loudness of the fluorescent hum; 1 is the default, 0 is silence.
    #[arg(long, default_value_t = 1.0)]
    hum: f32,
    /// Move the render origin after straying this many chunks from it. A
    /// testing aid; there is no reason to change it when playing.
    #[arg(long, default_value_t = 8, hide = true, value_parser = clap::value_parser!(i32).range(1..=64))]
    recentre: i32,
    /// Window size, as WIDTHxHEIGHT.
    #[arg(long, default_value = "1600x900", value_parser = parse_size)]
    size: (u32, u32),
    /// Direction to face at the start, in degrees anticlockwise from north.
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    yaw: f32,
    /// Angle to look up (or, if negative, down) at the start, in degrees.
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    pitch: f32,
    /// Save one picture of the starting view to this PNG file, then quit.
    #[arg(long)]
    screenshot: Option<PathBuf>,
    /// Frames to wait before taking the screenshot. A testing aid.
    #[arg(long, default_value_t = 90, hide = true, value_parser = clap::value_parser!(u32).range(50..=100000))]
    after: u32,
}

fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let (w, h) = text.split_once('x').ok_or("expected WIDTHxHEIGHT, for example 1600x900")?;
    let parse = |n: &str| n.parse::<u32>().ok().filter(|n| (160..=8192).contains(n));
    parse(w).zip(parse(h)).ok_or_else(|| "width and height must be numbers from 160 to 8192".into())
}

fn parse_scale(text: &str) -> Result<RenderScale, String> {
    if text == "auto" {
        return Ok(RenderScale::Auto);
    }
    text.parse::<f32>()
        .ok()
        .filter(|scale| (0.25..=1.0).contains(scale))
        .map(RenderScale::Fixed)
        .ok_or_else(|| "expected `auto` or a number from 0.25 to 1".into())
}

/// Works out where to start from the command line.
fn start_address(args: &Args) -> Result<Address, String> {
    if let Some(address) = args.address {
        return Ok(address);
    }
    let level = LevelId::new(args.level).ok_or("level must be a number from 0 to 127")?;
    let cell = if args.random {
        // The only use of real randomness in the game so far: picking a
        // place to start. It comes from the operating system and never
        // touches the world generator.
        let bits = getrandom::u64().map_err(|e| format!("no randomness available from the system: {e}"))?;
        CellPos::new(bits as i32, (bits >> 32) as i32)
    } else {
        CellPos::new(args.x, args.y)
    };
    Ok(Address { version: GeneratorVersion::LATEST, level, cell })
}

/// The nearest cell to `wanted` that is not inside a pillar, searching
/// outwards in rings. Deterministic: the same request always lands on the
/// same cell.
fn nearest_floor(cache: &ChunkCache, wanted: CellPos) -> CellPos {
    for ring in 0..8i32 {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs().max(dy.abs()) != ring {
                    continue;
                }
                let cell = wanted.offset(dx, dy);
                let (x, y) = cell.local();
                if !cache.get(cell.chunk()).cells[y][x].solid {
                    return cell;
                }
            }
        }
    }
    wanted
}

/// Pending screenshot: take it once the scene has had time to appear.
#[derive(Resource)]
struct Capture {
    path: PathBuf,
    /// Frames to wait before the picture: long enough for every chunk to be
    /// built and every shader compiled.
    after: u32,
}

fn capture(
    mut commands: Commands,
    capture: Res<Capture>,
    time: Res<Time<Real>>,
    mut frame: Local<u32>,
    mut measured_from: Local<f64>,
    mut exit: MessageWriter<AppExit>,
) {
    *frame += 1;
    // Frame time, averaged over the settled frames just before the picture.
    // Printed so that two builds can be compared on the same machine.
    if *frame == capture.after - 40 {
        *measured_from = time.elapsed_secs_f64();
    }
    if *frame == capture.after {
        let per_frame = (time.elapsed_secs_f64() - *measured_from) / 40.0;
        println!("Average frame time: {:.1} ms", per_frame * 1000.0);
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(capture.path.clone()));
    }
    // The file is written in the background; leave once it has appeared, or
    // give up after a while rather than hang.
    let saved = *frame > capture.after + 10 && capture.path.exists();
    if saved || *frame > capture.after + 1200 {
        exit.write(if saved { AppExit::Success } else { AppExit::error() });
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    let address = match start_address(&args) {
        Ok(address) => address,
        Err(message) => {
            eprintln!("error: {message}");
            return ExitCode::FAILURE;
        }
    };
    let info = level_info(address.version, address.level);
    if !info.built {
        eprintln!("error: level {} is reserved but not built yet", address.level);
        return ExitCode::FAILURE;
    }

    // Room for everything in view several times over; beyond that the
    // least recently used chunks are forgotten and regenerated on demand.
    let cache = ChunkCache::new(address.version, address.level, 4096);
    let start = nearest_floor(&cache, address.cell);
    // The render origin is the corner of the starting chunk. Chunk corners
    // are multiples of 8 m, so every repeating pattern in the surface shader
    // lines up no matter which chunk that is.
    let origin = RenderOrigin { version: address.version, level: address.level, cell: start.chunk().origin() };
    let spawn = player::Spawn {
        // The middle of the starting cell.
        plan: origin.plan_of(start) + Vec2::splat(CELL / 2.0),
        yaw: args.yaw,
        pitch: args.pitch,
    };
    println!("Starting at {}", Address { cell: start, ..address });

    let window = Window {
        title: "Backrooms".into(),
        resolution: WindowResolution::new(args.size.0, args.size.1),
        // Without vsync a finished frame is shown at once instead of being
        // queued for the display's next refresh, which takes a frame or two
        // of delay out of mouse look. The frame-rate cap keeps that from
        // running the GPU flat out.
        present_mode: if args.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync },
        ..default()
    };
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: Some(window), ..default() }))
        .insert_resource(ClearColor(Color::BLACK))
        // No light without a source: bounced light is part of what the
        // chunk meshes carry (see `bake`).
        .insert_resource(GlobalAmbientLight::NONE)
        .insert_resource(origin)
        .insert_resource(cache)
        .insert_resource(spawn)
        .insert_resource(player::MouseSensitivity(args.sensitivity.clamp(0.05, 20.0)))
        .insert_resource(audio::HumVolume(args.hum.clamp(0.0, 10.0)))
        .insert_resource(screen::FrameCap((args.fps > 0 && !args.vsync).then_some(args.fps as f64)))
        .insert_resource(world::RecentreAfter(args.recentre))
        .insert_resource(args.lights)
        .insert_resource(world::ViewDistance(args.view))
        .insert_resource(world::Ceiling(info.ceiling_mm as f32 / 1000.0))
        .add_plugins((
            screen::ScreenPlugin { scale: args.render_scale, initial_window: UVec2::new(args.size.0, args.size.1) },
            surface::SurfacePlugin { ceiling: info.ceiling_mm as f32 / 1000.0 },
            lights::LightsPlugin,
            world::WorldPlugin,
            player::PlayerPlugin,
            hud::HudPlugin,
            audio::HumPlugin,
        ));
    if let Some(path) = args.screenshot {
        // An old file with this name would look like a finished screenshot.
        let _ = std::fs::remove_file(&path);
        app.insert_resource(Capture { path, after: args.after }).add_systems(Update, capture);
    }

    // `run` returns how the app ended; anything but success becomes a
    // failing exit code for scripts.
    match app.run() {
        AppExit::Success => ExitCode::SUCCESS,
        AppExit::Error(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_parse() {
        assert_eq!(parse_size("1600x900"), Ok((1600, 900)));
        assert!(parse_size("1600").is_err());
        assert!(parse_size("0x900").is_err());
        assert!(parse_size("axb").is_err());
    }

    #[test]
    fn arguments_are_consistent() {
        // clap checks its own configuration (conflicts, defaults) here.
        use clap::CommandFactory;
        Args::command().debug_assert();
    }

    #[test]
    fn render_scales_parse() {
        assert_eq!(parse_scale("auto"), Ok(RenderScale::Auto));
        assert_eq!(parse_scale("0.5"), Ok(RenderScale::Fixed(0.5)));
        assert!(parse_scale("2").is_err());
        assert!(parse_scale("fast").is_err());
    }

    #[test]
    fn spawning_avoids_pillars() {
        let cache = ChunkCache::new(GeneratorVersion::V1, LevelId::ZERO, 4096);
        let solid = |cell: CellPos| {
            let (x, y) = cell.local();
            cache.get(cell.chunk()).cells[y][x].solid
        };
        // Find a pillar near the origin and ask to start inside it.
        let pillar = (-50..50)
            .flat_map(|y| (-50..50).map(move |x| CellPos::new(x * 8, y * 8)))
            .find(|&cell| solid(cell))
            .expect("there are pillars within 200 m of the origin");
        let start = nearest_floor(&cache, pillar);
        assert_ne!(start, pillar);
        let (dx, dy) = pillar.delta_to(start);
        assert_eq!(dx.abs().max(dy.abs()), 1, "should be an adjacent cell");
        assert!(!solid(start));
        // A request that is already floor is left alone.
        assert_eq!(nearest_floor(&cache, start), start);
    }
}

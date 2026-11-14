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
use bevy::window::WindowResolution;
use clap::Parser;

mod audio;
mod collide;
mod hud;
mod lights;
mod mesh;
mod player;
mod space;
mod surface;
mod world;

use space::{CELL, ChunkStore, Plan, RenderOrigin};

/// Explore the Backrooms. Click to look around, WASD to walk, Shift to run,
/// Escape to free the mouse.
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
    /// How fixtures cast light. `point` is cheaper on weak graphics.
    #[arg(long, value_enum, default_value = "rect")]
    lights: lights::LightKind,
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
}

fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let (w, h) = text.split_once('x').ok_or("expected WIDTHxHEIGHT, for example 1600x900")?;
    let parse = |n: &str| n.parse::<u32>().ok().filter(|n| (160..=8192).contains(n));
    parse(w).zip(parse(h)).ok_or_else(|| "width and height must be numbers from 160 to 8192".into())
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
fn nearest_floor(store: &mut ChunkStore, wanted: CellPos) -> CellPos {
    for ring in 0..8i32 {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs().max(dy.abs()) != ring {
                    continue;
                }
                let cell = wanted.offset(dx, dy);
                store.ensure(cell.chunk());
                if !store.plan(cell).solid(0, 0) {
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
}

/// Frames to wait before the picture: long enough for every chunk to be
/// built and every shader compiled.
const CAPTURE_AFTER: u32 = 90;

fn capture(mut commands: Commands, capture: Res<Capture>, mut frame: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frame += 1;
    if *frame == CAPTURE_AFTER {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(capture.path.clone()));
    }
    // The file is written in the background; leave once it has appeared, or
    // give up after a while rather than hang.
    let saved = *frame > CAPTURE_AFTER + 10 && capture.path.exists();
    if saved || *frame > CAPTURE_AFTER + 1200 {
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

    let mut store = ChunkStore::new(address.version, address.level);
    let start = nearest_floor(&mut store, address.cell);
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

    let window =
        Window { title: "Backrooms".into(), resolution: WindowResolution::new(args.size.0, args.size.1), ..default() };
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: Some(window), ..default() }))
        .insert_resource(ClearColor(Color::BLACK))
        // A little light with no source, standing in for the light that
        // bounces between walls. Real bounce lighting comes later.
        .insert_resource(GlobalAmbientLight { color: Color::srgb(1.0, 0.93, 0.7), brightness: 42.0, ..default() })
        .insert_resource(origin)
        .insert_resource(store)
        .insert_resource(spawn)
        .insert_resource(args.lights)
        .insert_resource(world::ViewDistance(args.view))
        .insert_resource(world::Ceiling(info.ceiling_mm as f32 / 1000.0))
        .add_plugins((
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
        app.insert_resource(Capture { path }).add_systems(Update, capture);
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
    fn spawning_avoids_pillars() {
        let mut store = ChunkStore::new(GeneratorVersion::V1, LevelId::ZERO);
        // Find a pillar near the origin and ask to start inside it.
        let mut pillar = None;
        'search: for y in (-400..400).step_by(8) {
            for x in (-400..400).step_by(8) {
                let cell = CellPos::new(x, y);
                store.ensure(cell.chunk());
                if store.plan(cell).solid(0, 0) {
                    pillar = Some(cell);
                    break 'search;
                }
            }
        }
        let pillar = pillar.expect("there are pillars within 200 m of the origin");
        let start = nearest_floor(&mut store, pillar);
        assert_ne!(start, pillar);
        let (dx, dy) = pillar.delta_to(start);
        assert_eq!(dx.abs().max(dy.abs()), 1, "should be an adjacent cell");
        assert!(!store.plan(start).solid(0, 0));
        // A request that is already floor is left alone.
        assert_eq!(nearest_floor(&mut store, start), start);
    }
}

//! `backrooms`: look at the generated world without running the game.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufWriter, Write as _};
use std::path::PathBuf;
use std::process::ExitCode;

use backrooms_core::generator::v1::golden;
use backrooms_core::{
    Address, CellPos, Chunk, ChunkPos, Dir, Edge, GenError, GeneratorVersion, LEVEL_COUNT, LevelId, generate_chunk,
    level_info, zstack,
};
use clap::{Args, Parser, Subcommand};
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressStyle};
use rayon::prelude::*;

mod render;

/// Inspect the Backrooms world generator.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the chunk containing a cell as text.
    Chunk {
        #[command(flatten)]
        at: Location,
    },
    /// Write a floor plan of a square block of chunks as a PNG.
    Map {
        #[command(flatten)]
        at: Location,
        /// Side of the block, in chunks (8 m each), centred on the location.
        #[arg(long, default_value_t = 16, value_parser = clap::value_parser!(u32).range(1..=512))]
        chunks: u32,
        /// Pixels per cell.
        #[arg(long, default_value_t = 4, value_parser = clap::value_parser!(u32).range(1..=32))]
        scale: u32,
        /// What to colour the map by.
        #[arg(long, value_enum, default_value = "plan")]
        layer: render::Layer,
        /// Outline the chunks.
        #[arg(long)]
        grid: bool,
        /// File to write.
        #[arg(long, short, default_value = "map.png")]
        out: PathBuf,
    },
    /// List the vertical order of the 128 levels.
    Stack,
    /// Find the nearest oddities to a location, with where to stand to see
    /// each one.
    Find {
        /// What to look for.
        #[arg(value_enum)]
        what: Oddity,
        #[command(flatten)]
        at: Location,
        /// How many to list.
        #[arg(long, default_value_t = 5, value_parser = clap::value_parser!(u32).range(1..=100))]
        count: u32,
        /// How far to search, in chunks of 8 m.
        #[arg(long, default_value_t = 64, value_parser = clap::value_parser!(i32).range(1..=512))]
        radius: i32,
    },
    /// Print the golden-hash table for the current generator.
    Golden,
}

/// Something out of the ordinary to search for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
enum Oddity {
    /// An unglazed opening in a wall.
    Window,
    /// A low hole through a wall.
    Crawl,
    /// A place where the walls are well out of true.
    Skew,
    /// A dark doorway that leads somewhere far away on the same level.
    Portal,
    /// A piece of wall that is not there, and leads off the level.
    Noclip,
    /// A stairwell to another level.
    Stairs,
}

/// A place to stand and a direction to face (degrees anticlockwise from
/// north, as the game's --yaw takes it) to see an oddity.
struct Sighting {
    stand: CellPos,
    yaw: i32,
}

/// The oddities in one chunk.
fn sightings(chunk: &Chunk, what: Oddity, found: &mut Vec<Sighting>) {
    let origin = chunk.pos.origin();
    let wanted = match what {
        Oddity::Window => Edge::Window,
        Oddity::Crawl => Edge::Crawl,
        Oddity::Portal => Edge::Portal,
        Oddity::Noclip => Edge::Phase,
        Oddity::Stairs => {
            if let Some(stair) = chunk.stair {
                // Two bays out from the open side, looking back at it.
                let (dx, dy) = stair.facing.step();
                let stand = origin.offset(stair.x as i32 + 1 + dx * 6, stair.y as i32 + 1 + dy * 6);
                let yaw = match stair.facing {
                    Dir::North => 180,
                    Dir::East => 90,
                    Dir::South => 0,
                    Dir::West => -90,
                };
                found.push(Sighting { stand, yaw });
            }
            return;
        }
        Oddity::Skew => {
            // Only lattice points pushed at least 15 cm: less is hard to see.
            for (j, row) in chunk.skew.iter().enumerate().take(4) {
                for (i, skew) in row.iter().enumerate().take(4) {
                    if skew.dx.unsigned_abs().max(skew.dy.unsigned_abs()) >= 15 {
                        found.push(Sighting { stand: origin.offset(i as i32 * 4 + 1, j as i32 * 4 + 1), yaw: 0 });
                    }
                }
            }
            return;
        }
    };
    // A feature fills the two middle cells of a wall piece; reporting the
    // first of the pair lists each feature once. Index 16 belongs to the
    // next chunk.
    for y in 0..16usize {
        for x in 0..16usize {
            let here = origin.offset(x as i32, y as i32);
            if y % 4 == 1 && chunk.west[y][x] == wanted {
                // The wall runs north; stand a metre west of it, facing east.
                found.push(Sighting { stand: here.offset(-2, 0), yaw: -90 });
            }
            if x % 4 == 1 && chunk.south[y][x] == wanted {
                // The wall runs east; stand a metre south of it, facing north.
                found.push(Sighting { stand: here.offset(0, -2), yaw: 0 });
            }
        }
    }
}

/// Searches outwards from `address` in square rings of chunks until enough
/// has been found, and returns the nearest `count`.
fn find(address: Address, what: Oddity, count: usize, radius: i32) -> Result<Vec<Sighting>, String> {
    let centre = address.cell.chunk();
    let mut found = Vec::new();
    for ring in 0..=radius {
        for dy in -ring..=ring {
            for dx in -ring..=ring {
                if dx.abs().max(dy.abs()) == ring {
                    let chunk = generate_chunk(address.version, address.level, centre.offset(dx, dy))
                        .map_err(|e| e.to_string())?;
                    sightings(&chunk, what, &mut found);
                }
            }
        }
        // One more ring past the first hits, since a square ring's corners
        // are further away than the next ring's sides.
        if found.len() >= count * 2 {
            break;
        }
    }
    let distance = |s: &Sighting| {
        let (dx, dy) = address.cell.delta_to(s.stand);
        (dx as i64).pow(2) + (dy as i64).pow(2)
    };
    found.sort_by_key(distance);
    found.truncate(count);
    Ok(found)
}

/// Where to look. Either a full address, or level and coordinates.
#[derive(Args)]
struct Location {
    /// An address such as backrooms://v1/0/120/-45. Overrides the other flags.
    #[arg(long)]
    address: Option<Address>,
    /// Level id, 0-127.
    #[arg(long, default_value_t = 0)]
    level: u8,
    /// Cell x (east is positive).
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    x: i32,
    /// Cell y (north is positive).
    #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
    y: i32,
}

impl Location {
    fn resolve(&self) -> Result<Address, String> {
        if let Some(address) = self.address {
            return Ok(address);
        }
        let level = LevelId::new(self.level).ok_or("level must be a number from 0 to 127")?;
        Ok(Address { version: GeneratorVersion::LATEST, level, cell: CellPos::new(self.x, self.y) })
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// `Box<dyn Error>` would also work as the error type; a `String` keeps this
/// small tool simple, since every error ends up as one printed line anyway.
fn run(cli: Cli) -> Result<(), String> {
    // Everything is gathered into one string and written at the end.
    // `writeln!` into a `String` cannot fail, hence the ignored results.
    let mut out = String::new();
    match cli.command {
        Command::Chunk { at } => {
            let address = at.resolve()?;
            let pos = address.cell.chunk();
            let chunk = generate_chunk(address.version, address.level, pos).map_err(|e| e.to_string())?;
            let origin = pos.origin();
            let _ = writeln!(
                out,
                "{address}  chunk {},{}  cells x {}..={}  y {}..={}  digest {:016x}",
                pos.x(),
                pos.y(),
                origin.x,
                origin.offset(15, 15).x,
                origin.y,
                origin.offset(15, 15).y,
                chunk.digest()
            );
            out.push_str(&render::ascii(&chunk));
        }
        Command::Map { at, chunks: side, scale, layer, grid, out: path } => {
            let address = at.resolve()?;
            let chunks = generate_block(address, side)?;
            let image = render::map(&chunks, side, scale, layer, grid);
            write_png(&path, &image, &address).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
            let metres = side * 8;
            let _ = writeln!(
                out,
                "{}: {side} x {side} chunks ({metres} m x {metres} m) around {address}, {} x {} px",
                path.display(),
                image.width,
                image.height
            );
        }
        Command::Stack => {
            let version = GeneratorVersion::LATEST;
            out.push_str("depth  level  group    status\n");
            for depth in 0..LEVEL_COUNT as u8 {
                let level = zstack::level_at(version, depth).expect("depth is inside the stack");
                let info = level_info(version, level);
                let status = if info.built { format!("LEVEL {}", info.name) } else { "reserved".into() };
                let _ = writeln!(out, "{depth:>5}  {:>5}  {:<7}  {status}", level.get(), info.group.name());
            }
        }
        Command::Find { what, at, count, radius } => {
            let address = at.resolve()?;
            let found = find(address, what, count as usize, radius)?;
            if found.is_empty() {
                let _ = writeln!(out, "Nothing within {radius} chunks of {address}. Try a larger --radius.");
            }
            for sighting in found {
                let (dx, dy) = address.cell.delta_to(sighting.stand);
                // Half a metre per cell.
                let metres = ((dx as f64).hypot(dy as f64) / 2.0).round();
                let _ = writeln!(
                    out,
                    "{metres:>5} m away: --address {} --yaw {}",
                    Address { cell: sighting.stand, ..address },
                    sighting.yaw
                );
            }
        }
        Command::Golden => {
            let _ = write!(out, "{}", golden::table());
        }
    }
    emit(&out)
}

/// Writes to standard output. `println!` panics if the reader has gone away
/// (as in `backrooms golden | head -3`); a closed pipe is a normal way for a
/// command-line tool to be told "that's enough", so it counts as success.
fn emit(text: &str) -> Result<(), String> {
    match io::stdout().lock().write_all(text.as_bytes()) {
        Err(e) if e.kind() != io::ErrorKind::BrokenPipe => Err(format!("cannot write output: {e}")),
        _ => Ok(()),
    }
}

/// Generates a `side` x `side` block of chunks centred on `address`, in
/// row-major order from the south-west.
///
/// The chunks are generated in parallel. `par_iter().map().collect()` keeps
/// the results in input order no matter which thread finishes first, and each
/// chunk is a pure function of its position, so the output cannot depend on
/// scheduling.
fn generate_block(address: Address, side: u32) -> Result<Vec<Chunk>, String> {
    let centre = address.cell.chunk();
    let half = (side / 2) as i32;
    let positions: Vec<ChunkPos> =
        (0..side as i32).flat_map(|cy| (0..side as i32).map(move |cx| centre.offset(cx - half, cy - half))).collect();

    let bar = ProgressBar::new(positions.len() as u64).with_style(
        ProgressStyle::with_template("generating {bar:40} {percent:>3}%  {pos}/{len} chunks  {eta} left")
            .expect("template is valid"),
    );
    // Collecting an iterator of `Result`s into `Result<Vec<_>, _>` stops at
    // the first error and returns it.
    let chunks: Result<Vec<Chunk>, GenError> = positions
        .par_iter()
        .progress_with(bar)
        .map(|&pos| generate_chunk(address.version, address.level, pos))
        .collect();
    chunks.map_err(|e| e.to_string())
}

fn write_png(path: &PathBuf, image: &render::Image, address: &Address) -> Result<(), Box<dyn std::error::Error>> {
    let mut encoder = png::Encoder::new(BufWriter::new(File::create(path)?), image.width, image.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.add_text_chunk("Address".into(), address.to_string())?;
    encoder.add_text_chunk("Software".into(), concat!("backrooms-cli ", env!("CARGO_PKG_VERSION")).into())?;
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.rgb)?;
    Ok(())
}

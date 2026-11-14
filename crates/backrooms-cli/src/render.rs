//! Turns chunks into something a person can look at. Inspection only: this is
//! the one place in the tool chain where colours are picked by taste.

use backrooms_core::chunk::N;
use backrooms_core::{Chunk, Edge, LightState};

/// One chunk as text, north at the top. Each cell is two characters wide so
/// that the plan comes out roughly square in a terminal.
///
/// ```text
/// #  wall or pillar        o  light, on
/// ~  light, flickering     x  light, dead
/// =  window                .  crawl hole (: when the wall runs north)
/// ```
// The loops below use `x` both as an array index and as a corner number that
// runs one past the last cell, which the iterator form clippy suggests
// cannot express.
#[allow(clippy::needless_range_loop)]
pub fn ascii(chunk: &Chunk) -> String {
    // Windows and crawl holes are built into walls, so they count as wall
    // when deciding where corner posts go.
    let wall = |edge: Edge| edge != Edge::Open;
    let across = |edge: Edge| match edge {
        Edge::Open => "  ",
        Edge::Wall => "##",
        Edge::Window => "==",
        Edge::Crawl => "..",
    };
    let along = |edge: Edge| match edge {
        Edge::Open => ' ',
        Edge::Wall => '#',
        Edge::Window => '=',
        Edge::Crawl => ':',
    };
    // A corner post is drawn wherever any wall touches the corner.
    let post = |x: usize, y: usize| {
        (x < N && wall(chunk.south[y][x]))
            || (x > 0 && wall(chunk.south[y][x - 1]))
            || (y < N && wall(chunk.west[y][x]))
            || (y > 0 && wall(chunk.west[y - 1][x]))
    };

    let mut out = String::new();
    for y in (0..=N).rev() {
        // The line of edges along the north side of row y - 1...
        for x in 0..=N {
            out.push(if post(x, y) { '#' } else { ' ' });
            if x < N {
                out.push_str(across(chunk.south[y][x]));
            }
        }
        out.push('\n');
        if y == 0 {
            break;
        }
        // ...then row y - 1 itself.
        let row = y - 1;
        for x in 0..=N {
            out.push(along(chunk.west[row][x]));
            if x < N {
                out.push_str(if chunk.cells[row][x].solid {
                    "##"
                } else {
                    match chunk.light_at(x, row) {
                        Some(LightState::On) => "oo",
                        Some(LightState::Flicker) => "~~",
                        Some(LightState::Dead) => "xx",
                        None => "  ",
                    }
                });
            }
        }
        out.push('\n');
    }
    out
}

/// What the map colours show.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Layer {
    /// Floor plan with damp patches and lights.
    Plan,
    /// Moisture only.
    Damp,
    /// Wear only.
    Wear,
}

/// An RGB image, row 0 at the top.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

impl Image {
    fn new(width: u32, height: u32, fill: [u8; 3]) -> Self {
        Self { width, height, rgb: fill.repeat((width * height) as usize) }
    }

    fn rect(&mut self, x: u32, y: u32, w: u32, h: u32, colour: [u8; 3]) {
        for py in y..(y + h).min(self.height) {
            for px in x..(x + w).min(self.width) {
                let i = ((py * self.width + px) * 3) as usize;
                self.rgb[i..i + 3].copy_from_slice(&colour);
            }
        }
    }
}

const WALL: [u8; 3] = [46, 38, 18];
const LIGHT_ON: [u8; 3] = [255, 255, 255];
const LIGHT_FLICKER: [u8; 3] = [255, 120, 30];
const LIGHT_DEAD: [u8; 3] = [70, 70, 80];
const CHUNK_GRID: [u8; 3] = [90, 150, 255];
const WINDOW: [u8; 3] = [60, 170, 230];
const CRAWL: [u8; 3] = [40, 190, 90];
const SKEW: [u8; 3] = [220, 40, 200];

/// Colour of a wall edge on the map, or `None` for no wall.
fn edge_colour(edge: Edge) -> Option<[u8; 3]> {
    match edge {
        Edge::Open => None,
        Edge::Wall => Some(WALL),
        Edge::Window => Some(WINDOW),
        Edge::Crawl => Some(CRAWL),
    }
}

fn mix(a: [u8; 3], b: [u8; 3], t: u8) -> [u8; 3] {
    let f = |a: u8, b: u8| ((a as u32 * (255 - t as u32) + b as u32 * t as u32) / 255) as u8;
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

/// Paints a square block of chunks. `chunks` is row-major from the
/// south-west: index `cy * side + cx`. `scale` is pixels per cell.
pub fn map(chunks: &[Chunk], side: u32, scale: u32, layer: Layer, grid: bool) -> Image {
    assert_eq!(chunks.len() as u32, side * side);
    let n = N as u32;
    let line = (scale / 4).max(1);
    let size = side * n * scale + line;
    let mut image = Image::new(size, size, WALL);

    for (index, chunk) in chunks.iter().enumerate() {
        let (cx, cy) = (index as u32 % side, index as u32 / side);
        // Pixel of the chunk's south-west corner; image rows run downwards.
        let left = cx * n * scale;
        let bottom = size - line - cy * n * scale;
        let cell_px = |x: u32, y: u32| (left + x * scale, bottom - (y + 1) * scale);

        for y in 0..n {
            for x in 0..n {
                let cell = chunk.cells[y as usize][x as usize];
                let colour = match layer {
                    Layer::Plan if cell.solid => WALL,
                    Layer::Plan => mix([214, 196, 110], [150, 132, 78], cell.damp),
                    Layer::Damp => mix([235, 235, 225], [20, 60, 140], cell.damp),
                    Layer::Wear => mix([235, 235, 225], [120, 40, 20], cell.wear),
                };
                let (px, py) = cell_px(x, y);
                image.rect(px, py, scale, scale, colour);
            }
        }

        if layer == Layer::Plan {
            for f in chunk.fixtures() {
                let colour = match f.state {
                    LightState::On => LIGHT_ON,
                    LightState::Flicker => LIGHT_FLICKER,
                    LightState::Dead => LIGHT_DEAD,
                };
                let (px, py) = cell_px(f.x as u32, f.y as u32);
                let inset = scale / 4;
                image.rect(px + inset, py + inset, 2 * scale - 2 * inset, scale - 2 * inset, colour);
            }
        }

        if grid {
            image.rect(left, bottom - n * scale, 1, n * scale, CHUNK_GRID);
            image.rect(left, bottom, n * scale, 1, CHUNK_GRID);
        }

        for y in 0..n {
            for x in 0..=n {
                if let Some(colour) = edge_colour(chunk.west[y as usize][x as usize]) {
                    let (px, py) = cell_px(x, y);
                    image.rect(px, py, line, scale + line, colour);
                }
            }
        }
        for y in 0..=n {
            for x in 0..n {
                if let Some(colour) = edge_colour(chunk.south[y as usize][x as usize]) {
                    // Not via `cell_px`: for the north boundary (y == 16) of
                    // the top row of chunks that would step above the image.
                    image.rect(left + x * scale, bottom - y * scale, scale + line, line, colour);
                }
            }
        }

        // Lattice points that are out of true, as a dot that grows with
        // how far they have been pushed.
        if layer == Layer::Plan {
            for (j, row) in chunk.skew.iter().enumerate() {
                for (i, skew) in row.iter().enumerate() {
                    let push = skew.dx.unsigned_abs().max(skew.dy.unsigned_abs()) as u32;
                    if push > 0 && i < 4 && j < 4 {
                        let dot = (line + push * scale / 40).max(2);
                        let (px, py) = (left + i as u32 * 4 * scale, bottom - j as u32 * 4 * scale);
                        image.rect(px.saturating_sub(dot / 2), py.saturating_sub(dot / 2), dot, dot, SKEW);
                    }
                }
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    use backrooms_core::{ChunkPos, GeneratorVersion, LevelId, generate_chunk};

    #[test]
    fn ascii_has_the_expected_shape() {
        let chunk = generate_chunk(GeneratorVersion::V1, LevelId::ZERO, ChunkPos::new(0, 0)).unwrap();
        let text = ascii(&chunk);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2 * N + 1);
        assert!(lines.iter().all(|line| line.chars().count() == 3 * N + 1));
    }

    #[test]
    fn map_has_the_expected_size() {
        let chunk = generate_chunk(GeneratorVersion::V1, LevelId::ZERO, ChunkPos::new(0, 0)).unwrap();
        let image = map(&[chunk], 1, 4, Layer::Plan, true);
        assert_eq!((image.width, image.height), (65, 65));
        assert_eq!(image.rgb.len(), 65 * 65 * 3);
    }
}

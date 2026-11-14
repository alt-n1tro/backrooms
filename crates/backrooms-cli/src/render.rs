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
/// ```
pub fn ascii(chunk: &Chunk) -> String {
    let wall = |edge: Edge| edge == Edge::Wall;
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
                out.push_str(if wall(chunk.south[y][x]) { "##" } else { "  " });
            }
        }
        out.push('\n');
        if y == 0 {
            break;
        }
        // ...then row y - 1 itself.
        let row = y - 1;
        for x in 0..=N {
            out.push(if wall(chunk.west[row][x]) { '#' } else { ' ' });
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
                if chunk.west[y as usize][x as usize] == Edge::Wall {
                    let (px, py) = cell_px(x, y);
                    image.rect(px, py, line, scale + line, WALL);
                }
            }
        }
        for y in 0..=n {
            for x in 0..n {
                if chunk.south[y as usize][x as usize] == Edge::Wall {
                    // Not via `cell_px`: for the north boundary (y == 16) of
                    // the top row of chunks that would step above the image.
                    image.rect(left + x * scale, bottom - y * scale, scale + line, line, WALL);
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

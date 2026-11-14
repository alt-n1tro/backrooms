//! Properties that must hold everywhere in the world, checked on samples
//! around the origin, across the world's wrap-around corner, and far out.

use std::collections::VecDeque;

use backrooms_core::chunk::N;
use backrooms_core::hash::Key;
use backrooms_core::{Chunk, ChunkPos, Dir, Edge, GenError, GeneratorVersion, LevelId, generate_chunk};

const V1: GeneratorVersion = GeneratorVersion::V1;
const L0: LevelId = LevelId::ZERO;

fn chunk(pos: ChunkPos) -> Chunk {
    generate_chunk(V1, L0, pos).expect("level 0 is built")
}

/// South-west corners of 12 x 12 chunk blocks worth sampling.
fn sample_blocks() -> Vec<ChunkPos> {
    let mut blocks = vec![
        ChunkPos::new(-6, -6),
        // Straddles the corner where both axes wrap.
        ChunkPos::new(ChunkPos::MAX - 5, ChunkPos::MAX - 5),
        ChunkPos::new(ChunkPos::MAX - 5, -6),
    ];
    // Plus pseudo-random places. Using the hash as a test-data generator
    // keeps the samples identical on every run.
    let key = Key::new(0x7E57);
    blocks.extend((0..12u64).map(|i| {
        let roll = key.with(i).roll().0;
        ChunkPos::new(roll as i32, (roll >> 32) as i32)
    }));
    blocks
}

fn for_each_sample(mut f: impl FnMut(ChunkPos)) {
    for block in sample_blocks() {
        for dy in 0..12 {
            for dx in 0..12 {
                f(block.offset(dx, dy));
            }
        }
    }
}

#[test]
fn generation_is_pure() {
    for_each_sample(|pos| assert_eq!(chunk(pos), chunk(pos)));
}

#[test]
fn neighbouring_chunks_agree_on_their_shared_edge() {
    for_each_sample(|pos| {
        let here = chunk(pos);
        let east = chunk(pos.offset(1, 0));
        let north = chunk(pos.offset(0, 1));
        for i in 0..N {
            assert_eq!(here.west[i][N], east.west[i][0], "east seam of {pos:?}, row {i}");
            assert_eq!(here.south[N][i], north.south[0][i], "north seam of {pos:?}, column {i}");
        }
    });
}

#[test]
fn pillars_are_walled_and_lights_hang_over_floor() {
    for_each_sample(|pos| {
        let c = chunk(pos);
        for y in 0..N {
            for x in 0..N {
                if c.cells[y][x].solid {
                    for dir in [Dir::North, Dir::East, Dir::South, Dir::West] {
                        assert!(!c.open_towards(x, y, dir), "pillar at {pos:?} {x},{y} is open to {dir:?}");
                    }
                }
            }
        }
        for f in c.fixtures() {
            let (x, y) = (f.x as usize, f.y as usize);
            assert!(x + 1 < N && y < N, "fixture leaves chunk {pos:?}");
            assert!(!c.cells[y][x].solid && !c.cells[y][x + 1].solid, "fixture over a pillar in {pos:?}");
            assert_eq!(c.west[y][x + 1], Edge::Open, "wall cuts through a fixture in {pos:?}");
        }
    });
}

/// Every gap in a wall line is at least two cells (1 m) wide, so there is no
/// slot the player could see through but not walk through.
#[test]
fn no_opening_is_narrower_than_one_metre() {
    let check = |edges: [Edge; 4], what: &str| {
        let open: Vec<bool> = edges.iter().map(|&e| e == Edge::Open).collect();
        for i in 0..4 {
            let alone = open[i] && (i == 0 || !open[i - 1]) && (i == 3 || !open[i + 1]);
            assert!(!alone, "half-metre slot in {what}: {edges:?}");
        }
    };
    for_each_sample(|pos| {
        let c = chunk(pos);
        // Wall lines run along every fourth row and column of edges.
        for line in (0..=N).step_by(4) {
            for start in (0..N).step_by(4) {
                let ns: [Edge; 4] = std::array::from_fn(|b| c.west[start + b][line]);
                let ew: [Edge; 4] = std::array::from_fn(|b| c.south[line][start + b]);
                check(ns, &format!("{pos:?} north-south line {line} from {start}"));
                check(ew, &format!("{pos:?} east-west line {line} from {start}"));
            }
        }
    });
}

/// Walls only stand on the 2 m lattice, except around pillars.
#[test]
fn walls_stay_on_the_lattice() {
    for_each_sample(|pos| {
        let c = chunk(pos);
        for y in 0..N {
            for x in 0..N {
                let beside_pillar = c.cells[y][x].solid || (x > 0 && c.cells[y][x - 1].solid);
                if x % 4 != 0 && !beside_pillar {
                    assert_eq!(c.west[y][x], Edge::Open, "stray wall in {pos:?} at {x},{y}");
                }
                let beside_pillar = c.cells[y][x].solid || (y > 0 && c.cells[y - 1][x].solid);
                if y % 4 != 0 && !beside_pillar {
                    assert_eq!(c.south[y][x], Edge::Open, "stray wall in {pos:?} at {x},{y}");
                }
            }
        }
    });
}

/// Windows and crawl holes only ever sit in the middle metre of a complete
/// 2 m piece of wall, with half a metre of plain wall on each side.
#[test]
fn features_sit_in_the_middle_of_whole_wall_pieces() {
    let check = |edges: [Edge; 4], what: &str| {
        let special = |e: Edge| e == Edge::Window || e == Edge::Crawl;
        if edges.iter().any(|&e| special(e)) {
            assert!(
                edges[0] == Edge::Wall && edges[3] == Edge::Wall && special(edges[1]) && edges[1] == edges[2],
                "misplaced feature in {what}: {edges:?}"
            );
        }
    };
    let (mut windows, mut crawls) = (0, 0);
    for_each_sample(|pos| {
        let c = chunk(pos);
        // Lines 0, 4, 8 and 12: line 16 is the next chunk's line 0.
        for line in (0..N).step_by(4) {
            for start in (0..N).step_by(4) {
                let ns: [Edge; 4] = std::array::from_fn(|b| c.west[start + b][line]);
                let ew: [Edge; 4] = std::array::from_fn(|b| c.south[line][start + b]);
                check(ns, &format!("{pos:?} north-south line {line} from {start}"));
                check(ew, &format!("{pos:?} east-west line {line} from {start}"));
                for piece in [ns, ew] {
                    windows += (piece[1] == Edge::Window) as u32;
                    crawls += (piece[1] == Edge::Crawl) as u32;
                }
            }
        }
        // Off the lattice lines there is never a feature.
        for y in 0..N {
            for x in 0..N {
                if x % 4 != 0 {
                    assert!(matches!(c.west[y][x], Edge::Open | Edge::Wall));
                }
                if y % 4 != 0 {
                    assert!(matches!(c.south[y][x], Edge::Open | Edge::Wall));
                }
            }
        }
    });
    // Windows are common; crawl holes exist only at secret rooms and are
    // far rarer.
    assert!(windows > 300, "{windows} windows");
    assert!(crawls >= 5 && crawls * 5 < windows, "{crawls} crawl holes against {windows} windows");
}

/// Lattice points on a chunk boundary belong to both chunks, which must
/// agree on how far each is out of true. Most of the world is square.
#[test]
fn skew_is_shared_bounded_and_mostly_absent() {
    let (mut points, mut skewed) = (0, 0);
    for_each_sample(|pos| {
        let here = chunk(pos);
        let east = chunk(pos.offset(1, 0));
        let north = chunk(pos.offset(0, 1));
        for i in 0..5 {
            assert_eq!(here.skew[i][4], east.skew[i][0], "east seam of {pos:?}");
            assert_eq!(here.skew[4][i], north.skew[0][i], "north seam of {pos:?}");
        }
        for skew in here.skew.as_flattened() {
            assert!(skew.dx.abs() <= 28 && skew.dy.abs() <= 28, "{skew:?} in {pos:?}");
            points += 1;
            skewed += (skew.dx != 0 || skew.dy != 0) as u32;
        }
    });
    assert!(skewed > 100, "only {skewed} skewed lattice points");
    assert!(skewed * 4 < points, "{skewed} of {points} lattice points are skewed");
}

/// Flood-fills a `side` x `side` block of chunks and returns
/// `(cells in the largest connected area, walkable cells)`.
fn largest_area(south_west: ChunkPos, side: usize) -> (usize, usize) {
    let areas = areas(south_west, side, false);
    (areas.iter().copied().max().unwrap_or(0), areas.iter().sum())
}

/// Sizes of the connected areas of a block of chunks, for a player who can
/// (`crawling`) or cannot use crawl holes.
fn areas(south_west: ChunkPos, side: usize, crawling: bool) -> Vec<usize> {
    let chunks: Vec<Chunk> =
        (0..side * side).map(|i| chunk(south_west.offset((i % side) as i32, (i / side) as i32))).collect();
    let width = side * N;
    let at = |x: usize, y: usize| (&chunks[(y / N) * side + x / N], x % N, y % N);
    let walkable = |x: usize, y: usize| {
        let (c, lx, ly) = at(x, y);
        !c.cells[ly][lx].solid
    };

    let passable = |c: &Chunk, lx: usize, ly: usize, dir: Dir| {
        let edge = match dir {
            Dir::West => c.west[ly][lx],
            Dir::East => c.west[ly][lx + 1],
            Dir::South => c.south[ly][lx],
            Dir::North => c.south[ly + 1][lx],
        };
        edge == Edge::Open || (crawling && edge == Edge::Crawl)
    };
    let mut seen = vec![false; width * width];
    let mut found = Vec::new();
    let mut queue = VecDeque::new();
    for start in 0..width * width {
        let (sx, sy) = (start % width, start / width);
        if seen[start] || !walkable(sx, sy) {
            continue;
        }
        seen[start] = true;
        queue.push_back((sx, sy));
        let mut area = 0;
        while let Some((x, y)) = queue.pop_front() {
            area += 1;
            let (c, lx, ly) = at(x, y);
            let steps = [
                (Dir::East, x + 1 < width, x + 1, y),
                (Dir::North, y + 1 < width, x, y + 1),
                (Dir::West, x > 0, x.wrapping_sub(1), y),
                (Dir::South, y > 0, x, y.wrapping_sub(1)),
            ];
            for (dir, inside, nx, ny) in steps {
                if inside && passable(c, lx, ly, dir) && !seen[ny * width + nx] {
                    seen[ny * width + nx] = true;
                    queue.push_back((nx, ny));
                }
            }
        }
        found.push(area);
    }
    found
}

/// Crawl holes lead only to places that cannot be walked to: on foot the
/// level falls into one big area plus sealed pockets, and crawling joins
/// the pockets to it.
#[test]
fn crawl_holes_lead_only_to_rooms_walking_cannot_reach() {
    let corner = ChunkPos::new(-24, -24);
    let mut walking = areas(corner, 48, false);
    let mut crawling = areas(corner, 48, true);
    walking.sort_unstable_by(|a, b| b.cmp(a));
    crawling.sort_unstable_by(|a, b| b.cmp(a));
    let pockets = walking.len() - 1;
    assert!(pockets >= 3, "only {pockets} secret rooms in 384 m square");
    // Each pocket is a small room, not a lost wing of the building.
    assert!(walking[1..].iter().all(|&cells| cells <= 6 * 16), "walking areas: {:?}", &walking[..walking.len().min(8)]);
    // Crawling reaches (nearly) all of them. A secret room on the edge of
    // the sample may have its hole on the far side.
    assert!(crawling.len() <= 1 + pockets / 3, "{} areas even when crawling, {pockets} pockets", crawling.len());
}

/// The level is one connected space. Inside a finite window a few pockets
/// near the border can only be reached by leaving the window, so the bar is
/// "almost everything" rather than "everything".
#[test]
fn the_level_is_connected() {
    for south_west in [ChunkPos::new(-24, -24), ChunkPos::new(ChunkPos::MAX - 23, ChunkPos::MAX - 23)] {
        let (largest, total) = largest_area(south_west, 48);
        let per_mille = largest * 1000 / total;
        assert!(per_mille >= 990, "only {per_mille} per mille of the floor around {south_west:?} is connected");
    }
}

#[test]
fn the_level_has_variety() {
    let (mut walls, mut pillars, mut on, mut dead, mut flicker, mut damp, mut edges) = (0, 0, 0, 0, 0, 0, 0);
    for_each_sample(|pos| {
        let c = chunk(pos);
        walls += c.west.as_flattened().iter().filter(|&&e| e == Edge::Wall).count();
        edges += c.west.as_flattened().len();
        pillars += c.cells.as_flattened().iter().filter(|cell| cell.solid).count();
        damp += c.cells.as_flattened().iter().filter(|cell| cell.damp > 0).count();
        for f in c.fixtures() {
            match f.state {
                backrooms_core::LightState::On => on += 1,
                backrooms_core::LightState::Dead => dead += 1,
                backrooms_core::LightState::Flicker => flicker += 1,
            }
        }
    });
    assert!(walls * 100 / edges >= 3 && walls * 100 / edges <= 25, "{walls} walls on {edges} edges");
    assert!(pillars > 50, "{pillars} pillars");
    assert!(on > dead && on > flicker && dead > 20 && flicker > 20, "lights: {on} on, {dead} dead, {flicker} flicker");
    assert!(damp > 1000, "{damp} damp cells");
}

#[test]
fn reserved_levels_have_no_chunks() {
    let reserved = LevelId::new(127).unwrap();
    assert_eq!(generate_chunk(V1, reserved, ChunkPos::new(0, 0)), Err(GenError::LevelNotBuilt(reserved)));
}

/// Generating the same chunks on several threads, each in a different order,
/// gives the same digests. (The generator has no shared state for threads to
/// fight over; this test is here to keep it that way.)
#[test]
fn results_do_not_depend_on_threads_or_order() {
    let positions: Vec<ChunkPos> = (0..256).map(|i| ChunkPos::new(i % 16 - 8, i / 16 - 8)).collect();
    let reference: Vec<u64> = positions.iter().map(|&p| chunk(p).digest()).collect();

    // `thread::scope` lets the threads borrow `positions` and `reference`
    // directly: the scope guarantees they finish before those are dropped.
    std::thread::scope(|scope| {
        for t in 0..4usize {
            let (positions, reference) = (&positions, &reference);
            scope.spawn(move || {
                // Thread t walks the list with its own stride and direction.
                let stride = [1, 7, 11, 13][t];
                for step in 0..positions.len() {
                    let i = (step * stride + t * 31) % positions.len();
                    let i = if t % 2 == 0 { i } else { positions.len() - 1 - i };
                    assert_eq!(chunk(positions[i]).digest(), reference[i]);
                }
            });
        }
    });
}

# Changelog

Every delivery gets a version (MAJOR.MINOR.PATCH), however small the change.
"Generator" lines say whether the world itself changed: once generator v1
ships, any such change will require a new generator version instead.

## 0.6.0 — Levels, portals, noclip, stairs

World (generator v1 changed; golden hashes re-blessed):

- 68 levels are built. Levels 1 to 63 are the yellow variants: Level 0's
  generator with its knobs turned by a hash of the level id (ceiling
  height, room sizes, corridors, doorways, columns, dead and flickering
  lights, damp, wear, skew, wallpaper tint). Levels 64 to 67 are the
  WAREHOUSE (concrete, 5.2 m ceilings, vast halls), the OFFICE (white
  walls, grey carpet, small rooms off corridors), the TUNNELS (low, dark,
  all corridor) and the TILES. These four reuse the room generator with
  different parameters and surfaces; they are not yet unique generators.
- Portals: a rare plain wall piece is a dark doorway paired with the same
  piece of a block at least 500 km away on the same level. Both ends roll
  the same hash, so most are two-way; where the far end is not a wall, it
  is one-way.
- Noclip: a rarer wall piece looks like wall and is not there. Each leads
  to a hashed built level at a hashed place.
- Stairs: very rarely, a room at least 6 m each way has a walled stairwell
  booth with one open side, going up or down to the nearest built level in
  that direction of the stack (empty slots are stepped over).

Game:

- Travel between places and levels: the render origin is redeclared at the
  destination, the scene is rebuilt there, and the surface material, fog
  and ceiling height are re-dressed for the level.
- Portals show nothing of where they lead (the doorway is black) and are
  walked through without a break. Noclip and stairs arrive behind a cut to
  black.
- Stair flights are drawn, and carry the view up or down as they are
  climbed.
- Four surface themes in the shader, and a per-level tint.

Tools:

- `backrooms find portal|noclip|stairs`; maps and chunk text show them.

## 0.5.0 — Rooms

World (generator v1 changed; golden hashes re-blessed):

- Level 0 is now laid out as rooms instead of independently hashed walls.
  Each 32 m block is divided the way a floor plan is: cut in two, then
  again, until the parts are room-sized, often leaving a 2 m corridor
  between the two sides of a cut. The result is offices off corridors,
  closets, and open halls with regular grids of columns.
- Every cut gets at least one doorway, as does every wall between blocks,
  so the level is connected by construction rather than by statistics.
- Secret rooms: a rare small room has no doorway, only one crawl hole.
  Crawl holes no longer appear anywhere else, so each one leads somewhere
  that cannot be walked into.
- Some cuts are left unbuilt, merging rooms into L-shapes and open-plan
  floors; corridors run straight through the walls they end at.
- Density now varies per block over 128 m zones.

Game:

- Fixed: the player wobbled and drifted near skewed walls. Collision now
  works directly against the walls as drawn, so nothing about the player
  changes near skew. Standing still is bit-for-bit still.
- Crouch also on Ctrl, crawl also on Z. Walking into a crawl hole while
  crouched drops to a crawl automatically.
- Less wallpaper peeling in heavily worn areas.

## 0.4.0 — Milestone 4a: Level 0 anomalies

Milestone 4 is delivered in three steps. This is the first; portals (4b) and
the 64 yellow variants (4c) follow.

World (generator v1 changed; golden hashes re-blessed):

- Rarity tiers: every site rolls into at most one of six tiers, each eight
  times rarer than the last, from 1 in 32 to 1 in 1,048,576.
- Interior windows: a common roll puts an unglazed opening in the middle
  metre of a complete 2 m wall piece.
- Crawl holes: an uncommon roll puts a 0.7 m high hole there instead.
- Skewed walls: in zones marked by a slow "wrongness" noise, points of the
  2 m wall lattice are pushed up to 28 cm out of true, so walls stop being
  parallel.
- None of these changes where a walking player can go, so every earlier
  guarantee (no sealed rooms, 1 m openings, connectivity) still holds.

Game:

- Crouch (C) and crawl (X); Space gets back up. Getting up waits until
  there is room. Only a crawling body fits a crawl hole.
- Wall slabs are built with their openings, lined with trim.
- Baked light passes partly through windows and crawl holes.
- Skewed zones are drawn and walked through consistently.
- The automatic render scale now follows the frame rate, shrinking or
  growing the 3D picture to hold 60 frames a second.

Tools:

- `backrooms find window|crawl|skew` lists the nearest ones with the
  options to start the game looking at each.
- Maps and chunk text show windows, crawl holes and skewed lattice points.

Not in this step: tilted floors (they need height-aware collision, which
arrives with stairs and slides) and portals.

## 0.3.1

- The running version is shown in the window title, printed at start-up and
  included in the F3 line, so a stale build is obvious.
- F3 and F11 have second keys, ` (left of 1) and F, for laptops whose F-keys
  need Fn held.
- Generator: unchanged.

## 0.3.0 — Milestone 3: performance pass

Performance:

- Lighting is baked per chunk instead of evaluated per pixel. The light of
  every steady panel, including a share for bounced light, is computed once
  when the chunk is built and stored in its mesh; about 500 live area lights
  become a few dozen (only flickering tubes stay live).
- Walls now block light, so rooms with dead tubes are dark.
- The 3D view is drawn at a capped internal size and stretched to the
  window, with the HUD at full sharpness. `--render-scale auto` (default)
  holds it to about 1080p worth of pixels; `--render-scale 1` is native.
- Depth prepass: the surface shader runs once per pixel instead of once per
  overlapping wall.
- Chunks are generated, lit and meshed on worker threads, with a shared
  least-recently-used chunk cache.
- Floating origin: the render origin follows the player in whole chunks, so
  precision no longer degrades with distance walked.
- Measured on a software renderer (relative numbers only): 356 ms to 151 ms
  per frame at 960 x 540, 1072 ms to 278 ms at 1920 x 1080, and 309 ms at
  2880 x 1800.

Feel:

- Jump with Space. Sprint by tapping W twice and holding it (Shift still
  works), with a slight widening of the view.
- Mouse look is about a third faster and no longer waits for the display's
  refresh (`--vsync` restores that; `--sensitivity` adjusts speed).
- Frame rate capped at 120 by default (`--fps`).
- The hum is far quieter (`--hum` scales it, 0 silences it).
- F3 shows frame rate and render size; F11 toggles fullscreen.

Not done, on purpose: cell-and-portal occlusion culling. With lighting baked
and a depth prepass, hidden surfaces cost almost nothing on Level 0, and the
portals of milestone 4 need a visibility structure of their own; building
one now would be building it twice.

- Generator: unchanged. Golden hashes are identical to 0.1.0.

## 0.2.0 — Milestone 2: walkable Level 0

- New crate `backrooms-game` (binary `backrooms-game`) on Bevy 0.20.
  - First-person walking: WASD or arrows, Shift to run, mouse look, sliding
    collision against the core's walls and pillars.
  - Spawn at an address, at coordinates, at the origin or at random
    (`--address`, `--x/--y`, `--random`); never inside a pillar.
  - Procedural surfaces with no textures: chevron wallpaper with grime, damp
    tide lines and peeling; carpet with damp patches and worn paths; ceiling
    tiles with water stains. All hashed from the true cell, so they are
    identical for every player and exact at any distance from 0,0.
  - Fluorescent fixtures as rectangular area lights (`--lights point` for a
    cheaper fallback), with dead tubes from the core and a deterministic
    flicker pattern per fixture.
  - Synthesised fluorescent hum whose loudness follows the lit tubes nearby
    and stutters with a flickering one.
  - HUD address readout, distance haze, bloom, `--screenshot` for a single
    picture of the starting view.
  - Chunks within `--view` (default 5, 40 m) are meshed on the main thread;
    the render origin is the starting chunk.
- Core: one loop in the Z-stack shuffle rewritten for a new compiler lint.
  No change in output.
- CI: the game is built, linted and tested on Linux; the core stays on all
  four platforms.
- Requires Rust 1.97.1 or newer for the game (the core still builds on 1.85).
- Generator: unchanged. Golden hashes are identical to 0.1.0.

## 0.1.0 — Milestone 1: core crate

- New crate `backrooms-core`: integer-only, stateless, `no_std`, no
  dependencies, no `unsafe`.
  - Torus coordinates (cell, chunk, region) and the shareable address
    `backrooms://v1/<level>/<x>/<y>`.
  - Counter-based hashing (SplitMix64 finaliser) keyed by seed, level,
    purpose and coordinates.
  - Fixed-point value noise, fBm and domain warping, seamless across the
    world edge.
  - 128 reserved level ids in four fixed groups, and the Z-stack shuffle,
    evaluated at compile time.
  - Level 0 chunk generation: walls on shared edges over three nested
    lattices, doorways, pillars, ceiling lights (on, dead, flickering),
    region brownouts and blackouts, damp and wear fields.
- New crate `backrooms-cli` (binary `backrooms`): `chunk` (text), `map`
  (PNG, parallel, with progress bar), `stack`, `golden`.
- Tests: golden hashes over 32 chunks and three 48 x 48 chunk blocks
  (origin, world corner, far out), seam agreement, wrap continuity,
  connectivity, minimum opening width, thread and order independence, and a
  source scan that rejects floats in the core.
- CI on x86_64 Linux, ARM64 Linux, ARM64 macOS and x86_64 Windows, in debug
  and release.
- Generator: v1 created (draft, not frozen).

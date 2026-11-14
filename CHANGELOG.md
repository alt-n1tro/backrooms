# Changelog

Every delivery gets a version (MAJOR.MINOR.PATCH), however small the change.
"Generator" lines say whether the world itself changed: once generator v1
ships, any such change will require a new generator version instead.

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

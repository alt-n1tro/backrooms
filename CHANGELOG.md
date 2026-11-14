# Changelog

Every delivery gets a version (MAJOR.MINOR.PATCH), however small the change.
"Generator" lines say whether the world itself changed: once generator v1
ships, any such change will require a new generator version instead.

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

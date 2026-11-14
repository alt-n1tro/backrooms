# Backrooms

A first-person exploration of the Backrooms in which the whole world is a
pure function of its coordinates: nothing is stored, and the same address is
the same place on every machine.

Current version: **0.1.0** (milestone 1 of 9, the core crate). See
[CHANGELOG.md](CHANGELOG.md).

## Layout

| Crate | What it is |
|---|---|
| `crates/backrooms-core` | The world function. Integers only, no state, no dependencies, `no_std`. |
| `crates/backrooms-cli` | `backrooms`, a headless tool for looking at the generated world. |

The game itself (`backrooms-game`, on Bevy) arrives with milestone 2.

## Try it

```bash
cargo run --release -p backrooms-cli -- chunk --x 0 --y 0
```

```bash
cargo run --release -p backrooms-cli -- map --chunks 32 --scale 6 --out map.png
```

```bash
cargo run --release -p backrooms-cli -- map --address backrooms://v1/0/120000/-45000 --layer damp
```

```bash
cargo run --release -p backrooms-cli -- stack
```

Map legend: white lights are on, orange flicker, grey are dead; dark squares
are pillars; brown patches are damp carpet. One chunk is 16 x 16 cells, 8 m
on a side.

![Level 0 around the origin, 96 m across](docs/level0-sample.png)

## Tests

```bash
cargo test --workspace
```

The golden test compares the generator with
`crates/backrooms-core/tests/golden/v1.txt`. While generator v1 is still a
draft, an intended change to the world is accepted with:

```bash
BACKROOMS_BLESS=1 cargo test -p backrooms-core --test golden
```

Once v1 ships, its output is frozen and that command is refused.

# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`egg-stitch` is a Rust project that uses the [egg](https://github.com/egraphs-good/egg) library for e-graph based program synthesis. It implements pattern matching and search state management over e-graphs.

The project uses a **patched version of egg** from a fork. Depending on which line of Cargo.toml is uncommented, it will either be from a fork on github or it'll be a local clone located at `../egg`

## Understanding egg

See the tutorials in `../egg/src/tutorials/`


## Style

- Add docstrings to all functions you write
- Keep your code concise. Whenever you finish a request go back and think about whether you could have done it simpler and more concise.


### Code Quality
```bash
# Check code for errors without building
cargo check

# Format code
cargo fmt

# Lint with clippy
cargo clippy
```

### WASM Build
```bash
# Prerequisites (one-time):
#   cargo install wasm-pack
#   rustup target add wasm32-unknown-unknown

# Build the WASM package (outputs to pkg/)
make wasm

# Or directly:
wasm-pack build --target web --features wasm

# Run the dev server for the interactive UI
make server
# Then open http://localhost:8066/viz/interactive.html
```

## Testing

### Snapshot (bless/check) suite

The main end-to-end suite is **data-driven**: every bless/check case lives as a
`[[case]]` in `tests/snapshots.toml` (its header documents the fields), and
`tests/snapshots.rs` turns each into one libtest-mimic trial — there is no
handwritten Rust test per fixture. The shared run/strip/bless-or-check machinery
is in `tests/common/mod.rs`. Each case owns one fixture under
`data/expected_outputs/`; a `coverage` trial enforces that the manifest and the
fixture tree correspond exactly (an orphan fixture or a case with no fixture
fails).

```bash
# check all snapshots (or a subset by trial-name substring)
cargo test --release --test snapshots
cargo test --release --test snapshots -- cogsci
# re-bless after a legitimate behavior change
BLESS=1 cargo test --release --test snapshots
```

To add a snapshot: append a `[[case]]` and run `BLESS=1`. Recipes are keyed by
`kind` (stitch / cogsci / dreamcoder / epfl) in `tests/common/mod.rs`.

- `tests/snapshot_asserts.rs` — feature-invariant assertions that ride on top of
  the snapshots (e.g. "a metavar is reused 3×", the EPFL `.aig` corpus
  regenerates). They reuse the shared runner; they do not own fixtures.
- `scripts/check_all_outputs.py` reads the **same** `tests/snapshots.toml` for
  each fixture's equivalence-oracle spec (the `oracle` field: `beta` / `circuit`
  / `{rules=…}` / `{skip=…}`) — no hand-maintained path lists.

Keep the manifest and CLAUDE.md in sync when the suite changes.

## Development Notes

- The project uses Rust edition 2024
- When working with e-graphs, remember that multiple equivalent expressions are stored in the same e-class
- **Keep this file up to date**: whenever code changes disagree with anything written here, update CLAUDE.md as part of the same change.

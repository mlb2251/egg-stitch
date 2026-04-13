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

## Development Notes

- The project uses Rust edition 2024
- When working with e-graphs, remember that multiple equivalent expressions are stored in the same e-class
- **Keep this file up to date**: whenever code changes disagree with anything written here, update CLAUDE.md as part of the same change.

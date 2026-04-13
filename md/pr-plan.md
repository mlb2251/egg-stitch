# PR Plan: dev-maddy -> dev-maddy-2

~30 commits spanning WASM support, interactive UI, search refactoring, benchmarking, and cleanup. Replay system has been removed (`3a67ce4`).

## Change Groups

### 1. WASM Infrastructure
**Commits:** `a3155b5` wasmification, `c50e031` wasm options (partial), `2f15135` (Cargo.toml part)

Initial WASM support: `wasm-pack` build pipeline, `#[wasm_bindgen]` `Engine` struct in `lib.rs`, `.cargo/config.toml`, `Cargo.toml` feature flags + deps (`wasm-bindgen`, `serde-wasm-bindgen`, `web-sys`), `Makefile` targets, `server.py`.

**Files:** `.cargo/config.toml`, `Cargo.toml`, `Cargo.lock`, `Makefile`, `src/lib.rs` (WASM API surface), `src/pattern.rs`, `src/io.rs`, `src/search.rs`, `src/smc.rs`, `src/best_first.rs` (conditional compilation), `viz/server.py`

---

### 2. Interactive Web UI
**Commits:** `a3155b5` (viz parts), `12185cd` (viz parts), `55d8c95` (viz/interactive.js), `c50e031` (viz parts), `2f15135` (viz parts), `1ce0ea3` (viz parts), `3c69f21` fix bug (viz parts), `beb9b93` rekeying (viz part), `f1d3ca1` many little changes (viz parts), `1279c32` batch runs to front, `7e9fb17` cleaner, `7b36415` we save things from wasm land, `1d72878` config better, `2bba864` meep, `f79a9b2` tweaks

The interactive explorer UI built on top of WASM:
- `interactive.html` + `interactive.js`: core tree explorer with expand/collapse, config panel
- `wasm-api.js`: centralized WASM/engine calls
- `shared.js`: domain loading, file saving, shared utilities
- `batch.js`: batch run management extracted from interactive.js
- `tree-render.js`: shared tree rendering logic
- Config UI for search parameters (temperature, particles, budget, etc.)
- Save/load results to server

**Files:** `viz/interactive.html`, `viz/interactive.js`, `viz/wasm-api.js`, `viz/shared.js`, `viz/batch.js`, `viz/tree-render.js`, `viz/index.html`, `viz/style.css`, `viz/server.py`, `viz/analysis.js`

---

### 3. InteractiveSearch Refactor
**Commits:** `55d8c95` cut out js-side heap, `beb9b93` rekeying the heap, `3c69f21` fix bug, `c50e031` (best_first parts), `1ce0ea3` (best_first parts), `12185cd` (best_first parts), `4c07f8d` determinism, `f1d3ca1` many little changes (best_first parts)

Major refactor of `best_first.rs`: introduced `InteractiveSearch` struct that owns the search tree, heap, and seen-set. Replaced the standalone `best_first()` function with a stateful object that supports step-by-step expansion (needed for WASM interactivity). Moved from `BinaryHeap` to `BTreeSet` for deterministic ordering. Added canonical dedup, heap rekeying on cost updates.

**Files:** `src/best_first.rs`, `src/lib.rs` (InteractiveSearch integration)

---

### 4. SMC Refactor
**Commits:** `409ba9b` smc but still kinda broken what do, `ade8146` (smc part)

Rewrote SMC to use `InteractiveSearch` as its underlying tree instead of managing its own `Vec<SearchState>`. Particles are now node IDs in the shared search tree. Dramatically simplified `smc.rs` (~160 lines removed).

**Files:** `src/smc.rs`, `src/main.rs` (CLI wiring)

---

### 5. Code Cleanup & Refactor
**Commits:** `add20ad` refactor, `f1d3ca1` many little changes, `ade8146` bunch of code review fixes, `3a67ce4` remove replay

Large cleanup pass: deleted `debug_log.rs`, `logging.rs`, `replay.rs`, `replay.js`, old `viz/tree.html` + `viz/tree.js`. Simplified `io.rs` and `search.rs`. Moved search config from CLI `Args` to `SharedSearchData` + config structs.

**Files:** `src/debug_log.rs` (deleted), `src/logging.rs` (deleted), `src/replay.rs` (deleted), `viz/replay.js` (deleted), `src/io.rs`, `src/search.rs`, `src/main.rs`, `src/results.rs`, `viz/tree.html` (deleted), `viz/tree.js` (deleted), `viz/debug.html`, `viz/debug.js`

---

### 6. Cost Fix
**Commits:** `2ff3977` cost fix

Fixed cost computation in `cost.rs` (21 lines added, 4 removed). Standalone bug fix.

**Files:** `src/cost.rs`

---

### 7. Determinism
**Commits:** `4c07f8d` determinism

Made search deterministic: switched heap from `BinaryHeap` to `BTreeSet`, fixed ordering in `matching.rs`.

**Files:** `src/best_first.rs`, `src/matching.rs`, `Cargo.toml`

---

### 8. Benchmarking & Experiments
**Commits:** `a9ff2ca` bfs and dfs, `cc3d42f` arity=2, `bcc8ac1` ok good, `c574ef2` samply, `4fc3606` stitch eval, `4f34905` babble benchmark, `7a210f0` nice babble printing

Experiment infrastructure in `run.py`: BFS/DFS search modes, stitch/babble baseline comparisons, samply profiling support, `expts/__init__.py` helpers.

**Files:** `run.py`, `expts/__init__.py`, `src/best_first.rs` (BFS/DFS modes), `src/main.rs` (CLI flags)

---

### 9. Tree Viz (pre-WASM)
**Commits:** `ed6aa06` better viz, `e54edf9` add children to viz, `ce5cd74` show expansion order clearly

Early tree visualization improvements to the old `tree.html`/`tree.js` (later superseded by the interactive explorer). Can be dropped or folded into group 2.

**Files:** `viz/tree.html`, `viz/tree.js`, `src/best_first.rs`, `src/debug_log.rs`

---

## Dependency Graph

```
                    ┌─────────────┐
                    │  6. Cost Fix │
                    └─────────────┘
                       (independent)

                    ┌──────────────┐
                    │ 7. Determinism│
                    └──────┬───────┘
                           │
                           v
               ┌───────────────────────┐
               │ 3. InteractiveSearch  │
               │       Refactor        │
               └───────┬───────────────┘
                       │
              ┌────────┴────────┐
              v                 v
   ┌──────────────────┐  ┌──────────────────┐
   │  4. SMC Refactor │  │  1. WASM Infra   │
   └──────────────────┘  └────────┬─────────┘
                                  │
                    ┌─────────────┼──────────────┐
                    v             v               v
        ┌────────────────┐ ┌───────────┐  ┌──────────────┐
        │ 2. Interactive │ │8. Benchm. │  │ 9. Tree Viz  │
        │    Web UI      │ │& Expts    │  │  (pre-WASM)  │
        └────────────────┘ └───────────┘  └──────────────┘
                                           (superseded by 2)
              │
              v
     ┌────────────────┐
     │ 5. Code Cleanup│
     └────────────────┘
```

### Dependency explanations

- **7 -> 3**: Determinism (BTreeSet) is a prerequisite for the InteractiveSearch refactor which relies on deterministic ordering.
- **3 -> 4**: SMC refactor depends on InteractiveSearch existing as the shared tree structure.
- **3 -> 1**: WASM infra wraps InteractiveSearch with `#[wasm_bindgen]` API.
- **1 -> 2**: Interactive UI loads the WASM module and calls its API.
- **1 -> 8**: Some experiment scripts depend on CLI flags added alongside WASM work. But many run.py changes are independent.
- **2 -> 5**: Cleanup deletes old code replaced by the interactive UI and wasm-api.js. Also includes replay removal.
- **9**: Early tree viz is superseded by group 2. Can be dropped or folded in.
- **6**: Cost fix is fully independent, can be PRed at any time.

### Suggested PR ordering

1. **Cost Fix** (standalone, no deps)
2. **Determinism** (small, foundational)
3. **InteractiveSearch Refactor** (core change, depends on 2)
4. **SMC Refactor** (depends on 3)
5. **WASM Infrastructure** (depends on 3)
6. **Interactive Web UI** (depends on 5)
7. **Benchmarking & Experiments** (mostly independent, some deps on CLI flags from 3/5)
8. **Code Cleanup & Refactor** (depends on everything, final pass — includes replay removal)

Group 9 (pre-WASM tree viz) can be dropped or folded into group 6 since it was superseded.

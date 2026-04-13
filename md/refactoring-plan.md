# Refactoring Plan

## Summary

The codebase is well-structured overall. The main issues are: dead code from a prior SMC refactor, duplication in `main.rs` and across JS files, and some files that are doing too many things. Replay functionality (planned for deprecation) is entangled with core search logic and should be isolated.

---

## 1. Dead Code Removal (Rust)

These are all unreachable after the SMC refactor to use `InteractiveSearch`:

| File | Dead code | Notes |
|------|-----------|-------|
| `logging.rs` | **entire file** | `apply_follow_constraint` and `print_top_particles` reference old per-particle `SearchState` vectors that no longer exist. Zero callers. |
| `debug_log.rs` | `DebugLog`, `StepLog`, `ParticleLog`, `build_particle_logs()`, `log_debug_step()` | These supported the old SMC debug trace. Zero callers. Keep `ReplayLog`/`ReplayConfig`/`ReplayStep` (used by replay). |
| `io.rs` | `print_programs()`, `print_expr()`, `from_file()` | All have zero callers. `print_programs` is `#[allow(dead_code)]`. |
| `main.rs` | `--debug_log` flag | `debug_log_json` is always `None` in both match arms. The flag parses but does nothing. |

**Action**: Delete `logging.rs` entirely. Remove dead items from `debug_log.rs` and `io.rs`. Remove `--debug_log` arg from `main.rs`.

---

## 2. Simplify `main.rs`

The two match arms (`Smc` / `BestFirst`) duplicate ~80% of their logic:
- Both create `InteractiveSearch`
- Both print "original size"
- Both extract `best`, `best_found_at`, `num_expansions`, `replay_json`
- Both serialize `replay_log`

The `Option<SharedSearchData>` + `.take()` dance is also unnecessary since both arms consume `shared`.

**Action**: Extract the common pre/post logic. The match should only contain the divergent part (which search to run). Sketch:

```rust
let mut search = InteractiveSearch::new(shared, root, original_size, priority, max_arity);
println!("original size: {}", original_size);

match args.search {
    Smc => { smc(&mut search, &smc_config); }
    BestFirst => { /* step loop */ }
}

// Common post-search: extract best, print results, save replay, build RunResult
```

---

## 3. Isolate Replay Functionality

Replay is planned for deprecation. Currently it's entangled with `InteractiveSearch` (in `best_first.rs`) and `debug_log.rs`.

**Action**: Create `src/replay.rs` containing:
- `ReplayLog`, `ReplayConfig`, `ReplayStep` (moved from `debug_log.rs`)
- `replay_from_json()` and `replay()` methods (moved from `InteractiveSearch` in `best_first.rs` -- implemented as free functions taking `&mut InteractiveSearch`)
- The CLI replay-mode block from `main.rs` (lines 121-137)

After this, `debug_log.rs` can be deleted entirely (its remaining replay structs move to `replay.rs`, and all the debug trace structs were dead code).

`best_first.rs` keeps `replay_log()` (it generates the log from expansion history, which is core functionality) but `replay_from_json()`/`replay()` move out.

The WASM `replay_from_json` wrapper in `lib.rs` just calls through, so it updates trivially.

---

## 4. JS: Extract WASM interaction layer

Currently, WASM engine calls (`new Engine(...)`, `step()`, `run_smc()`, `results_json()`, etc.) are scattered across `interactive.js` and `batch.js`.

**Action**: Create `viz/wasm-api.js` containing:
- `loadWasm()` -- module import + init
- `createEngine(programsText, rulesText, configJson)` -- wraps `new wasm.Engine(...)`
- `runSearch(engine, searchType, params)` -- dispatches to `run_smc` or `step_n`
- Re-exports of engine method calls that the UI needs

This makes the WASM boundary explicit and testable. `interactive.js` and `batch.js` import from `wasm-api.js` instead of touching `wasm` directly.

---

## 5. JS: Extract replay logic from `interactive.js`

`interactive.js` is 729 lines. ~180 lines (350-552) are replay-specific.

**Action**: Create `viz/replay.js` containing:
- `scanReplays()`, `parseDirectoryListing()`, `applyReplayConfig()`
- `replayOneStep()`, `runReplayFromUrl()`, `runReplayFromJson()`
- Replay state (`replayJsonText`, `replaySteps`, `replayIdx`, `replayExpectedCost`)
- Replay event handlers (`selReplay`, `btnReplay`, `btnReplayAll`)

This also prepares for eventual deprecation -- the whole file can be removed.

---

## 6. JS: Deduplicate shared utilities

| Function | Copies | Where |
|----------|--------|-------|
| `esc()` / `escapeHtml()` | 4 | `interactive.js`, `debug.js`, `analysis.js`, `tree-render.js` |
| `parseDirectoryListing()` / `extractLinks()` | 2 | `interactive.js`, `analysis.js` |

**Action**: 
- Use `escapeHtml` from `tree-render.js` everywhere (it's already exported). Remove the local `esc()` copies.
- Move `parseDirectoryListing` to `shared.js` and import in both `interactive.js` and `analysis.js`.

---

## 7. Minor Rust improvements

- **`results.rs`**: The `debug_log_file` field references dead functionality. Remove it.
- **`best_first.rs` line 120**: `expand_one` takes 10 positional args. After removing replay methods, consider whether making it a method on `InteractiveSearch` (accessing fields directly) is cleaner.
- **`search.rs`**: `expand_random` creates a fresh `Extractor` on every call (line 53), only used for verbose printing. Gate the `Extractor::new` behind the `verbose` flag.

---

## Execution Order

1. Dead code removal (safe, no behavior change)
2. Create `replay.rs`, move replay code out of `best_first.rs` and `debug_log.rs`
3. Delete `debug_log.rs` and `logging.rs`
4. Simplify `main.rs`
5. JS: create `wasm-api.js`, `replay.js`; deduplicate utilities
6. Minor improvements

Each step is independently shippable and testable with `cargo check && cargo test && make wasm`.

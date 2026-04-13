# Per-File Code Review

## Rust Files

### `lib.rs` (243 lines) -- GOOD

Module declarations + WASM API wrapper. Clean separation. No issues.

The `EngineConfig` and `SearchResults` structs are WASM-only and correctly feature-gated.

**No changes needed.**

---

### `main.rs` (293 lines) -- NEEDS REFACTOR

**Issues:**
1. **Duplicated match arms (lines 145-209)**: SMC and BestFirst arms are ~80% identical. Both create `InteractiveSearch`, print status, extract `best`/`replay_json`, etc. Should extract common pre/post logic.
2. **`Option<SharedSearchData>` dance (line 141)**: Wrapping `shared` in `Option` just to `.take()` it is unnecessary complexity. Both arms consume it, so just move the `InteractiveSearch::new()` call before the match.
3. **`--debug_log` flag (line 99-100)**: Parses but has no effect. `debug_log_json` is hardcoded to `None` in both arms. Dead feature.
4. **5-tuple return type (line 145)**: `#[allow(clippy::type_complexity)]` is a code smell. After simplification this goes away.
5. **Replay block (lines 121-137)**: Should live in `replay.rs`.

**No bugs found.**

---

### `best_first.rs` (461 lines) -- GOOD, minor cleanup

Well-structured. `InteractiveSearch` is the central search driver, used by both CLI and WASM.

**Issues:**
1. **`expand_one` has 10 positional args (line 120)**: Could become a method on `InteractiveSearch` since it accesses every field. The current free-function form was probably needed when there were multiple search structs, but now there's only one.
2. **Replay methods (lines 264-314)**: `replay_from_json()` and `replay()` should move to `replay.rs` for isolation.
3. **`replay_log()` (lines 430-451)**: This *generates* replay logs from expansion history. It's core functionality (not replay-specific) and should stay.

**No bugs found.** The search logic is correct and well-commented.

---

### `search.rs` (264 lines) -- GOOD, one perf note

Core search state and expansion logic. Clean design.

**Issues:**
1. **Wasteful `Extractor::new` in `expand_random` (line 53)**: Creates a full `Extractor` on every call, but only uses it for verbose printing. Should be gated:
   ```rust
   let extractor = if verbose { Some(egg::Extractor::new(...)) } else { None };
   ```
2. **`_shared` unused parameter in `reuse` (line 109)**: The parameter is accepted but prefixed with `_`. Either remove it or document why it's kept for API consistency.

**No bugs found.**

---

### `pattern.rs` (262 lines) -- EXCELLENT

Canonical-form invariant is clever and well-documented. Thorough test suite (165 lines of tests) validates the invariant across expand/reuse/nesting scenarios.

**No changes needed.** This is the best-documented file in the codebase.

---

### `cost.rs` (112 lines) -- GOOD

Worklist-based cost computation. Handles equality saturation correctly (eclasses may need revisiting).

**Issues:**
1. **`build_rewritten_egraph` is `pub(crate)` (line 94)**: Only called from within `cost.rs` itself and could be private. (Minor.)

**No bugs found.** The `check_slow` validation path is a nice safety net.

---

### `smc.rs` (105 lines) -- GOOD

Clean SMC implementation. Proper numerical handling with log-space weights.

**No changes needed.**

---

### `debug_log.rs` (98 lines) -- MOSTLY DEAD CODE

**Keep:** `ReplayLog`, `ReplayConfig`, `ReplayStep` (lines 1-35). These are actively used.

**Dead code (remove):**
- `DebugLog` (line 38-44)
- `StepLog` (line 47-58)
- `ParticleLog` (line 61-68)
- `build_particle_logs()` (line 71-83)
- `log_debug_step()` (line 86-98)

All of the above reference the old per-particle `SearchState` model. Zero callers anywhere in the codebase.

**Action:** Move the live structs to `replay.rs`, delete this file.

---

### `logging.rs` (79 lines) -- ENTIRELY DEAD CODE

Both functions (`apply_follow_constraint`, `print_top_particles`) take `&[SearchState]` parameters matching the old SMC model where particles were `SearchState` objects. After the refactor to `InteractiveSearch` (where particles are node IDs), nothing calls these.

**Action:** Delete entire file.

---

### `io.rs` (133 lines) -- GOOD, some dead code

**Dead code:**
- `print_programs()` (line 65-78): `#[allow(dead_code)]`, zero callers
- `print_expr()` (line 81-93): Only called by `print_programs`
- `from_file()` (line 96-105): Zero callers (CLI reads files directly, then calls `parse`)

**Live code is clean.** `build_egraph` / `load_egraph` / `load_egraph_from_strings` / `parse` are all well-structured.

**Action:** Remove the three dead items (~38 lines).

---

### `lang.rs` (78 lines) -- GOOD

Minimal language definition + analysis. Nothing to change.

---

### `revexpr.rs` (72 lines) -- GOOD

Clever design choice (reverse node ordering for easy partial pattern expansion). Well-implemented with proper `Index`/`IndexMut`/`From` impls.

**One nit:** The comment on line 39 ("somewhat silly clone now but it's okay") is fine -- `Display` isn't hot-path.

---

### `results.rs` (36 lines) -- MINOR CLEANUP

**Issue:** `debug_log_file` field (line 32) references dead debug logging functionality. Should be removed along with the debug log cleanup.

---

### `matching.rs` (31 lines) -- GOOD

Minimal. `identity_matches` sorts for cross-platform determinism (documented in comment). No changes.

---

### `follow.rs` (33 lines) -- GOOD

Recursive follow-constraint checking. Correct logic.

**Possible concern:** Stack depth on very deep patterns. Unlikely in practice since patterns are typically shallow (<20 depth). Not worth changing.

---

### `math.rs` (14 lines) -- GOOD

Standard `logaddexp` with proper `NEG_INFINITY` handling. No changes.

---

## JavaScript Files

### `interactive.js` (729 lines) -- NEEDS SPLITTING

This is the largest JS file and does too many things.

**Issues:**
1. **Replay logic (lines 350-552, ~200 lines)**: Should be in `replay.js`. Includes state (`replayJsonText`, `replaySteps`, `replayIdx`), scanning (`scanReplays`, `parseDirectoryListing`), and execution (`replayOneStep`, `runReplayFromJson`, `runReplayFromUrl`).
2. **WASM interaction scattered**: `loadWasm()`, `new wasm.Engine(...)`, `engine.run_smc(...)`, `engine.step_n(...)` should be in `wasm-api.js`.
3. **`runSingleSearch` (lines 189-207)**: Duplicates the SMC/BestFirst dispatch logic that also appears in `btnRun` handler (lines 269-278) and `batch.js`. Should be in `wasm-api.js`.
4. **Local `esc()` function (line 727-729)**: Duplicate of `escapeHtml` in `tree-render.js`. Import instead.

**No bugs found.**

---

### `shared.js` (85 lines) -- GOOD

Clean utility module. Well-structured exports.

**Minor issue:** `RULES_DIR` points to `/babble/harness/data/benchmark-dsrs` which is an absolute path to another project. This is fine for local dev but worth noting.

**Action:** Add `parseDirectoryListing` here (dedup from `interactive.js` and `analysis.js`).

---

### `tree-render.js` (228 lines) -- GOOD

Clean rendering module with proper exports. `escapeHtml` is the canonical version that others should import.

**No changes needed.**

---

### `batch.js` (80 lines) -- GOOD

Simple batch runner. Clean.

**Minor**: Could import engine creation from `wasm-api.js` instead of doing `new wasm.Engine(...)` directly.

---

### `debug.js` (251 lines) -- GOOD

Step-by-step SMC debug viewer. Well-organized.

**Issue:** Local `esc()` function (line 251). Should import `escapeHtml` from `tree-render.js`.

**Note:** This file uses the `DebugLog` format which is dead on the Rust side. If no existing debug JSON files need viewing, this entire file + `debug.html` may be dead too. Worth confirming with user.

---

### `analysis.js` (262 lines) -- GOOD

Results table viewer.

**Issues:**
1. **`extractLinks()` (line 15-25)**: Duplicate of `parseDirectoryListing` in `interactive.js`. Should be in `shared.js`.
2. **Local `esc()` (line 260)**: Should import from `tree-render.js`.

---

### HTML Files

**`interactive.html`**, **`index.html`**, **`debug.html`**: All clean. CSS is inline but appropriate for a dev tool. No issues.

---

## Possible Bugs

1. **`--debug_log` does nothing (main.rs:100)**: The flag is accepted by the CLI parser but never wired up. `debug_log_json` is always `None`. This is not a crash bug but a silent misconfiguration -- a user passing `--debug_log` would expect output and get none. **Fix:** Remove the flag entirely (debug logging is dead code).

2. **`debug.js` may be broken**: It loads `DebugLog`-format JSON files, but the Rust code no longer generates them. If users navigate to `debug.html?file=...` with a file generated by an older version, it works. But new runs never produce these files. The "debug" links in `analysis.js` (line 116-118) check for `r.debug_log_file` which is always null for new runs. **Not a crash bug**, but the feature is vestigial.

3. **`expand_random` creates `Extractor` unconditionally (search.rs:53)**: `Extractor::new` traverses the entire egraph. This runs on every SMC particle expansion even when `verbose=false`. **Perf bug**: should be gated behind `verbose`.

# PR plan: shrinking `dominance-rebased` into `main`

Companion to `REBASE_AUDIT.md`. The audit verifies the rebase didn't lose anything; this file tracks the plan to *split* `dominance-rebased` into focused PRs against `main`.

## Model

`dominance-rebased` is the immutable source-of-truth tree. PRs against `main` are formed by *selectively pulling content* from `dominance-rebased`. The branch itself doesn't move — except for occasional **cleanup commits on dominance-rebased** when the diff needs to be made PR-shaped (dead code removal, stale comment fixes, debug prints that snuck in, etc).

### Running scoreboard

```sh
git diff --stat main dominance-rebased -- ':(exclude)rebase-tooling'
git diff --name-status main dominance-rebased -- ':(exclude)rebase-tooling'
```

Every landed PR shrinks this diff without anyone touching `dominance-rebased`.

### Per-PR mechanical recipe

```sh
git switch main && git pull
git switch -c pr/<slice-name>

# Whole files:
git checkout dominance-rebased -- src/foo.rs src/bar.rs

# Partial files (interactive hunk picking):
git checkout -p dominance-rebased -- src/baz.rs

git diff --staged              # review
cargo check && cargo test      # local sanity
git commit -m "..."
gh pr create
```

After the PR lands and `main` advances, the running-scoreboard diff naturally shrinks for that slice.

### Ordering principle (from `REBASE_PROCEDURE.md` lines 8-10)

1. **Refactors first** (pure structural changes, no behavior).
2. **Additive features second** (new files, new flags that are off by default).
3. **Behavior changes last** (the actual dominance pruning semantics).

## Current diff (snapshot 2026-04-30)

```
 expts/stackpath.py          |  91 +++++
 expts/stitch.py             |   8 +-
 expts/table1.py             |  75 ++--
 expts/table2.py             |  73 ++--
 run.py                      | 115 +++++-
 src/best_first.rs           | 119 +++++-
 src/cost.rs                 | 366 +++++++++++++----
 src/lib.rs                  |  34 +-
 src/main.rs                 |   2 +
 src/pattern.rs              |  34 ++
 src/results.rs              |   3 +
 src/revexpr.rs              |   5 +
 src/rewrite.rs              |  24 ++
 src/search.rs               | 160 +++++---
 src/smc.rs                  |   7 +-
 tests/stitch_compat_test.rs |  12 +-
 viz/server.py               | 143 ++++++-
 viz/stackpath.html          | 281 +++++++++++++
 viz/stackpath.js            | 961 ++++++++++++++++++++++++++++++++++++++++++++
 23 files changed, 2293 insertions(+), 239 deletions(-)
```

**Note (2026-04-30):** A cleanup commit on `dominance-rebased` collapsed the
`src/cost/` directory back into a single `src/cost.rs`, so the diff is now a
clean superset of main's `cost.rs` rather than a deletion + new module tree.
This dissolves the old PR-A/PR-B split — see the candidate list below.

Refresh with `git diff --stat main dominance-rebased -- ':(exclude)rebase-tooling'` whenever the table below moves.

## Candidate slice list

Roughly grouped by ordering principle. Sizes are rough — actual slice boundaries get nailed down at PR time.

### Refactor (pure structural)

| Slice | Files | Notes |
|---|---|---|
| **PR-cost: cost.rs additions (single file)** | `src/cost.rs` (+~210 net), `src/rewrite.rs` (+24) | Now that `dominance-rebased`'s cost machinery lives in a single `cost.rs`, this PR is just: take dominance-rebased's `src/cost.rs` verbatim. It's a clean superset of main's `cost.rs` — adds `CostCache`/`CostScratch`/`RunnerScratch`, the `StitchAnalysis` trait + `StitchAnalysisRunner`, `CostOnlyExtractor`, `LowerBoundAnalysis`, `RewriteAnalysis`/`RewriteScratch`, plus `min_enode_size`. Some of this is dependency-only for later behavior PRs (e.g. `LowerBoundAnalysis` is only used by `--no-opt-lower-bound`), but it all sits behind explicit call sites so shipping it together is safe and additive. May still be worth splitting if review asks; default is one PR. **Likely first PR.** |

### Additive features

| Slice | Files | Notes |
|---|---|---|
| **stackpath viz** | `viz/stackpath.{html,js}` (+1242), `viz/server.py` (+~140), `expts/stackpath.py` (+91), `run.py` (parts), `.gitignore` | Wholly additive: a new viewer panel + python helpers. Touches `viz/server.py` for routing, but everything else is new files. |
| **`run.py` shortcut suite** | `run.py` (+115) | The `quick_eval` / `quick_full_enum` shortcuts used during the rebase regression checks. Self-contained dev-tool. |
| **expts harness updates** | `expts/{__init__,babble,egg_stitch,stitch,table1,table2}.py` | Combined upstream's `num_abstractions`/`rebuild_egraph` plumbing with dominance's `max_arity`/`num_runs`/`domains`/`stackpathpush` plumbing. May want to split: pure-additive bits (`stackpathpush`, `subgroup`) vs. signature changes that touch upstream code. |
| **CLI flags off by default** | `src/lib.rs` (parts), `src/main.rs` (parts) | `--no-opt-lower-bound`, `--no-seen`, `--no-opt-dominance`, `--rebuild-egraph` — additive flags whose default is the existing behavior, so they're safe even before the behavior PRs land. |
| **`load_egraph` timing line** | `src/main.rs` (1 line) | Trivial. Could fold into another PR or stand alone. |

### Behavior changes (dominance proper)

| Slice | Files | Notes |
|---|---|---|
| **`SeenTracker` plumbing** | `src/search.rs` (parts), `src/best_first.rs` (parts), `src/lib.rs` (parts) | The seen-set pruning machinery. Gated by `--no-seen` flag. |
| **`compute_lower_bound` pruning** | `src/cost/lower_bound_cost.rs` (additive in the cost-split PR), call sites in `src/best_first.rs` and `src/search.rs` | The lower-bound cost-pruning pass. Gated by `--no-opt-lower-bound`. |
| **`opt_dominance_reuse` (dominance pruning, narrowed to reuse branch)** | `src/search.rs` (parts), `src/best_first.rs` (parts), `src/lib.rs` (parts) | The actual dominance-equivalence pruning. Gated by `--no-opt-dominance`. |
| **`num_substs` threading** | `src/search.rs` (parts) | Diagnostic counter on `SearchState`. May be folded into one of the above. |
| **`best_history` on `AbstractionResult`** | `src/lib.rs`, `src/results.rs`, `src/best_first.rs`, `expts/egg_stitch.py`, `tests/stitch_compat_test.rs` (the `strip_library_field` extension) | Per-step best-cost trace. Touches python and the stitch-compat test infrastructure. |
| **`rebuild_egraph` knob** | `src/lib.rs`, call sites | Separate from the above. Gated by `--rebuild-egraph`. |
| **`pattern.rs` `Eq`/`Hash` on `Pattern<F, O>`** | `src/pattern.rs` (+34) | Required by `SeenTracker`'s set membership. May ship with the SeenTracker PR. |

### Test infrastructure changes

| Slice | Files | Notes |
|---|---|---|
| **`stitch_compat_test` `strip_library_field` extension** | `tests/stitch_compat_test.rs` (+12) | Added to make best-first/smc agree on shape after `best_history`. Ships with `best_history` PR. |

## Queued / in-flight / landed

| Slice | Status | PR | Notes |
|---|---|---|---|
| PR-cost: cost.rs additions (single file) | queued | — | Take dominance-rebased's `src/cost.rs` verbatim; superset of main's. |
| (others) | not yet picked | — | Will be slotted in as we go. |

**Cleanup commits on `dominance-rebased`:**

| Commit | Date | Notes |
|---|---|---|
| `a9c12b8` collapse cost/ module back into single src/cost.rs | 2026-04-30 | Reverted the `cost/` module split so the future cost PR is additive against main rather than a delete-and-rebuild. |

## Open decisions before we start

1. **Audit-vs-PR ordering.** Audit step 1 + step 5 first (the bookends) before any PR? Audit per-step as we slice? Audit fully after?
2. **Which slice ships first.** `cost.rs` → `cost/` split is the obvious candidate (largest diff-shrink, pure refactor, no behavior risk). Confirm before slicing.
3. **Cleanup-commits-on-dominance-rebased policy.** When we do these, do we tag (`pr-prep-N-pre-<slice>`) for recovery? Or just rely on reflog?

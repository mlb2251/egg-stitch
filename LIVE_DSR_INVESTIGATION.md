# Live-DSR investigation — summary

Question driving the whole thread: **does applying rewrite rules (DSRs) *live*
during abstraction search beat applying them only *at-start* (one-shot
canonicalize, then rule-free search) — and can it beat plain no-rules
abstraction-finding?** Three modes compared throughout:

- **baseline** — no rules, plain best-first/SMC abstraction.
- **live** — rules kept active in the e-graph during search (re-saturate between abstractions).
- **at-start** (`--only-use-dsrs-at-start`) — saturate once, extract the size-minimal
  min-term per program, rebuild a rule-free e-graph, then search.

Lower cost = better compression.

## The property that matters (and the cheap predictor for it)
Live can only beat at-start when the corpus has **many distinct expressions that
share a common sub-structure written in *non-canonical* forms** — so live's
unification exposes sharing a frozen at-start canonicalization misses. If the
shared idioms are already canonical (single-generator corpora, or fill-down
duplicates), at-start captures them just as well and live wins nothing.

**AC-census** (the predictor, pure-Python, cheap): reduce each program to a
structural sketch (leaves blanked), count distinct sketches, then count again
after AC-normalization (flatten+sort commutative chains). The fraction merged
estimates the live-vs-at-start opportunity. Validated as predictive below.

## Corpora tried (all real datasets)
| corpus | AC-merge | live vs at-start | live vs baseline |
|--------|----------|------------------|------------------|
| physics-unfolded (lambda-calc) | ~0% | ≈ (assoc helps both ~1%) | live ~= baseline |
| lample-deriv (Facebook eqns)   | ~0% | live ~1% better | distrib-live slightly > baseline |
| FPBench (real FP kernels)      | ~0% (these rules) | live ~3% better | live ~3% > baseline (AC coverage) |
| Enron spreadsheets (913k formulas) | **1 of 112** | live≈at-start (1.6%) | rules HURT vs baseline |
| **EPFL multiplier cones**      | **63% (1721→642)** | **live ≫ at-start** | see below |

Negative finding across physics/lample/FPBench/Enron: the shared structure is
canonical (or fill-down duplication), so live's edge over at-start is ~0–3% and
rules rarely beat the no-rules baseline. Multi-source hunt (spreadsheets) failed
the same way — measured, not assumed: Enron's 913k formulas, AC merges 1/112.

## The multiplier (the corpus that has the property)
`data/domains/mult/` — EPFL `multiplier.aig` (And-Inverter graph), decomposed into
depth-4 fan-in cones per gate → 800 op-children boolean programs (named signal
leaves). AC-census: **1721 distinct cone structures → 642 under AND-AC + ¬¬ (63%
mergeable)** — the only corpus with real non-canonical structure.

Config: SMC, op-children, max-arity 4, 5000 particles / 100 steps, temp 1000,
seed 1 (deterministic). Final cost over 10 abstractions:

| rules | baseline | live | at-start |
|-------|----------|------|----------|
| AC (`and_ac`)                      | 13599 | 14364 | 15526 |
| De Morgan (`and_or_demorgan`)      | 13599 | **12980** | 16212 |
| + factoring (`and_or_demorgan_factor`) | 13599 | **12401** | 15463 |

### Mechanisms (all measured, per-abstraction trajectories + library diffs)
- **AC-only fragment** (`and_comm`, `and_assoc`, `notnot`): live beats at-start
  (AC unifies operand orderings → more match sites) but **loses to baseline**.
  Live is ahead for the first 4 abstractions then **front-loads-then-starves**:
  it founds on a fat 3-input AOI cell that grabs the most sites early, leaving a
  worse residual → crosses over at abstraction 4. at-start is worst because
  size-preserving AC canonicalization picks *inconsistent per-program orderings*,
  **breaking cross-program alignment up front** (its very first abstraction saves
  less: 8375 vs baseline 9131). Confirmed live not beating baseline is *not* a
  cap/blowup artifact: holds under best-first (capped), SMC (uncapped), 8× more
  particles/steps, and temp 1000 — live plateaus at 14364.
  - Reachability check (`--follow`): baseline's NOR cell **is** reachable by the
    live search and is *more* valuable there in isolation (22591 vs 23131) — so
    AC's loss is a greedy phase-ordering divergence, not inaccessibility.

- **De Morgan** (add explicit `or` + AND↔OR bridges): captures real *semantic*
  equivalence (a cell's AND-tree vs OR-tree forms), minimizes cones to NNF
  (after-rules 32262→22659). **Live (12980) beats baseline (13599) wire-to-wire**
  — ahead at every abstraction, no crossover — the first configuration to do so.
  It founds on the natural product-of-sums XNOR cell directly. at-start does
  *worst* (16212): more alternative forms → more inconsistent per-program picks.

- **+ factoring** (size-reducing distributivity `(a|x)(a|y)⇒a|(x&y)` + absorption
  + idempotence): collapses the redundant doubled-literal POS cells (no blow-up,
  since only the *factoring* not *expanding* direction is used). after-rules
  22659→21021; **live improves to 12401** (best; ~9% < baseline), cleaner library.

Live's final cost improves **monotonically with ruleset richness** (14364 →
12980 → 12401) while baseline stays 13599; at-start loses under every ruleset.

## Key takeaways
1. Live-DSR's edge over at-start tracks the AC-census (validated: Enron ~0%→live≈
   at-start; multiplier 63%→live≫at-start).
2. Beating at-start ≠ beating baseline. With only AC, live loses to baseline
   (greedy phase-ordering). It takes a **richer, semantically-meaningful ruleset
   (De Morgan, then factoring)** to make live beat baseline — because that's what
   exposes genuinely non-canonical sharing the original forms didn't have.
3. at-start consistently does worst on this corpus: independent per-program
   min-term extraction with size-preserving rules scrambles cross-program
   alignment.
4. egg-stitch optimizes description-length (structural reuse), not boolean
   minimality — abstractions can look logically redundant yet compress well.

## Method/infra notes
- `--language op-children` (boolean circuits are first-order: gates + signal
  leaves, no binders). lambda-calc would be wrong here.
- SMC + fixed `--seed` is deterministic and needs no match-set cap, so it sidesteps
  the best-first `--max-match-set`/`--max-factor-rows` flag (which was renamed
  back and forth by a parallel session during this work).
- AIGER parser / cone extractor / AC-census: `scripts/aig_cones.py`.
  Corpus generator: `scripts/aig_to_egg.py` (regenerates `all.json` byte-identical).
  Experiment driver: `scripts/aig_mult_experiment.py`.
  Regression test: `scripts/test_aig_mult_regression.py` (census + corpus
  provenance + fixed 4-abstraction rollouts for all 5 configs; deterministic).

## Artifacts
- **PR #274** (branch `aig-mult-livedsr-clean`, base `add-physics-deriv-8k-table6`):
  3 commits — AIG experiment + regression test; De Morgan; factoring. No `src/`
  changes, no constant folding; uses only pre-existing flags.
- PR #273 closed (superseded — the shared working tree had interleaved a parallel
  session's matmul commit into the original branch's history).

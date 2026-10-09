# E-Stitch: Top-Down Library Learning with E-Graphs — Artifact

For an updated version of this document, see
https://github.com/mlb2251/egg-stitch/blob/main/artifact/README.md.

Contains E-Stitch and the two baselines, babble and Stitch, at the commits used
for the paper:

| Repository | Commit | Source |
| --- | --- | --- |
| egg-stitch (E-Stitch) | `@ESTITCH_COMMIT@` | https://github.com/mlb2251/egg-stitch |
| babble | `@BABBLE_COMMIT@` | https://github.com/kavigupta/babble |
| Stitch | `@STITCH_COMMIT@` | https://github.com/mlb2251/stitch |

## Claims

Sections, tables and figures refer to the paper. Outputs are in `figures/`.
Step 1 recomputes only E-Stitch's cells in Tables 1–4 and reads everything else
from the cache; step 2 recomputes the ablation; step 3 recomputes everything.

| # | Claim | Paper | Output | Step |
| --- | --- | --- | --- | --- |
| 1 | E-Stitch BFS and SMC get higher compression than Babble on every Babble benchmark, in less time | §7.2, Table 1 | `babble_comparison_rewrites.tex` | 1 |
| 2 | On molecules, E-Stitch BFS and SMC out-compress Babble, BFS/MT and BFS/NR; Babble does not finish on Hexyl; SMC is much faster than BFS at similar compression | §7.3, Table 2 | `molecules.tex` | 1 |
| 3 | Per-abstraction search progress of E-Stitch SMC vs BFS/MT on molecules | §7.3, Figure 6 | `molecules/search-progress.png` | 1 |
| 4 | On circuits, E-Stitch BFS and SMC are far more compressive than BFS/MT and BFS/NR; Babble finishes none; BFS/MT generally underperforms BFS/NR | §7.4, Table 3 | `circuits.tex` | 1 |
| 5 | Without rewrites, Stitch is fastest and E-Stitch BFS close behind, at similar compression | §7.5, Table 4 | `babble_comparison_no_rewrites.tex` | 1 |
| 6 | Every BFS pruning technique matters in at least one domain; equivalence pruning in all; SMC is less sensitive | §7.6, Tables 5, 7 | `ablation.tex`, `ablation-appendix.tex` | 2 |

The soundness and completeness proofs (§4.7, App. D) are on paper and are not
checked by the artifact.

## Setup

Requires an x86-64 host with Docker. Tested on Ubuntu 24.04 with Docker Engine 29.8.2.

```bash
docker load < estitch-artifact.tar.gz
mkdir -p estitch-out
docker run -it --name estitch -v "$PWD/estitch-out:/out" estitch-artifact
```

The working directory is `/artifact/egg-stitch`; babble and Stitch are at
`/artifact/babble` and `/artifact/stitch`. All tools are prebuilt and no network
access is needed. `/out` is `estitch-out/` on the host; each run below ends by
copying `figures/` (tables as LaTeX `tabular` fragments, figures as PNGs) there.
The `chown` gives the copies to the owner of `estitch-out/` instead of root.

If you exit the container or lose the session, re-enter it with
`docker start -ai estitch`; finished results are kept. To resume an interrupted
step, rerun only its `bash scripts/run_all_tables.sh` and `cp` lines, since its
first lines delete results. `docker rm estitch` deletes the container.

## Example

Learn one abstraction on the glycol molecule corpus (Table 2) with the molecule
rewrite rules (~5s):

```bash
cargo run --release -- --search smc \
    -i data/domains/molecules/scramble/glycol.scram.json \
    -r data/domains/molecules/molecules.rewrites \
    --num-particles 10000 --num-steps 100 --temperature 100 --max-arity 2
```

Expected compression ratio: ~2.6×. Without `-r`: ~1.4×. Results may vary slightly
between runs; `--seed` fixes them. `--help` lists all options.

## Reproducing the paper

`bash scripts/run_all_tables.sh` runs every experiment and renders all tables
and figures to `figures/`. Results are cached per (table, method) under
`results/` and reused; the cache used for the paper is included.

| Paper | Experiment | Output in `figures/` |
| --- | --- | --- |
| Table 1 | `babble_comparison_rewrites` | `babble_comparison_rewrites.tex` |
| Table 2 | `molecules` | `molecules.tex` |
| Table 3 | `circuits` | `circuits.tex` |
| Table 4 | `babble_comparison_no_rewrites` | `babble_comparison_no_rewrites.tex` |
| Table 5 | `ablation` | `ablation.tex` |
| Table 6 | `circuits_all` | `circuits_all.tex` |
| Table 7 | `ablation` | `ablation-appendix.tex` |
| Figure 6 | `molecules` | `molecules/search-progress.png` |
| Figure 10 | the four main tables | `curve-grid/` |
| Figure 11 | `arity_experiment` | `arity/` |

### Check hardware requirements

The paper's results, and the times below, were produced on an AMD Ryzen 7 5800X
(8 cores) with 64 GB of RAM, running Ubuntu 24.04. `molecules` and `circuits`/`circuits_all` experiments
require up to 20 GiB of free memory, and limit to 300s per run.

### 1. Recompute E-Stitch cells in the main paper tables (~1h)

The first command clears all the E-Stitch results reported in the main tables from the cache,
and the second command recomputes them, then regenerates the figures and tables in the paper.
(Note that the timing comparisons should not be read literally, as the baselines have not been
recomputed on your hardware).

```bash
rm -rf figures
python scripts/clear_table_estitch_results.py
bash scripts/run_all_tables.sh
cp -r figures /out/figures-from-short-run && chown -R --reference=/out /out
```

Note: there are 3 kick-down notices (where we use lower parameters to fit the
compute budget). Two are for `circuits_all`, which is not a main table and is not
recomputed here; the first should read

```
!!   circuits BFS: kicked down 10000 -> 2000 steps (geomean DNF at 10000)
```

This is expected, and is why `enum-2000` is the cell regenerated. On `square` it
takes 268s of the 300s cap on our machine; if it exceeds the cap, the table
falls back to our cached `enum-1000` and the notice reads `10000 -> 1000`. If so, do one of

  - ignore the circuits BFS cell and audit the rest of the results
  - raise `EXPERIMENT_TIMEOUT` and rerun as described in Section 1.5
  - run the full experiment as described in Section 3.

### 1.5. Main paper tables, but with increased time limit (Still ~1h)

To increase the time limit, prepend `EXPERIMENT_TIMEOUT=1200` to the `bash` command.
Note that this gives the recomputed E-Stitch cells a longer budget than the cached babble
and Stitch cells, which were run with 300s.

```bash
rm -rf figures
git restore results/
python scripts/clear_table_estitch_results.py
EXPERIMENT_TIMEOUT=1200 bash scripts/run_all_tables.sh
cp -r figures /out/figures-from-short-run-increased-time-limit && chown -R --reference=/out /out
```

`git restore results/` restores the original cache.

### 2. Recompute the ablation (~3.5h)

This recomputes Tables 5 and 7 on the domains and BFS points the paper uses
(Furniture, Hexyl and Square).

```bash
rm -rf figures
rm -rf results/ablation
bash scripts/run_all_tables.sh
cp -r figures /out/figures-from-ablation-run && chown -R --reference=/out /out
```

On our machine the ablation's `circuits` target run (BFS on Square, 2000 steps)
takes 244s of the 300s cap. If it exceeds the cap, the ablation stops and
`ablation.tex` and `ablation-appendix.tex` are not produced; rerun all of step 2's
commands with `EXPERIMENT_TIMEOUT=1200` prepended to the `bash` command.

### 3. Rerun everything (~24h) [Optional]

```bash
rm -rf figures
rm -rf results/
bash scripts/run_all_tables.sh
cp -r figures /out/figures-from-long-run && chown -R --reference=/out /out
```

Interrupted runs resume from the last (table, method) that finished (see Setup).

### Expected similarities

- BFS should give the same compression as the paper.
- SMC should give similar compression; it is stochastic, and results are
  aggregated over 10 runs.
- Both should generally remain above the baselines.

### Expected differences

- Timings depend on hardware.
- Tables 2, 3 and 6 cap each run at 300s and 20 GiB, reporting DNF otherwise.
  Runs near the cap may change status on different hardware.

## Running on your own inputs

A corpus is a JSON list of programs as s-expressions, for example
`data/domains/examples-paper/corpus_b.json`:

```json
[
    "(+ (- a) (* b b))",
    "(+ (- (* c d)) (* g g))",
    "(sqrt (+ (* i i) (- h)))",
    "(exp (* (/ y 2) (/ y 2)))"
]
```

A rewrite-rule file has one rule per line, `name: lhs => rhs` (one direction)
or `name: lhs <=> rhs` (both directions). `?x` is a pattern variable and `//`
starts a comment. `data/domains/examples-paper/rules.rewrites`:

```
plus_comm: (+ ?x ?y) <=> (+ ?y ?x)
add_zero: ?x <=> (+ 0 ?x)
neg_zero: (- 0) <=> 0
```

```bash
cargo run --release -- --search best-first --max-arity 2 \
    -i data/domains/examples-paper/corpus_b.json \
    -r data/domains/examples-paper/rules.rewrites \
    -o out.json
```

This finds `(+ (- ?#0) (* ?#1 ?#1))` (~1.20×), which needs the rules to match its
rearranged uses; without `-r` it finds only `(* ?#0 ?#0)` (~1.11×).

- `--language lambda-calc` reads lambda-calculus programs, with `lam` and de
  Bruijn variables `$0`, `$1`, … (see `data/domains/list/`).
- `--num-abstractions N` learns N abstractions in sequence.
- `-o` writes JSON with the learned abstractions (`library`), the rewritten
  corpus (`rewritten_programs`) and `compression_ratio`.
- `data/domains/` has more corpora and rule files; `--help` lists all options.

## Layout of `/artifact/egg-stitch`

- `src/`: E-Stitch (Rust).
- `data/domains/`: benchmark corpora and rewrite rules.
- `expts/`: experiment definitions and tool wrappers.
- `scripts/`: rendering and utility scripts.
- `results/`, `figures/`: cached results and rendered outputs.
- `tests/`: snapshot tests (`cargo test --release --test snapshots`).

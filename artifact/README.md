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

## Setup

Requires x86-64 and Docker.

```bash
docker load < estitch-artifact.tar.gz
mkdir -p estitch-out
docker run --rm -it -v "$PWD/estitch-out:/out" estitch-artifact
```

The working directory is `/artifact/egg-stitch`; babble and Stitch are at
`/artifact/babble` and `/artifact/stitch`. All tools are prebuilt and no network
access is needed. `/out` is `estitch-out/` on the host; each run below ends by
copying `figures/` (tables as LaTeX `tabular` fragments, figures as PNGs) there.
The `chown` gives the copies to the owner of `estitch-out/` instead of root.

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
and figures to `figures/`. Results are cached per (method, domain) under
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

```bash
python scripts/clear_table_estitch_results.py
bash scripts/run_all_tables.sh
cp -r figures /out/figures-recompute && chown -R --reference=/out /out
```

Only the sweep point each table reports is recomputed, so a run that exceeds the
default 300s cap can't fall back to a smaller one and would DNF.
If you are running on a machine with a slower CPU, you may need to adjust the time limit
by prepending e.g., `EXPERIMENT_TIMEOUT=1200` to the `bash` command, as in:

```bash
python scripts/clear_table_estitch_results.py
EXPERIMENT_TIMEOUT=1200 bash scripts/run_all_tables.sh
cp -r figures /out/figures-recompute && chown -R --reference=/out /out
```

`git restore results/` restores the original cache.

### 2. Rerun everything (~24h)

```bash
rm -rf results/
bash scripts/run_all_tables.sh
cp -r figures /out/figures-rerun && chown -R --reference=/out /out
```

Interrupted runs resume from the cache.

### Expected similarities

- BFS should give the same compression as the paper.
- SMC should give similar compression; it is stochastic, and results are
  aggregated over 10 runs.
- Both should generally remain above the baselines.

### Expected differences

- Timings depend on hardware.
- Tables 2, 3 and 6 cap each run at 300s and 20 GiB, reporting DNF otherwise.
  Runs near the cap may change status on different hardware.

## Layout of `/artifact/egg-stitch`

- `src/`: E-Stitch (Rust).
- `data/domains/`: benchmark corpora and rewrite rules.
- `expts/`: experiment definitions and tool wrappers.
- `scripts/`: rendering and utility scripts.
- `results/`, `figures/`: cached results and rendered outputs.
- `tests/`: snapshot tests (`cargo test --release --test snapshots`).

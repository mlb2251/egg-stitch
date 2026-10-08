# E-Stitch: Top-Down Library Learning with E-Graphs — Artifact

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
docker run --rm -it estitch-artifact
```

The working directory is `/artifact/egg-stitch`; babble and Stitch are at
`/artifact/babble` and `/artifact/stitch`. All tools are prebuilt and no network
access is needed.

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

### 1. Recompute E-Stitch cells in the main paper tables (~1h)

All other results are read from the cache.

```bash
python scripts/clear_table_estitch_results.py
bash scripts/run_all_tables.sh
```

`git restore results/` restores the original cache.

### 2. Rerun everything (~24h)

```bash
rm -rf results/
bash scripts/run_all_tables.sh
```

Interrupted runs resume from the cache.

### Expected differences

- Timings depend on hardware.
- Enum and baseline compression ratios are deterministic. SMC results are
  aggregated over 10 runs and vary slightly.
- Tables 2, 3 and 6 cap each run at 300s and 20 GiB, reporting DNF otherwise.
  Runs near the cap may change status on different hardware.
- `molecules`, `circuits` and `circuits_all` require 20 GiB of free memory.

The paper's results, and the times above, were produced on an AMD Ryzen 7 5800X
(8 cores) with 64 GB of RAM, running Ubuntu 24.04.

## Layout of `/artifact/egg-stitch`

- `src/`: E-Stitch (Rust).
- `data/domains/`: benchmark corpora and rewrite rules.
- `expts/`: experiment definitions and tool wrappers.
- `scripts/`: rendering and utility scripts.
- `results/`, `figures/`: cached results and rendered outputs.
- `tests/`: snapshot tests (`cargo test --release --test snapshots`).

## Rebuilding the image

`/artifact` is the Docker build context, with all Rust crates in `vendor/` and
Python wheels in `wheels/`.

```bash
docker build -t estitch-artifact .
```

To regenerate the build context from an egg-stitch checkout, with Stitch
checked out at its pinned commit alongside it:

```bash
artifact/prepare.sh
docker build -t estitch-artifact artifact/build
docker save estitch-artifact | gzip > estitch-artifact.tar.gz
```

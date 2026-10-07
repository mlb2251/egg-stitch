# E-Stitch: Top-Down Library Learning with E-Graphs — Artifact

This artifact contains E-Stitch (the `egg-stitch` repository) and the two
baselines it is compared against, babble and Stitch. It reproduces every
table and figure in the paper's evaluation.

| Repository | Pinned commit | Source |
| --- | --- | --- |
| egg-stitch (E-Stitch) | `@ESTITCH_COMMIT@` | https://github.com/mlb2251/egg-stitch |
| babble | `@BABBLE_COMMIT@` | https://github.com/kavigupta/babble |
| Stitch | `@STITCH_COMMIT@` | https://github.com/mlb2251/stitch |

The GitHub repositories are where development continues. This archive is
frozen at the commits the paper's numbers were produced with.

## Getting started

```bash
docker load < estitch-artifact.tar.gz
docker run --rm -it estitch-artifact
```

You land in `/artifact/egg-stitch`. babble and Stitch are next to it at
`/artifact/babble` and `/artifact/stitch`, and all three tools are already
built. Nothing in the container needs network access.

### An example run

E-Stitch is a single command-line binary. This finds one abstraction for the
ester molecule corpus (paper Table 2), using the molecule rewrite rules, and
takes about a second:

```bash
cargo run --release -- --search smc \
    -i data/domains/molecules/scramble/ester.scram.json \
    -r data/domains/molecules/molecules.rewrites \
    --num-particles 1000 --num-steps 100 --temperature 100 --max-arity 2 \
    -o ester.json
```

It prints the abstraction it found and its compression ratio, about 2.2×,
and writes the full result to `ester.json`. If you drop the `-r` flag, the same
search runs without rewrite rules. It finds the same pattern but can only use
it at the sites where it appears syntactically, so compression falls to about
1.4×. That gap is the paper's main point. SMC is stochastic, so the exact
numbers can vary from run to run (`--seed` fixes them).
`cargo run --release -- --help` lists every option.

## Reproducing the paper

Every number in the paper comes from a table experiment in `expts/tables.py`,
run with `./run.py <experiment>`. Each (method, domain) cell is cached as JSON
under `results/`, and the cache the paper was produced from is included. The
render scripts read `results/` and write the tables and figures to `figures/`.

| In the paper | Experiment | Rendered by | Output under `figures/` |
| --- | --- | --- | --- |
| Table 1 | `table3` | `scripts/render_tables.py` | `table3.tex` |
| Table 2 | `table5` | `scripts/render_tables.py` | `table5.tex` |
| Table 3 | `table7` | `scripts/render_tables.py` | `table7.tex` |
| Table 4 | `table4` | `scripts/render_tables.py` | `table4.tex` |
| Table 5 | `ablation` | `scripts/render_ablation.py` | `ablation.tex` |
| Table 6 (appendix) | `table7_5` | `scripts/render_tables.py` | `table7_5.tex` |
| Table 7 (appendix) | `ablation` | `scripts/render_ablation.py` | `ablation-appendix.tex` |
| Figure 6 | `table5` | `scripts/render_molecules.py` | `molecules/search-progress.png` |
| Figure 10 | `table3`, `table4`, `table5`, `table7` | `scripts/render_tables.py` | `curve-grid/` |
| Figure 11 | `arity_experiment` | `scripts/render_arity.py` | `arity/` |

`table1` and `table2` are one-abstraction versions of `table3` and `table4`.
They aren't in the paper, but they are part of the full run.

There are three levels of reproduction, from cheapest to most expensive.

### 1. Render from the shipped results (minutes)

```bash
python scripts/render_tables.py
python scripts/render_molecules.py
python scripts/render_ablation.py
python scripts/render_arity.py
```

This regenerates `figures/` from the cached results. Compare the `.tex`
tables against the paper.

### 2. Recompute the E-Stitch cells in the main paper tables (TODO hours)

This reruns E-Stitch for the cells shown in Tables 1–4 (and in `table1` and
`table2`). babble, Stitch and the other points of the E-Stitch hyperparameter
sweeps are read from the cache:

```bash
python scripts/clear_table_estitch_results.py
for t in table1 table2 table3 table4 table5 table7; do ./run.py $t; done
python scripts/render_tables.py
```

`git diff results/` shows what changed, and `git restore results/` puts the
shipped cache back.

### 3. Rerun everything (TODO days)

```bash
rm -rf results/
scripts/run_all_tables.sh
python scripts/render_molecules.py
```

`run_all_tables.sh` runs every experiment and renders the tables, the arity
figure and the ablation. Every cell is cached as it finishes, so an
interrupted run picks up where it left off.

### What to expect when recomputing

- **Timing columns** depend on the machine and won't match the paper exactly.
- **Compression ratios** are what should be checked. Enumeration (Enum) and
  the baselines are deterministic, so their ratios should match. SMC is
  stochastic: each SMC cell is reported over 10 runs, so expect small
  run-to-run differences.
- **Capped cells.** Tables 2, 3 and 6 cap every tool run at 300s and 20 GiB of
  memory, and a run that hits either cap is reported as DNF. On a slower
  machine, a cell that finished close to the cap can become a DNF (or the
  reverse on a faster one).
- `table5`, `table7` and `table7_5` refuse to start unless 20 GiB of memory is
  free, so that the memory cap means the same thing for every tool. Give the
  container at least that much (on Docker Desktop: Settings → Resources).

## Hardware

The paper's numbers were produced on TODO. The image is built for x86-64. On
other architectures (e.g. Apple Silicon), see "Rebuilding the image": a native
build avoids emulation, which would distort timings.

## Repository layout (`/artifact/egg-stitch`)

- `src/`: E-Stitch itself (Rust, built on the egg e-graph library).
- `data/domains/`: benchmark corpora and their rewrite rules.
- `expts/`: experiment harness: the table definitions (`tables.py`) and
  wrappers that run E-Stitch, babble and Stitch (`run_models/`).
- `scripts/`: rendering and utility scripts.
- `results/`, `figures/`: cached results and rendered outputs.
- `tests/`: the end-to-end snapshot suite, run with
  `cargo test --release --test snapshots`.

## Rebuilding the image

The directory containing this README (`/artifact` in the image) is the full
Docker build context: the three repositories at their pinned commits, every
Rust dependency vendored under `vendor/`, and every Python package (pinned in
`egg-stitch/requirements-lock.txt`) under `wheels/`. Only the base image and
the Rust toolchain are downloaded.

```bash
docker build -t estitch-artifact .
```

The archive itself was produced from a checkout of egg-stitch with Stitch
cloned next to it at its pinned commit (Stitch doesn't track its `Cargo.lock`,
so `prepare.sh` ships that checkout's):

```bash
artifact/prepare.sh
docker build -t estitch-artifact artifact/build
docker save estitch-artifact | gzip > estitch-artifact.tar.gz
```

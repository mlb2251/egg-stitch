"""Table 1-4 experiments: Ours (Enum + SMC) vs babble (vs Stitch) on the
cogsci drawing domains plus the dreamcoder benchmarks.

Each table is a single configuration of: which domains to run, which
runners participate, whether DSRs are enabled, and ``num_abstractions``.

    Table 1: with DSRs, 1 abstraction,  Enum/SMC/babble.
    Table 2: no DSRs,   1 abstraction,  Enum/SMC/babble/Stitch.
    Table 3: with DSRs, 20 abstractions (same shape as Table 1).
    Table 4: no DSRs,   20 abstractions (same shape as Table 2).
"""

from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Sequence

from tqdm import tqdm

from . import ALL_DOMAINS
from .folders import SUMMARY_RESULTS_DIR, set_folder, summary_results_path
from .run_models import Babble, OursBf, OursSmc, Stitch

NUM_RUNS = 10

# Table 1 / 3: babble has no equational theory for text/logo/towers, so the
# "with DSRs" comparison excludes them.
TABLE1_DOMAINS = ["nuts-bolts", "dials", "wheels", "furniture", "list", "physics"]
# Table 2 / 4: no-DSR comparison, includes the dreamcoder domains without
# rewrite files (text/logo/towers).
TABLE2_DOMAINS = TABLE1_DOMAINS + ["text", "logo", "towers"]

# Hyperparameter sweeps for the two ours-search modes. Each value gets its
# own runner / cache file, labelled ``enum-<num_steps>`` /
# ``smc-<num_particles>`` so the renderer can group sweep points back into
# a single line per base method. The table cells still show one canonical
# point per method (see ``TABLE_BFS_STEPS`` / ``TABLE_SMC_PARTICLES`` in
# ``scripts/render_tables.py``).
BFS_STEP_SWEEP: tuple[int, ...] = (200, 500, 1000, 2000, 5000, 10000, 20000, 50000)
SMC_PARTICLE_SWEEP: tuple[int, ...] = (20, 50, 100, 200, 500, 1000, 2000, 5000)


def _sweep_runners() -> tuple[tuple[str, object], ...]:
    """``(label, runner)`` pairs for every BFS-step and SMC-particle sweep value."""
    bfs = tuple((f"enum-{n}", OursBf(num_steps=n)) for n in BFS_STEP_SWEEP)
    smc = tuple((f"smc-{p}", OursSmc(num_particles=p)) for p in SMC_PARTICLE_SWEEP)
    return bfs + smc


# Runner rosters — Table 1/3 share the no-Stitch roster, Table 2/4 share
# the with-Stitch roster.
BASE_RUNNERS: tuple[tuple[str, object], ...] = _sweep_runners() + (
    ("babble", Babble()),
)
RUNNERS_WITH_STITCH: tuple[tuple[str, object], ...] = BASE_RUNNERS + (
    ("stitch", Stitch()),
)


def _run_method_for_table(
    label: str,
    runner: object,
    *,
    domains: Sequence[str],
    num_abstractions: int,
    use_dsrs: bool,
    cache_path: Path,
    bar: tqdm,
) -> dict[str, list[list[dict]]]:
    """Run one method across all domains × ``NUM_RUNS`` for a single table.

    Returns ``{domain: [run0_per_file_dicts, run1_per_file_dicts, ...]}``.
    Cached as a single JSON file per (table, method); delete the file to
    force a recompute.
    """
    from .runner import run_method  # local import: runner pulls heavy deps

    if cache_path.exists():
        with open(cache_path) as fh:
            out = json.load(fh)
        bar.update(len(domains) * NUM_RUNS)
        return out

    out: dict[str, list[list[dict]]] = {}
    for domain in domains:
        runs: list[list[dict]] = []
        for i in range(NUM_RUNS):
            bar.set_description(f"{domain} {label} rep {i+1}/{NUM_RUNS}")
            per_file = run_method(
                runner,
                domain,
                rounds=num_abstractions,
                use_dsrs=use_dsrs,
            )
            runs.append([r.to_dict() for r in per_file])
            bar.update()
        out[domain] = runs

    cache_path.parent.mkdir(parents=True, exist_ok=True)
    with open(cache_path, "w") as fh:
        json.dump(out, fh, indent=2)
    return out


def _run_table(
    *,
    domains: Sequence[str],
    runners: Sequence[tuple[str, object]],
    num_abstractions: int,
    use_dsrs: bool,
    folder_prefix: str,
    output_name: str,
) -> Path:
    """Run each ``(label, runner)`` on every domain ``NUM_RUNS`` times and save JSON."""
    assert all(d in ALL_DOMAINS for d in domains), "domain typo"
    set_folder(f"{folder_prefix}/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    results: dict = {
        "config": {"num_abstractions": num_abstractions},
        "domains": {domain: {"runs": {}} for domain in domains},
    }
    cache_root = SUMMARY_RESULTS_DIR / Path(output_name).stem

    total = len(domains) * NUM_RUNS * len(runners)
    with tqdm(total=total, unit="run", smoothing=0.05) as bar:
        for label, runner in runners:
            by_domain = _run_method_for_table(
                label,
                runner,
                domains=domains,
                num_abstractions=num_abstractions,
                use_dsrs=use_dsrs,
                cache_path=cache_root / f"{label}.json",
                bar=bar,
            )
            for domain, runs in by_domain.items():
                results["domains"][domain]["runs"][label] = runs

    out_path = summary_results_path(output_name)
    with open(out_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nwrote {out_path}", flush=True)
    return out_path


def table1() -> Path:
    """Run Enum, SMC, and babble on the Table 1 domains with DSRs."""
    return _run_table(
        domains=TABLE1_DOMAINS,
        runners=BASE_RUNNERS,
        num_abstractions=1,
        use_dsrs=True,
        folder_prefix="table1",
        output_name="table1.json",
    )


def table2() -> Path:
    """Run Enum, SMC, babble, and Stitch on the Table 2 domains with no DSRs."""
    return _run_table(
        domains=TABLE2_DOMAINS,
        runners=RUNNERS_WITH_STITCH,
        num_abstractions=1,
        use_dsrs=False,
        folder_prefix="table2",
        output_name="table2.json",
    )


def table3() -> Path:
    """Run the Table 1 setup with 20 stacked abstractions."""
    return _run_table(
        domains=TABLE1_DOMAINS,
        runners=BASE_RUNNERS,
        num_abstractions=20,
        use_dsrs=True,
        folder_prefix="table3",
        output_name="table3.json",
    )


def table4() -> Path:
    """Run the Table 2 setup with 20 stacked abstractions."""
    return _run_table(
        domains=TABLE2_DOMAINS,
        runners=RUNNERS_WITH_STITCH,
        num_abstractions=20,
        use_dsrs=False,
        folder_prefix="table4",
        output_name="table4.json",
    )

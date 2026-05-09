"""Table 2 experiment: Ours (Enum + SMC) vs babble vs Stitch, no DSRs.

Same cogsci domains as Table 1 plus the dreamcoder benchmarks without DSRs
(text/logo/towers); every method runs *without* any domain-specific
rewrites, and Stitch is included (Table 1 uses DSRs, which Stitch doesn't
accept). Results land under ``viz/results/table2/<timestamp>/``.
"""

import json
import time
from pathlib import Path

import numpy as np

from . import ALL_DOMAINS
from .folders import current_folder_path, set_folder
from .run_models import Babble, OursBf, OursSmc, Stitch
from .runner import run_method
from .table1 import DOMAIN_LABELS, NUM_RUNS

# Table 2 is the no-DSR comparison, so it includes the dreamcoder domains
# without rewrite files (text/logo/towers) in addition to everything in
# Table 1.
TABLE2_DOMAINS = ["nuts-bolts", "dials", "wheels", "furniture", "list", "physics", "text", "logo", "towers"]


DEFAULT_TABLE2_TITLE = "Table 2: Ours (SMC and Enum) vs Babble vs Stitch on benchmarks without DSRs"


def table2(
    *,
    num_abstractions: int = 1,
    folder_prefix: str = "table2",
    output_name: str = "table2.json",
    title: str = DEFAULT_TABLE2_TITLE,
    enum: OursBf = OursBf(),
    smc: OursSmc = OursSmc(),
    babble: Babble = Babble(),
    stitch: Stitch = Stitch(),
) -> Path:
    """Run Enum, SMC, babble, and Stitch on the Table 2 domains with no DSRs.

    Each runner is a dataclass instance carrying its own hyperparameters; pass
    overrides as kwargs at construction (see :func:`expts.table1.table1` for
    the pattern). ``num_abstractions`` is forwarded to every compressor so
    each run stacks that many abstractions sequentially.
    """
    assert all(d in ALL_DOMAINS for d in TABLE2_DOMAINS), "domain typo"
    set_folder(f"{folder_prefix}/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    results: dict = {
        "title": title,
        "config": {"num_abstractions": num_abstractions},
        "domains": {},
    }

    for domain in TABLE2_DOMAINS:
        print(f"\n=== {domain} ===", flush=True)
        enum_runs, smc_runs, babble_runs, stitch_runs = [], [], [], []
        for i in range(NUM_RUNS):
            print(f"  run {i+1}/{NUM_RUNS}", flush=True)
            enum_res, _ = run_method(enum, domain, rounds=num_abstractions, use_dsrs=False)
            smc_res, _ = run_method(smc, domain, rounds=num_abstractions, use_dsrs=False)
            babble_res, _ = run_method(babble, domain, rounds=num_abstractions, use_dsrs=False)
            stitch_res, _ = run_method(stitch, domain, rounds=num_abstractions, use_dsrs=False)
            enum_runs.append(enum_res.to_dict())
            smc_runs.append(smc_res.to_dict())
            babble_runs.append(babble_res.to_dict())
            stitch_runs.append(stitch_res.to_dict())
        results["domains"][domain] = {
            "runs": {"enum": enum_runs, "smc": smc_runs, "babble": babble_runs, "stitch": stitch_runs},
        }

    out_path = current_folder_path() / output_name
    with open(out_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nwrote {out_path}", flush=True)
    print_table2(out_path)
    return out_path


def _fmt(x, spec: str, na: str = "N/A") -> str:
    """Format ``x`` with ``spec`` or return ``na`` when ``x`` is None."""
    return na if x is None else format(x, spec)


def print_table2(path: str | Path) -> None:
    """Pretty-print a saved Table 2 JSON."""
    with open(path) as f:
        saved = json.load(f)
    domains = saved["domains"]

    header_top = (
        f"{'':<14}{'':>14}  "
        f"{'Compression Ratio':^36}  {'Time (s)':^36}"
    )
    header_sub = (
        f"{'':<14}{'original size':>14}  "
        f"{'Enum':>10}{'SMC':>10}{'babble':>8}{'Stitch':>8}  "
        f"{'Enum':>10}{'SMC':>10}{'babble':>8}{'Stitch':>8}"
    )
    print()
    print(saved.get("title", DEFAULT_TABLE2_TITLE))
    print()
    print(header_top)
    print(header_sub)
    print("-" * len(header_sub))
    for domain in TABLE2_DOMAINS:
        if domain not in domains:
            continue
        d = domains[domain]
        runs = d.get("runs", {})
        label = DOMAIN_LABELS.get(domain, domain)
        any_run = (runs.get("enum") or next(iter(runs.values())))[0]
        original_size = any_run["initial_cost"]

        def cr(m):
            if m not in runs:
                return None
            return float(np.exp(np.mean(np.log([r["compression_ratio"] for r in runs[m]]))))

        def t(m):
            if m not in runs:
                return None
            return float(np.exp(np.mean(np.log([r["elapsed_secs"] for r in runs[m]]))))

        row = (
            f"{label:<14}"
            f"{_fmt(original_size, 'd'):>14}  "
            f"{_fmt(cr('enum'), '.2f'):>10}"
            f"{_fmt(cr('smc'), '.2f'):>10}"
            f"{_fmt(cr('babble'), '.2f'):>8}"
            f"{_fmt(cr('stitch'), '.2f'):>8}  "
            f"{_fmt(t('enum'), '.1f'):>10}"
            f"{_fmt(t('smc'), '.1f'):>10}"
            f"{_fmt(t('babble'), '.1f'):>8}"
            f"{_fmt(t('stitch'), '.1f'):>8}"
        )
        print(row)
    print()

"""Table 2 experiment: Ours (Enum + SMC) vs babble vs Stitch, no DSRs.

Same four cogsci domains as Table 1 but every method runs *without* any
domain-specific rewrites, and Stitch is actually included (Table 1 runs
with DSRs, which Stitch doesn't accept). Results land under
``viz/results/table2/<timestamp>/``.
"""

import json
import time
from pathlib import Path

import numpy as np

from . import *
from .babble import *
from .egg_stitch import *
from .stitch import *
from .table1 import TABLE1_DOMAINS, DOMAIN_LABELS, NUM_RUNS

# Reuse the Table 1 domain list/labels so the HTML viewer sees the same order.
TABLE2_DOMAINS = TABLE1_DOMAINS


DEFAULT_TABLE2_TITLE = "Table 2: Ours (SMC and Enum) vs Babble vs Stitch on benchmarks without DSRs"

MAX_ARITY = 2


def table2(
    *,
    smc_num_steps: int = 100,
    smc_num_particles: int = 1000,
    smc_temperature: float = 1000.0,
    enum_num_steps: int = 500,
    num_abstractions: int = 1,
    rebuild_egraph: bool = False,
    folder_prefix: str = "table2",
    output_name: str = "table2.json",
    title: str = DEFAULT_TABLE2_TITLE,
) -> Path:
    """Run Enum, SMC, babble, and Stitch on the four domains with no DSRs.

    ``num_abstractions`` is forwarded to every compressor so each run
    stacks that many abstractions sequentially.
    """
    assert all(d in ALL_DOMAINS for d in TABLE2_DOMAINS), "domain typo"
    set_folder(f"{folder_prefix}/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    results: dict = {
        "title": title,
        "config": {
            "smc": {"num_steps": smc_num_steps, "num_particles": smc_num_particles, "temperature": smc_temperature},
            "enum": {"num_steps": enum_num_steps},
            "num_abstractions": num_abstractions,
            "rebuild_egraph": rebuild_egraph,
            "max_arity": MAX_ARITY,
        },
        "domains": {},
    }

    for domain in TABLE2_DOMAINS:
        print(f"\n=== {domain} ===", flush=True)
        enum_runs, smc_runs, babble_runs, stitch_runs = [], [], [], []
        for i in range(NUM_RUNS):
            print(f"  run {i+1}/{NUM_RUNS}", flush=True)
            enum_res, _ = run_ours(domain, "best-first", num_steps=enum_num_steps, rewrites=None, num_abstractions=num_abstractions, rebuild_egraph=rebuild_egraph, max_arity=MAX_ARITY, no_zero_arity=True)
            smc_res, _ = run_ours(
                domain, "smc",
                num_steps=smc_num_steps,
                num_particles=smc_num_particles,
                temperature=smc_temperature,
                rewrites=None,
                num_abstractions=num_abstractions,
                rebuild_egraph=rebuild_egraph,
                max_arity=MAX_ARITY,
                no_zero_arity=True,
            )
            babble_res = run_babble(domain, num_abstractions=num_abstractions, max_arity=MAX_ARITY)
            stitch_res = run_stitch(domain, num_abstractions=num_abstractions, max_arity=MAX_ARITY)
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

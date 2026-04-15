"""Table 1 experiment: compare Ours (Enum + SMC) against babble and Stitch.

Runs each method on all four cogsci domains with rewrite rules turned on,
stores results (one :class:`Result` per (method, domain) pair) in a single
JSON under the session results folder, and offers ``print_table1`` to
render a text table matching the paper layout.
"""

import json
import time
from pathlib import Path

import numpy as np

from . import *
from .babble import *
from .egg_stitch import *

DOMAIN_LABELS = {
    "nuts-bolts": "Nuts & Bolts",
    "dials": "Dials",
    "wheels": "Wheels",
    "furniture": "Furniture",
}


def table1(
    *,
    smc_num_steps: int = 100,
    smc_num_particles: int = 1000,
    smc_temperature: float = 1000.0,
    enum_num_steps: int = 500,
    max_arity: int = 2,
    num_runs: int = 10,
    domains: list[str] | None = None,
    output_name: str = "table1.json",
) -> Path:
    """Run Enum, SMC, and babble on ``domains`` with rewrites.

    Defaults to all four cogsci domains. Collects one :class:`Result` per
    (method, domain) pair into a single JSON in the current results folder
    and calls :func:`print_table1`.
    """
    if domains is None:
        domains = ALL_DOMAINS
    assert all(d in ALL_DOMAINS for d in domains), "domain typo"
    set_folder(f"table1/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    results: dict = {
        "config": {
            "smc": {"num_steps": smc_num_steps, "num_particles": smc_num_particles, "temperature": smc_temperature},
            "enum": {"num_steps": enum_num_steps},
        },
        "domains": {},
        "max_arity": max_arity,
        "num_runs": num_runs,
    }

    for domain in domains:
        print(f"\n=== {domain} ===", flush=True)
        stackpathpush(domain)
        enum_runs, smc_runs, babble_runs = [], [], []
        egraph_min = None
        for i in range(num_runs):
            print(f"  run {i+1}/{num_runs}", flush=True)
            stackpathpush(f"rep{i}")
            with subgroup("best-first"):
                enum_res, egraph_min = run_ours(domain, "best-first", num_steps=enum_num_steps, max_arity=max_arity)
            with subgroup("smc"):
                smc_res, _ = run_ours(
                    domain, "smc",
                    num_steps=smc_num_steps,
                    num_particles=smc_num_particles,
                    temperature=smc_temperature,
                    max_arity=max_arity,
                )
            with subgroup("babble"):
                babble_res = run_babble(domain, dsr=rewrites_path(domain), max_arity=max_arity)
            enum_runs.append(enum_res.to_dict())
            smc_runs.append(smc_res.to_dict())
            babble_runs.append(babble_res.to_dict())
            stackpathpop()
        results["domains"][domain] = {
            "egraph_min_size": egraph_min,
            "runs": {"enum": enum_runs, "smc": smc_runs, "babble": babble_runs},
        }
        stackpathpop()

    out_path = current_folder_path() / output_name
    with open(out_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nwrote {out_path}", flush=True)
    print_table1(out_path)
    return out_path


def _fmt(x, spec: str, na: str = "N/A") -> str:
    """Format ``x`` with ``spec`` or return ``na`` when ``x`` is None."""
    return na if x is None else format(x, spec)


def print_table1(path: str | Path) -> None:
    """Pretty-print a saved Table 1 JSON in the layout from the paper."""
    with open(path) as f:
        saved = json.load(f)
    domains = saved["domains"]

    header_top = (
        f"{'':<14}{'':>14}{'':>22}  "
        f"{'Compression Ratio':^36}  {'Time (s)':^36}"
    )
    header_sub = (
        f"{'':<14}{'original size':>14}{'E-graph min term size':>22}  "
        f"{'Enum':>10}{'SMC':>10}{'babble':>8}{'Stitch':>8}  "
        f"{'Enum':>10}{'SMC':>10}{'babble':>8}{'Stitch':>8}"
    )
    print()
    print("Table 1: Ours (SMC and Enum) vs Babble on benchmarks with domain-specific rewrites")
    print()
    print(header_top)
    print(header_sub)
    print("-" * len(header_sub))
    for domain in domains:
        d = domains[domain]
        runs = d.get("runs", {})
        label = DOMAIN_LABELS.get(domain, domain)
        # "original size" is the same for all runs; take it from the first enum run.
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
            f"{_fmt(original_size, 'd'):>14}"
            f"{_fmt(d.get('egraph_min_size'), 'd'):>22}  "
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

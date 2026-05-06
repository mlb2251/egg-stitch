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

NUM_RUNS = 10

# Order matches the Table 1 screenshot, with the dreamcoder benchmarks that
# ship with DSRs appended after the four cogsci drawing domains. text/logo/
# towers are excluded: babble has no equational theory for them, so a "with
# DSRs" comparison is not defined.
TABLE1_DOMAINS = ["nuts-bolts", "dials", "wheels", "furniture", "list", "physics"]
DOMAIN_LABELS = {
    "nuts-bolts": "Nuts & Bolts",
    "dials": "Dials",
    "wheels": "Wheels",
    "furniture": "Furniture",
    "list": "List",
    "physics": "Physics",
    "text": "Text",
    "logo": "Logo",
    "towers": "Towers",
}


DEFAULT_TABLE1_TITLE = "Table 1: Ours (SMC and Enum) vs Babble on benchmarks with domain-specific rewrites"

MAX_ARITY = 2


def table1(
    *,
    smc_num_steps: int = 100,
    smc_num_particles: int = 1000,
    smc_temperature: float = 1000.0,
    enum_num_steps: int = 500,
    num_abstractions: int = 1,
    rebuild_egraph: bool = False,
    folder_prefix: str = "table1",
    output_name: str = "table1.json",
    title: str = DEFAULT_TABLE1_TITLE,
) -> Path:
    """Run Enum, SMC, and babble on the four Table 1 domains with rewrites.

    Collects one :class:`Result` per (method, domain) pair into a single
    JSON in the current results folder and calls :func:`print_table1`.
    ``num_abstractions`` is forwarded to our compressor so each run stacks
    that many abstractions sequentially.
    """
    assert all(d in ALL_DOMAINS for d in TABLE1_DOMAINS), "domain typo"
    # Each run gets its own subfolder under viz/results/<folder_prefix>/ so the
    # HTML viewer can enumerate them independently of other experiments.
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

    for domain in TABLE1_DOMAINS:
        print(f"\n=== {domain} ===", flush=True)
        d_enum_steps = scale_budget_for_domain(domain, enum_num_steps)
        d_smc_particles = scale_budget_for_domain(domain, smc_num_particles)
        enum_runs, smc_runs, babble_runs = [], [], []
        egraph_min_term_size = None
        for i in range(NUM_RUNS):
            print(f"  run {i+1}/{NUM_RUNS}", flush=True)
            enum_res, enum_egraph_min = run_ours(domain, "best-first", num_steps=d_enum_steps, num_abstractions=num_abstractions, rebuild_egraph=rebuild_egraph, max_arity=MAX_ARITY, no_zero_arity=True)
            smc_res, smc_egraph_min = run_ours(
                domain, "smc",
                num_steps=smc_num_steps,
                num_particles=d_smc_particles,
                temperature=smc_temperature,
                num_abstractions=num_abstractions,
                rebuild_egraph=rebuild_egraph,
                max_arity=MAX_ARITY,
                no_zero_arity=True,
            )
            # The post-rewrite e-graph minimum term size is a property of the
            # corpus + DSRs alone, so Enum and SMC must agree on it.
            assert enum_egraph_min == smc_egraph_min, (
                f"{domain}: e-graph min term size disagrees between algorithms "
                f"(enum={enum_egraph_min}, smc={smc_egraph_min})"
            )
            egraph_min_term_size = enum_egraph_min
            babble_res = run_babble(domain, dsr=rewrites_path(domain), num_abstractions=num_abstractions, max_arity=MAX_ARITY)
            enum_runs.append(enum_res.to_dict())
            smc_runs.append(smc_res.to_dict())
            babble_runs.append(babble_res.to_dict())
        results["domains"][domain] = {
            "egraph_min_term_size": egraph_min_term_size,
            "runs": {"enum": enum_runs, "smc": smc_runs, "babble": babble_runs},
        }

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
    print(saved.get("title", DEFAULT_TABLE1_TITLE))
    print()
    print(header_top)
    print(header_sub)
    print("-" * len(header_sub))
    for domain in TABLE1_DOMAINS:
        if domain not in domains:
            continue
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
            f"{_fmt(d.get('egraph_min_term_size'), 'd'):>22}  "
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

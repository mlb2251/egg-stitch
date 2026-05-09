"""Table 1 experiment: compare Ours (Enum + SMC) against babble on the four
cogsci drawing domains plus the dreamcoder benchmarks that ship with DSRs.

Runs each method on every domain *with* rewrite rules turned on, stores
results (one :class:`Result` per (method, domain) pair) in a single JSON
under the session results folder, and offers ``print_table1`` to render a
text table matching the paper layout.
"""

import json
import math
import time
from pathlib import Path

import numpy as np

from . import ALL_DOMAINS
from .folders import current_folder_path, set_folder
from .run_models import Babble, OursBf, OursSmc
from .runner import run_method

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


def table1(
    *,
    num_abstractions: int = 1,
    folder_prefix: str = "table1",
    output_name: str = "table1.json",
    title: str = DEFAULT_TABLE1_TITLE,
    enum: OursBf = OursBf(),
    smc: OursSmc = OursSmc(),
    babble: Babble = Babble(),
) -> Path:
    """Run Enum, SMC, and babble on the Table 1 domains with DSRs.

    Each runner is a dataclass instance carrying its own hyperparameters
    (``num_steps``, ``num_particles``, ``temperature``, ``rebuild_egraph``,
    …). Pass overrides as kwargs at construction — e.g. ``smc=OursSmc(
    num_steps=50)`` — rather than mutating module state. Table 3 reuses
    this runner with ``rebuild_egraph=True``.

    ``num_abstractions`` is forwarded to every compressor so each run stacks
    that many abstractions sequentially.
    """
    assert all(d in ALL_DOMAINS for d in TABLE1_DOMAINS), "domain typo"
    set_folder(f"{folder_prefix}/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    results: dict = {
        "title": title,
        "config": {"num_abstractions": num_abstractions},
        "domains": {},
    }

    for domain in TABLE1_DOMAINS:
        print(f"\n=== {domain} ===", flush=True)
        enum_runs, smc_runs, babble_runs = [], [], []
        egraph_min_term_size = None
        for i in range(NUM_RUNS):
            print(f"  run {i+1}/{NUM_RUNS}", flush=True)
            enum_res, enum_egraph_min = run_method(enum, domain, rounds=num_abstractions, use_dsrs=True)
            smc_res, smc_egraph_min = run_method(smc, domain, rounds=num_abstractions, use_dsrs=True)
            # ``cost_after_rewrites`` is a property of the corpus + DSRs, so
            # Enum and SMC must agree on it. NaN==NaN is False, so handle the
            # "no DSRs / not ours" case (both NaN) explicitly.
            assert enum_egraph_min == smc_egraph_min or (math.isnan(enum_egraph_min) and math.isnan(smc_egraph_min)), (
                f"{domain}: e-graph min term size disagrees between algorithms "
                f"(enum={enum_egraph_min}, smc={smc_egraph_min})"
            )
            egraph_min_term_size = enum_egraph_min
            babble_res, _ = run_method(babble, domain, rounds=num_abstractions, use_dsrs=True)
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
    """Format ``x`` with ``spec`` or return ``na`` when ``x`` is None / NaN."""
    if x is None or (isinstance(x, float) and math.isnan(x)):
        return na
    return format(x, spec)


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
            f"{_fmt(d.get('egraph_min_term_size'), '.0f'):>22}  "
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

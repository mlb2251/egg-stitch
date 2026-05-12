"""Table 1 experiment: compare Ours (Enum + SMC) against babble on the four
cogsci drawing domains plus the dreamcoder benchmarks that ship with DSRs.

Runs each method on every domain *with* rewrite rules turned on, stores
results as raw per-file records (``list[PerFileResult]`` per method-repeat)
in a single JSON under ``results/`` (checked into git), and offers
``print_table1`` to render a text table matching the paper layout.
"""

import json
import time
from pathlib import Path

from tqdm import tqdm

from . import ALL_DOMAINS
from .folders import set_folder, summary_results_path
from .render_common import (
    DOMAIN_LABELS,
    aggregate_methods_cr,
    aggregate_methods_time,
    egraph_min_for_domain,
    initial_size_for_domain,
)
from .run_models import Babble, OursBf, OursSmc
from .runner import run_method

NUM_RUNS = 10

# Order matches the Table 1 screenshot, with the dreamcoder benchmarks that
# ship with DSRs appended after the four cogsci drawing domains. text/logo/
# towers are excluded: babble has no equational theory for them, so a "with
# DSRs" comparison is not defined.
TABLE1_DOMAINS = ["nuts-bolts", "dials", "wheels", "furniture", "list", "physics"]


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
    (``num_steps``, ``num_particles``, ``temperature``, …). Pass overrides
    as kwargs at construction — e.g. ``smc=OursSmc(num_steps=50)`` —
    rather than mutating module state.
    """
    assert all(d in ALL_DOMAINS for d in TABLE1_DOMAINS), "domain typo"
    set_folder(f"{folder_prefix}/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    results: dict = {
        "title": title,
        "config": {"num_abstractions": num_abstractions},
        "domains": {},
    }
    runners = (("enum", enum), ("smc", smc), ("babble", babble))

    # One progress bar tick per run_method invocation (one tool × domain × rep).
    total = len(TABLE1_DOMAINS) * NUM_RUNS * len(runners)
    with tqdm(total=total, unit="run", smoothing=0.05) as bar:
        for domain in TABLE1_DOMAINS:
            by_method: dict[str, list[list[dict]]] = {label: [] for label, _ in runners}
            for i in range(NUM_RUNS):
                for label, runner in runners:
                    bar.set_description(f"{domain} {label} rep {i+1}/{NUM_RUNS}")
                    per_file = run_method(
                        runner, domain, rounds=num_abstractions, use_dsrs=True,
                    )
                    by_method[label].append([r.to_dict() for r in per_file])
                    bar.update()
            results["domains"][domain] = {"runs": by_method}

    out_path = summary_results_path(output_name)
    with open(out_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nwrote {out_path}", flush=True)
    print_table1(out_path)
    return out_path


def _fmt(x, spec: str, na: str = "N/A") -> str:
    """Format ``x`` with ``spec`` or return ``na`` when ``x`` is None / NaN."""
    import math
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
        runs = domains[domain].get("runs", {})
        label = DOMAIN_LABELS.get(domain, domain)
        cr = aggregate_methods_cr(runs)
        t = aggregate_methods_time(runs)
        row = (
            f"{label:<14}"
            f"{_fmt(initial_size_for_domain(runs), '.0f'):>14}"
            f"{_fmt(egraph_min_for_domain(runs), '.0f'):>22}  "
            f"{_fmt(cr.get('enum'), '.2f'):>10}"
            f"{_fmt(cr.get('smc'), '.2f'):>10}"
            f"{_fmt(cr.get('babble'), '.2f'):>8}"
            f"{_fmt(cr.get('stitch'), '.2f'):>8}  "
            f"{_fmt(t.get('enum'), '.1f'):>10}"
            f"{_fmt(t.get('smc'), '.1f'):>10}"
            f"{_fmt(t.get('babble'), '.1f'):>8}"
            f"{_fmt(t.get('stitch'), '.1f'):>8}"
        )
        print(row)
    print()

"""Shared run-loop and renderer for the Table 1 / Table 2 family of experiments.

``table1`` and ``table2`` only differ in: which domains they iterate over,
which runners participate, whether DSRs are enabled, the column set in the
printout, and the default title. This module factors out the rest.
"""

from __future__ import annotations

import json
import math
import time
from pathlib import Path
from typing import Sequence

from tqdm import tqdm

from . import ALL_DOMAINS
from .folders import SUMMARY_RESULTS_DIR, set_folder, summary_results_path
from .render_common import (
    DOMAIN_LABELS,
    aggregate_methods_cr,
    aggregate_methods_time,
    egraph_min_for_domain,
    initial_size_for_domain,
)

NUM_RUNS = 10


def _fmt(x, spec: str, na: str = "N/A") -> str:
    """Format ``x`` with ``spec`` or return ``na`` when ``x`` is None / NaN."""
    if x is None or (isinstance(x, float) and math.isnan(x)):
        return na
    return format(x, spec)


def run_table(
    *,
    domains: Sequence[str],
    runners: Sequence[tuple[str, object]],
    num_abstractions: int,
    use_dsrs: bool,
    folder_prefix: str,
    output_name: str,
    title: str,
    show_egraph_min: bool,
) -> Path:
    """Run each ``(label, runner)`` on every domain ``NUM_RUNS`` times, save JSON, print."""
    from .runner import run_method  # local import: runner pulls heavy deps

    assert all(d in ALL_DOMAINS for d in domains), "domain typo"
    set_folder(f"{folder_prefix}/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    results: dict = {
        "title": title,
        "config": {"num_abstractions": num_abstractions},
        "domains": {},
    }
    cache_root = SUMMARY_RESULTS_DIR / Path(output_name).stem

    total = len(domains) * NUM_RUNS * len(runners)
    with tqdm(total=total, unit="run", smoothing=0.05) as bar:
        for domain in domains:
            by_method: dict[str, list[list[dict]]] = {label: [] for label, _ in runners}
            for i in range(NUM_RUNS):
                for label, runner in runners:
                    bar.set_description(f"{domain} {label} rep {i+1}/{NUM_RUNS}")
                    per_file = run_method(
                        runner, domain, rounds=num_abstractions, use_dsrs=use_dsrs,
                        cache_path=cache_root / label / domain / f"rep{i}.json",
                    )
                    by_method[label].append([r.to_dict() for r in per_file])
                    bar.update()
            results["domains"][domain] = {"runs": by_method}

    out_path = summary_results_path(output_name)
    with open(out_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nwrote {out_path}", flush=True)
    print_table(out_path, domains=domains, default_title=title, show_egraph_min=show_egraph_min)
    return out_path


def print_table(
    path: str | Path,
    *,
    domains: Sequence[str],
    default_title: str,
    show_egraph_min: bool,
) -> None:
    """Pretty-print a saved table JSON in the layout from the paper."""
    with open(path) as f:
        saved = json.load(f)
    saved_domains = saved["domains"]

    egraph_col_top = f"{'':>22}" if show_egraph_min else ""
    egraph_col_sub = f"{'E-graph min term size':>22}" if show_egraph_min else ""

    header_top = (
        f"{'':<14}{'':>14}{egraph_col_top}  "
        f"{'Compression Ratio':^36}  {'Time (s)':^36}"
    )
    header_sub = (
        f"{'':<14}{'original size':>14}{egraph_col_sub}  "
        f"{'Enum':>10}{'SMC':>10}{'babble':>8}{'Stitch':>8}  "
        f"{'Enum':>10}{'SMC':>10}{'babble':>8}{'Stitch':>8}"
    )
    print()
    print(saved.get("title", default_title))
    print()
    print(header_top)
    print(header_sub)
    print("-" * len(header_sub))
    for domain in domains:
        if domain not in saved_domains:
            continue
        runs = saved_domains[domain].get("runs", {})
        label = DOMAIN_LABELS.get(domain, domain)
        cr = aggregate_methods_cr(runs)
        t = aggregate_methods_time(runs)
        egraph_col = (
            f"{_fmt(egraph_min_for_domain(runs), '.0f'):>22}" if show_egraph_min else ""
        )
        row = (
            f"{label:<14}"
            f"{_fmt(initial_size_for_domain(runs), '.0f'):>14}"
            f"{egraph_col}  "
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

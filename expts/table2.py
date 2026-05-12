"""Table 2 experiment: Ours (Enum + SMC) vs babble vs Stitch, no DSRs.

Same cogsci domains as Table 1 plus the dreamcoder benchmarks without DSRs
(text/logo/towers); every method runs *without* any domain-specific
rewrites, and Stitch is included (Table 1 uses DSRs, which Stitch doesn't
accept). Results land at ``results/table2.json`` (checked into git).
"""

import json
import time
from pathlib import Path

from tqdm import tqdm

from . import ALL_DOMAINS
from .folders import SUMMARY_RESULTS_DIR, set_folder, summary_results_path
from .render_common import (
    DOMAIN_LABELS,
    aggregate_methods_cr,
    aggregate_methods_time,
    initial_size_for_domain,
)
from .run_models import Babble, OursBf, OursSmc, Stitch
from .runner import run_method
from .table1 import NUM_RUNS

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
    """Run Enum, SMC, babble, and Stitch on the Table 2 domains with no DSRs."""
    assert all(d in ALL_DOMAINS for d in TABLE2_DOMAINS), "domain typo"
    set_folder(f"{folder_prefix}/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    results: dict = {
        "title": title,
        "config": {"num_abstractions": num_abstractions},
        "domains": {},
    }

    runners = (("enum", enum), ("smc", smc), ("babble", babble), ("stitch", stitch))
    cache_root = SUMMARY_RESULTS_DIR / Path(output_name).stem

    total = len(TABLE2_DOMAINS) * NUM_RUNS * len(runners)
    with tqdm(total=total, unit="run", smoothing=0.05) as bar:
        for domain in TABLE2_DOMAINS:
            by_method: dict[str, list[list[dict]]] = {m: [] for m, _ in runners}
            for i in range(NUM_RUNS):
                for label, runner in runners:
                    bar.set_description(f"{domain} {label} rep {i+1}/{NUM_RUNS}")
                    per_file = run_method(
                        runner, domain, rounds=num_abstractions, use_dsrs=False,
                        cache_path=cache_root / label / domain / f"rep{i}.json",
                    )
                    by_method[label].append([r.to_dict() for r in per_file])
                    bar.update()
            results["domains"][domain] = {"runs": by_method}

    out_path = summary_results_path(output_name)
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
        runs = domains[domain].get("runs", {})
        label = DOMAIN_LABELS.get(domain, domain)
        cr = aggregate_methods_cr(runs)
        t = aggregate_methods_time(runs)
        row = (
            f"{label:<14}"
            f"{_fmt(initial_size_for_domain(runs), '.0f'):>14}  "
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

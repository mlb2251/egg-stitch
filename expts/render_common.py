"""Aggregation helpers used by ``scripts/render_tables.py``.

The on-disk JSON shape after the per-file refactor is::

    {
      "config": {...},
      "domains": {
        "<domain>": {
          "runs": {
            "<method>": [               # one entry per repeat
              [perfile, perfile, ...],  # one PerFileResult per input file
              ...
            ]
          }
        }
      }
    }

Cogsci domains have one file per repeat (inner list length 1); dreamcoder
("DC") domains have many. All aggregation — sums for sizes, geomean of
per-file ratios, sums of times within a repeat then geomean across repeats —
happens here so writers can stay dumb.
"""

from __future__ import annotations

import math

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


def _geomean(xs: list[float]) -> float | None:
    """Geometric mean of strictly-positive ``xs``; None if empty."""
    xs = [x for x in xs if x is not None and x > 0 and not math.isnan(x)]
    if not xs:
        return None
    return math.exp(sum(math.log(x) for x in xs) / len(xs))


def repeat_cr(per_file: list[dict]) -> float | None:
    """Geomean of per-file compression ratios for one repeat (Fig. 12, babble paper)."""
    return _geomean([r["compression_ratio"] for r in per_file])


def repeat_time(per_file: list[dict]) -> float:
    """Sum of per-file elapsed seconds for one repeat."""
    return sum(r["elapsed_secs"] for r in per_file)


def aggregate_cr(repeats: list[list[dict]]) -> float | None:
    """Geomean of per-repeat compression ratios (which are themselves per-file geomeans)."""
    return _geomean([cr for cr in (repeat_cr(r) for r in repeats) if cr is not None])


def aggregate_time(repeats: list[list[dict]]) -> float | None:
    """Geomean of per-repeat total elapsed seconds."""
    return _geomean([repeat_time(r) for r in repeats])


def aggregate_methods_cr(runs: dict[str, list[list[dict]]]) -> dict[str, float | None]:
    """{method: aggregated compression ratio} for every method present."""
    return {m: aggregate_cr(repeats) for m, repeats in runs.items()}


def aggregate_methods_time(runs: dict[str, list[list[dict]]]) -> dict[str, float | None]:
    """{method: aggregated elapsed seconds} for every method present."""
    return {m: aggregate_time(repeats) for m, repeats in runs.items()}


def initial_size_for_domain(runs: dict[str, list[list[dict]]]) -> float | None:
    """Geomean ``initial_cost`` per input file (same for every method/repeat).

    Cogsci domains have one file so this is just that file's size; DC
    domains have many files and this is the per-file geomean — matching
    how compression ratios and times are aggregated across files."""
    for repeats in runs.values():
        if repeats and repeats[0]:
            return _geomean([r["initial_cost"] for r in repeats[0]])
    return None


def egraph_min_for_domain(runs: dict[str, list[list[dict]]]) -> float | None:
    """Geomean e-graph-min term size per input file. Uses any repeat whose
    per-file records all have a non-None value; returns None otherwise."""
    for repeats in runs.values():
        for per_file in repeats:
            vals = [r.get("egraph_min_term_size") for r in per_file]
            if vals and all(v is not None for v in vals):
                return _geomean(vals)
    return None

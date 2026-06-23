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


def dnf_status(repeats: list[list[dict]]) -> str:
    """Classify a method's DNF state over all its per-file records.

    Returns ``"none"`` (every run finished), ``"all"`` (every run timed
    out/OOM'd), or ``"partial"`` (a mix). The partial case matters because of
    an averaging asymmetry: ``repeat_cr``/``aggregate_cr`` drop DNFs (geomean
    over the finishers) while ``repeat_time``/``aggregate_time`` still sum in
    the timed-out runs' budget, so a partial set averages CR and time over
    inconsistent sets and silently misleads. Callers therefore report no number
    for a partial method and annotate it ``DNF*`` (vs ``DNF`` for an all-DNF
    method). A record is a DNF when ``compression_ratio is None``."""
    dnf = [r["compression_ratio"] is None for per_file in repeats for r in per_file]
    if not dnf or not any(dnf):
        return "none"
    return "all" if all(dnf) else "partial"


def aggregate_methods_cr(runs: dict[str, list[list[dict]]]) -> dict[str, float | None]:
    """{method: aggregated compression ratio} for every method present.

    Any method with a DNF (partial or all) gets ``None`` — a partial set has no
    comparable number (see ``dnf_status``), and an all-DNF set has nothing to
    average. ``aggregate_methods_dnf`` distinguishes the two for rendering."""
    return {
        m: (aggregate_cr(reps) if dnf_status(reps) == "none" else None)
        for m, reps in runs.items()
    }


def aggregate_methods_time(runs: dict[str, list[list[dict]]]) -> dict[str, float | None]:
    """{method: aggregated elapsed seconds} for every method present.

    ``None`` for any method with a DNF, matching ``aggregate_methods_cr``."""
    return {
        m: (aggregate_time(reps) if dnf_status(reps) == "none" else None)
        for m, reps in runs.items()
    }


def aggregate_methods_dnf(runs: dict[str, list[list[dict]]]) -> dict[str, str]:
    """{method: dnf_status} (``none``/``all``/``partial``) for every method present."""
    return {m: dnf_status(reps) for m, reps in runs.items()}


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
    per-file records all have a non-None value; returns None otherwise.

    Subtracts 1 per file to drop the synthetic ``(programs …)`` root that
    egg-stitch wraps the corpus in; ``initial_size_for_domain`` has no
    such wrapper."""
    for repeats in runs.values():
        for per_file in repeats:
            vals = [r.get("egraph_min_term_size") for r in per_file]
            if vals and all(v is not None for v in vals):
                return _geomean([v - 1 for v in vals])
    return None

#!/usr/bin/env python3
"""Render the ablation experiment (``results/ablation.json``) into LaTeX.

Emits ``figures/ablation.tex`` with two tables — one for the BFS ablations
(wall-clock at the target compression, via ``--compression-limit``) and one for
the SMC ablations (particles needed to reach the target, and that point's
wall-clock) — with one column group per hardest experiment (tables 3/5/7).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
RESULTS_JSON = PROJECT_ROOT / "results" / "ablation.json"
FIGURES_DIR = PROJECT_ROOT / "figures"

# Row order + display labels for each ablation family (keys match expts.ablation).
BFS_ABLATIONS = [
    ("baseline", "Baseline (all on)"),
    ("no-lower-bound", "No lower-bound pruning"),
    ("no-dominance", "No dominance"),
    ("no-equivalence", "No equivalence pruning"),
    ("no-var-ordering", "No variable ordering"),
    ("var-ordering-l2r", "Variable ordering L$\\to$R"),
]
SMC_ABLATIONS = [
    ("baseline", "Baseline"),
    ("no-lower-bound", "No lower-bound pruning"),
    ("no-dominance", "No dominance"),
    ("add-var-ordering", "Add variable ordering"),
]

# Column order across the three hardest experiments.
TABLE_ORDER = ["3", "5", "7"]

# Pretty names for the family/dreamcoder domains that can be picked as hardest.
DOMAIN_LABELS = {
    "list": "List",
    "physics": "Physics",
    "molecules:hexyl": "Hexyl",
    "molecules:ester": "Ester",
    "molecules:glycol": "Glycol",
    "epfl-circuits:log2": "Log2",
    "epfl-circuits:hyp": "Hypotenuse",
    "epfl-circuits:voter": "Voter",
    "epfl-circuits:multiplier": "Multiplier",
    "epfl-circuits:square": "Square",
}


def _domain_label(domain: str) -> str:
    """Human-readable name for a picked hardest domain (falls back to the raw id)."""
    return DOMAIN_LABELS.get(domain, domain.split(":")[-1].replace("-", " ").title())


def _fmt_time(t: float | None) -> str:
    """Seconds with two significant-ish digits, or ``DNF`` when missing."""
    if t is None:
        return "DNF"
    return f"{t:.2f}" if t < 100 else f"{t:.0f}"


def _fmt_steps(s: int | None) -> str:
    """Search-step count with thousands separators, or ``--`` when missing."""
    return "--" if s is None else f"{s:,}"


def _bfs_table(tables: dict) -> list[str]:
    """LaTeX ``tabular`` for the BFS ablations: per experiment, the search work
    (best-first heap pops, deterministic) and wall-clock (s) to reach the target
    compression."""
    cols = [t for t in TABLE_ORDER if t in tables]
    lines = ["% BFS ablations: search steps + time to reach the target compression (--compression-limit)"]
    lines.append("\\begin{tabular}{l" + "".join(" rr" for _ in cols) + "}")
    lines.append("\\toprule")
    heads = " & ".join(
        f"\\multicolumn{{2}}{{c}}{{{_domain_label(tables[t]['domain'])} "
        f"($\\geq${tables[t]['target_cr']:.2f}$\\times$)}}"
        for t in cols
    )
    lines.append("Ablation (BFS) & " + heads + " \\\\")
    lines.append(" & " + " & ".join("Steps & Time (s)" for _ in cols) + " \\\\")
    lines.append("\\midrule")
    for key, label in BFS_ABLATIONS:
        cells = []
        for t in cols:
            cell = tables[t]["bfs"].get(key, {})
            cells.append(f"{_fmt_steps(cell.get('steps'))} & {_fmt_time(cell.get('time'))}")
        lines.append(f"{label} & " + " & ".join(cells) + " \\\\")
    lines.append("\\bottomrule")
    lines.append("\\end{tabular}")
    return lines


def _smc_table(tables: dict) -> list[str]:
    """LaTeX ``tabular`` for the SMC ablations: per experiment, the particle
    count needed to reach the target compression and that point's wall-clock."""
    cols = [t for t in TABLE_ORDER if t in tables]
    lines = ["% SMC ablations: particles needed to reach the target compression, and that point's time"]
    lines.append("\\begin{tabular}{l" + "".join(" rr" for _ in cols) + "}")
    lines.append("\\toprule")
    heads = " & ".join(
        f"\\multicolumn{{2}}{{c}}{{{_domain_label(tables[t]['domain'])} "
        f"($\\geq${tables[t]['target_cr']:.2f}$\\times$)}}"
        for t in cols
    )
    lines.append("Ablation (SMC) & " + heads + " \\\\")
    lines.append(" & " + " & ".join("Particles & Time (s)" for _ in cols) + " \\\\")
    lines.append("\\midrule")
    for key, label in SMC_ABLATIONS:
        cells = []
        for t in cols:
            cell = tables[t]["smc"].get(key, {})
            parts = cell.get("particles")
            parts_s = str(parts) if parts is not None else "--"
            cells.append(f"{parts_s} & {_fmt_time(cell.get('time'))}")
        lines.append(f"{label} & " + " & ".join(cells) + " \\\\")
    lines.append("\\bottomrule")
    lines.append("\\end{tabular}")
    return lines


def main() -> None:
    """Read ``results/ablation.json`` and write ``figures/ablation.tex``."""
    if not RESULTS_JSON.exists():
        sys.exit(f"missing {RESULTS_JSON}; run ./run.py ablation first")
    with open(RESULTS_JSON) as fh:
        tables = json.load(fh)["tables"]
    out = _bfs_table(tables) + ["", ""] + _smc_table(tables)
    FIGURES_DIR.mkdir(parents=True, exist_ok=True)
    dest = FIGURES_DIR / "ablation.tex"
    dest.write_text("\n".join(out) + "\n")
    print(f"wrote {dest}", file=sys.stderr)


if __name__ == "__main__":
    main()

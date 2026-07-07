#!/usr/bin/env python3
"""Render the ablation experiment (``results/ablation.json``) into LaTeX.

Emits ``figures/ablation.tex`` with a single table: one column per ablation and
one row per domain/algorithm combo (the hardest experiment of tables 3/5/7, each
run under BFS and SMC — six rows). Cells hold the wall-clock (s) to reach the
target compression, and are blank when the ablation doesn't apply to that
algorithm.
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
RESULTS_JSON = PROJECT_ROOT / "results" / "ablation.json"
FIGURES_DIR = PROJECT_ROOT / "figures"

# Which ablations apply to each algorithm (keys match expts.ablation).
BFS_KEYS = {"baseline", "no-lower-bound", "no-dominance", "no-equivalence",
            "no-var-ordering", "var-ordering-l2r", "no-forced-expansion"}
SMC_KEYS = {"baseline", "no-lower-bound", "no-dominance", "add-var-ordering"}

# One row per ablation (union across both algorithms), in display order,
# grouped into sections: baseline, the three prunes, the variable-ordering
# knobs, then forced expansion. A ``\midrule`` follows each key in RULE_AFTER.
ABLATION_COLUMNS = [
    ("baseline", "Baseline"),
    ("no-lower-bound", "No lower-bound pruning"),
    ("no-dominance", "No dominance"),
    ("no-equivalence", "No equivalence pruning"),
    ("add-var-ordering", "Add variable ordering"),
    ("no-var-ordering", "No variable ordering"),
    ("var-ordering-l2r", "Variable ordering L$\\to$R"),
    ("no-forced-expansion", "No forced expansion"),
]
RULE_AFTER = {"baseline", "no-equivalence", "var-ordering-l2r"}

# Column order across the three hardest experiments (one domain per table).
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
    """Seconds with at least two significant figures and at least one decimal
    place, or ``DNF`` when the ablation ran but never reached the target."""
    if t is None:
        return "DNF"
    if t <= 0:
        return "0.0"
    # Decimals for two sig figs (2-1-exponent), floored at one decimal place.
    decimals = max(1 - math.floor(math.log10(t)), 1)
    return f"{t:.{decimals}f}"


def _ablation_table(tables: dict) -> list[str]:
    """LaTeX ``tabular``: one row per ablation, one column per domain/algorithm
    combo, holding the wall-clock (s) to reach the target compression. Cells are
    blank where the ablation doesn't apply to that algorithm."""
    applies = {"bfs": BFS_KEYS, "smc": SMC_KEYS}
    # The 6 (domain, algorithm) columns, grouped two-per-domain.
    cols = [(t, algo, disp) for t in TABLE_ORDER if t in tables
            for algo, disp in (("bfs", "BFS"), ("smc", "SMC"))]
    lines = ["% Ablation wall-clock (s) to reach the target compression: one row per ablation, one column per domain/algorithm."]
    lines.append("\\begin{tabular}{l" + "r" * len(cols) + "}")
    lines.append("\\toprule")
    groups = " & ".join(
        f"\\multicolumn{{2}}{{c}}{{{_domain_label(tables[t]['domain'])}}}"
        for t in TABLE_ORDER if t in tables
    )
    lines.append("Ablation & " + groups + " \\\\")
    lines.append(" & " + " & ".join(disp for _, _, disp in cols) + " \\\\")
    lines.append("\\midrule")
    for key, label in ABLATION_COLUMNS:
        cells = []
        for t, algo, _ in cols:
            if key not in applies[algo]:
                cells.append("")  # ablation doesn't apply to this algorithm
                continue
            cell = tables[t][algo].get(key, {})
            cells.append(_fmt_time(cell.get("time")))
        lines.append(f"{label} & " + " & ".join(cells) + " \\\\")
        if key in RULE_AFTER:
            lines.append("\\midrule")
    lines.append("\\bottomrule")
    lines.append("\\end{tabular}")
    return lines


def main() -> None:
    """Read ``results/ablation.json`` and write ``figures/ablation.tex``."""
    if not RESULTS_JSON.exists():
        sys.exit(f"missing {RESULTS_JSON}; run ./run.py ablation first")
    with open(RESULTS_JSON) as fh:
        tables = json.load(fh)["tables"]
    out = _ablation_table(tables)
    FIGURES_DIR.mkdir(parents=True, exist_ok=True)
    dest = FIGURES_DIR / "ablation.tex"
    dest.write_text("\n".join(out) + "\n")
    print(f"wrote {dest}", file=sys.stderr)


if __name__ == "__main__":
    main()

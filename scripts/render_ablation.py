#!/usr/bin/env python3
"""Render the ablation experiment (``results/ablation.json``) into LaTeX.

Emits two identically-structured tables — one row per ablation, one column per
domain/algorithm combo (the hardest experiment of tables 3/5/7, each run under
BFS and SMC — six columns, grouped two-per-domain under a centred header):

* ``figures/ablation.tex`` — the wall-clock (s) to reach the target compression.
* ``figures/ablation-appendix.tex`` — the same, with the search work (BFS heap
  pops / SMC particles) in parentheses; for an appendix.

Cells are blank where the ablation doesn't apply to that algorithm.
"""

from __future__ import annotations

import json
import math
import sys
from pathlib import Path
from typing import Callable

PROJECT_ROOT = Path(__file__).resolve().parent.parent
RESULTS_JSON = PROJECT_ROOT / "results" / "ablation.json"
FIGURES_DIR = PROJECT_ROOT / "figures"

# Which ablations apply to each algorithm (keys match expts.ablation). BFS
# defaults lower-bound pruning on (so it can be *removed*); SMC defaults it off
# (so it can only be *added*) — mirroring the variable-ordering pair.
BFS_KEYS = {"baseline", "no-lower-bound", "no-dominance", "no-equivalence",
            "no-var-ordering", "var-ordering-l2r"}
SMC_KEYS = {"baseline", "add-lower-bound", "no-dominance", "add-var-ordering"}

# One row per ablation (union across both algorithms), in display order,
# grouped into sections: baseline, the prunes (lower-bound both directions,
# dominance, equivalence), then the variable-ordering knobs. A ``\midrule``
# follows each key in RULE_AFTER.
ABLATION_COLUMNS = [
    ("baseline", "Baseline"),
    ("no-lower-bound", "No lower-bound pruning"),
    ("add-lower-bound", "Add lower-bound pruning"),
    ("no-dominance", "No dominance"),
    ("no-equivalence", "No equivalence pruning"),
    ("add-var-ordering", "Add variable ordering"),
    ("no-var-ordering", "No variable ordering"),
    ("var-ordering-l2r", "Variable ordering L$\\to$R"),
]
RULE_AFTER = {"baseline", "no-equivalence"}

# Column order across the three hardest experiments (one domain per table).
TABLE_ORDER = ["3", "5", "7"]

# Each BFS/SMC data column is a fixed-width centred column (needs the `array`
# package). Equal widths keep a domain's two columns balanced, so its centred
# \multicolumn header lines up with the BFS/SMC sub-headers and data below it.
# The width must exceed the widest cell (e.g. ``0.025``) in the target font — a
# too-narrow column lets wide cells overflow and unbalances the pair, drifting
# the header off centre (visible under acmart/Libertine but not Computer Modern).
CELL_COL = ">{\\centering\\arraybackslash}p{3.4em}"
# Inter-group separation is an empty fixed-width *column* before each domain,
# NOT an ``@{\hspace{...}}`` gap: an @-gap deletes the surrounding \tabcolsep and
# leaks into the neighbouring \multicolumn's box, pushing the domain header ~half
# a gap off-centre (confirmed by pixel-measuring the acmart build). A real column
# keeps normal \tabcolsep boundaries, so the header stays centred over its pair.
SPACER_COL = ">{\\centering\\arraybackslash}p{1.5em}"
# The appendix table appends the search work (e.g. ``0.88 (34,997)``), so its
# data columns must be wider to fit the count without overflowing (an overflow
# would unbalance the pair and drift the header off centre).
APPENDIX_CELL_COL = ">{\\centering\\arraybackslash}p{7em}"

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


def _fmt_count(n: int | None) -> str:
    """A search-work count (BFS steps / SMC particles) with thousands separators."""
    return "--" if n is None else f"{n:,}"


def _time_only(cell: dict, algo: str) -> str:
    """The wall-clock alone (main table)."""
    return _fmt_time(cell.get("time"))


def _time_and_count(cell: dict, algo: str) -> str:
    """Wall-clock plus, in parentheses, the search work at that point — BFS heap
    pops (``steps``) or the SMC particle count (appendix table). ``DNF`` cells
    have no count to show."""
    t = cell.get("time")
    if t is None:
        return _fmt_time(t)  # DNF
    count = cell.get("steps") if algo == "bfs" else cell.get("particles")
    return f"{_fmt_time(t)} ({_fmt_count(count)})"


def _ablation_table(tables: dict, fmt: Callable[[dict, str], str], cell_col: str,
                    caption: str) -> list[str]:
    """LaTeX ``tabular``: one row per ablation, one column per domain/algorithm
    combo. Each cell is rendered by ``fmt(cell, algo)`` and set in a ``cell_col``
    (fixed-width centred) column. Cells are blank where the ablation doesn't
    apply to that algorithm. ``caption`` is the leading ``%`` comment."""
    applies = {"bfs": BFS_KEYS, "smc": SMC_KEYS}
    present = [t for t in TABLE_ORDER if t in tables]
    # Columns: label, then per domain a spacer column + the BFS/SMC pair. The
    # spacer separates groups without an @-gap that would off-centre the header.
    colspec = "l" + f" {SPACER_COL} {cell_col}{cell_col}" * len(present)

    def row(label: str, cell: Callable[[str, str], str]) -> str:
        """One table row: the ``label``, then for each domain an empty spacer
        cell followed by its BFS and SMC cells (via ``cell(table, algo)``)."""
        parts = [label]
        for t in present:
            parts += ["", cell(t, "bfs"), cell(t, "smc")]  # "" = spacer column
        return " & ".join(parts) + " \\\\"

    lines = [f"% {caption}"]
    lines.append("% Requires \\usepackage{array} (centred fixed-width p-columns) and \\usepackage{booktabs}.")
    lines.append("\\begin{tabular}{" + colspec + "}")
    lines.append("\\toprule")
    # Domain header: an empty spacer cell then a \multicolumn spanning the pair.
    header = "Ablation" + "".join(
        f" & & \\multicolumn{{2}}{{c}}{{{_domain_label(tables[t]['domain'])}}}"
        for t in present
    )
    lines.append(header + " \\\\")
    lines.append(row("", lambda t, algo: "BFS" if algo == "bfs" else "SMC"))
    lines.append("\\midrule")
    for key, label in ABLATION_COLUMNS:
        def cell(t: str, algo: str, _key=key) -> str:
            if _key not in applies[algo]:
                return ""  # ablation doesn't apply to this algorithm
            return fmt(tables[t][algo].get(_key, {}), algo)

        lines.append(row(label, cell))
        if key in RULE_AFTER:
            lines.append("\\midrule")
    lines.append("\\bottomrule")
    lines.append("\\end{tabular}")
    return lines


def main() -> None:
    """Read ``results/ablation.json`` and write the ablation tables: the main
    ``figures/ablation.tex`` (wall-clock only) and the appendix
    ``figures/ablation-appendix.tex`` (wall-clock with BFS steps / SMC particles
    in parentheses), structured identically."""
    if not RESULTS_JSON.exists():
        sys.exit(f"missing {RESULTS_JSON}; run ./run.py ablation first")
    with open(RESULTS_JSON) as fh:
        tables = json.load(fh)["tables"]
    FIGURES_DIR.mkdir(parents=True, exist_ok=True)
    outputs = {
        "ablation.tex": _ablation_table(
            tables, _time_only, CELL_COL,
            "Ablation wall-clock (s) to reach the target compression: "
            "one row per ablation, one column per domain/algorithm."),
        "ablation-appendix.tex": _ablation_table(
            tables, _time_and_count, APPENDIX_CELL_COL,
            "Ablation wall-clock (s) with BFS steps / SMC particles in parentheses: "
            "one row per ablation, one column per domain/algorithm."),
    }
    for name, lines in outputs.items():
        dest = FIGURES_DIR / name
        dest.write_text("\n".join(lines) + "\n")
        print(f"wrote {dest}", file=sys.stderr)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Render the molecule scramble results as one figure per family.

Reads ``results/table5.json`` (the molecule-subset table; see
``expts/tables.py``). table5 records two best-first conditions whose
per-iteration cost trajectory this renderer draws:

  ``enum-dsrs-at-start`` -- DSRs canonicalise the egraph once, then search
                            runs rule-free (the "Stitch on E-graph min term"
                            baseline);
  ``smc-1000``           -- DSRs kept live during search (E-Stitch), with the
                            SMC sampler at the canonical 1000-particle point.

Each family gets its own figure, with compression ratio on the y axis and
step number on the x axis. Step 0 is the normalized starting point
``(0, 1)``; steps 1-4 use the first four learned library functions. Each
point is annotated with the corresponding learned library entry, and the
final plotted point gets a wall-clock time callout (``elapsed_secs``).
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
import numpy as np

PROJECT_ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(PROJECT_ROOT))
from expts.tables import TABLE5_DOMAINS, TABLE5_SMC_POINT  # noqa: E402

RESULTS_PATH = PROJECT_ROOT / "results" / "table5.json"
DEFAULT_OUT_DIR = PROJECT_ROOT / "figures" / "molecules"
DEFAULT_MAX_STEP = 4

# Logical method name -> the table5 method key holding its run data.
METHOD_DATA_KEY = {
    "DSR-canon": "enum-dsrs-at-start",
    "search-DSR": f"smc-{TABLE5_SMC_POINT}",
}
METHODS = ("DSR-canon", "search-DSR")
METHOD_COLORS = {
    "DSR-canon": "#6300CC",
    "search-DSR": "#d55e00",
}
METHOD_MARKERS = {
    "DSR-canon": "o",
    "search-DSR": "s",
}

METHOD_COMMON_NAME = {
    "DSR-canon": "Stitch on E-Graph min term",
    "search-DSR": "E-Stitch",
}

# Font sizes (points) for the combined 2x2 figure. Kept deliberately large
# relative to the shrunken per-panel size so that, once the whole grid is
# placed at \textwidth in the paper, the text reads clearly.
TITLE_FONTSIZE = 14
LABEL_FONTSIZE = 13
TICK_FONTSIZE = 11
LEGEND_FONTSIZE = 14
TIME_FONTSIZE = 9

# RDKit depiction: keep bond length constant (in px) across molecules so every
# diagram renders at the same scale, then autocrop and place at a single zoom.
BOND_LENGTH_PX = 26
DIAGRAM_ZOOM = 0.28


@dataclass(frozen=True)
class ExprNode:
    """Parsed rooted expression used to build a compact diagram."""

    head: str
    label: str
    children: tuple["ExprNode", ...]

    def subst(self, mapping: dict[str, "ExprNode"]) -> "ExprNode":
        """Substitute according to the mapping, replacing any matching leaf."""
        children = tuple(child.subst(mapping) for child in self.children)
        if self.head in mapping:
            if not self.children:
                return mapping[self.head]
            return mapping[self.head].subst(
                {f"?#{i}": child for i, child in enumerate(children)}
            )
        return ExprNode(
            head=self.head,
            label=self.label,
            children=children,
        )

    def collect_labels(self) -> list[str]:
        """Return a list of all labels in the expression tree."""
        labels = [self.label]
        for child in self.children:
            labels.extend(child.collect_labels())
        return labels

    def relabel(self, mapping: dict[str, str]) -> "ExprNode":
        """Return a new tree with each ``label`` remapped via ``mapping``."""
        return ExprNode(
            head=self.head,
            label=mapping.get(self.label, self.label),
            children=tuple(child.relabel(mapping) for child in self.children),
        )

    def normalize_labels(self) -> "ExprNode":
        """Return a new expression tree with normalized labels."""
        # Distinct R labels in first-seen (tree traversal) order.
        r_labels = list(
            dict.fromkeys(
                label for label in self.collect_labels() if label.startswith("R")
            )
        )
        assert len(r_labels) <= 2, "too many R labels to normalize"
        if len(r_labels) == 0:
            return self
        if len(r_labels) == 1:
            remap = {r_labels[0]: "R"}
        else:
            remap = {label: f"R{i+1}" for i, label in enumerate(r_labels)}
        return self.relabel(remap)


def compression_ratio(initial_cost: float, cost: float) -> float:
    """Return the compression ratio ``initial_cost / cost``."""
    return initial_cost / cost


def load_results(path: Path) -> dict:
    """Load and return the scramble results JSON from ``path``."""
    with open(path) as fh:
        return json.load(fh)


def tokenize_expr(expr: str) -> list[str]:
    """Split an expression into parentheses and atom-like tokens."""
    return re.findall(r"\(|\)|[^\s()]+", expr)


def display_label(label: str) -> str:
    """Normalize variables into molecule-style attachment labels."""
    if label.startswith("?#"):
        return "R" + str(int(label.removeprefix("?#")) + 1)
    return label


def parse_expr(tokens: list[str], pos: int = 0) -> tuple[ExprNode, int]:
    """Parse a rooted list expression into nested ``ExprNode`` values."""
    token = tokens[pos]
    if token != "(":
        return ExprNode(head=token, label=display_label(token), children=()), pos + 1

    head = tokens[pos + 1]
    if re.fullmatch(r"[msd]\d+", head):
        label = display_label(tokens[pos + 2])
        children = []
        pos += 3
    else:
        assert re.fullmatch(r"fn_\d+", head), f"unexpected expression head: {head}"
        label = display_label(head)
        children = []
        pos += 2
    while tokens[pos] != ")":
        child, pos = parse_expr(tokens, pos)
        children.append(child)
    return ExprNode(head=head, label=label, children=tuple(children)), pos + 1


def parse_library_expr(entry: str) -> ExprNode:
    """Parse one learned library entry into an expression tree."""
    _, _, body = entry.partition(":")
    tokens = tokenize_expr(body.strip())
    node, end = parse_expr(tokens)
    if end != len(tokens):
        raise ValueError(f"unparsed tokens in library expression: {entry}")
    return node


def bond_order(label: str) -> int:
    """Infer a bond order from an expression label."""
    if label.startswith("d"):
        return 2
    return 1


def with_implicit_root(node: ExprNode) -> ExprNode:
    """Add a dummy attachment root for non-``m`` top-level expressions."""
    if node.head.startswith("m"):
        return node
    return ExprNode(head="m1", label="R", children=(node,))


def build_rdkit_mol(node: ExprNode):
    """Build an RDKit molecule from a parsed expression tree.

    Each ``ExprNode`` becomes an atom (``R*`` labels become dummy attachment
    atoms), and the parent link becomes a single or double bond according to the
    child head (``d`` = double). Explicit ``H`` atoms are added so valences are
    complete; the depiction later collapses them into the skeletal drawing.
    """
    from rdkit import Chem

    mol = Chem.RWMol()

    def add(current: ExprNode, parent_idx: int | None, order: int) -> int:
        label = current.label
        if label.startswith("R"):
            atom = Chem.Atom(0)  # dummy attachment point
            atom.SetProp("atomLabel", label)
        elif label == "H":
            atom = Chem.Atom(1)
            atom.SetNoImplicit(True)
        else:
            atom = Chem.Atom(label)
        idx = mol.AddAtom(atom)
        if parent_idx is not None:
            bond = Chem.BondType.DOUBLE if order == 2 else Chem.BondType.SINGLE
            mol.AddBond(parent_idx, idx, bond)
        for child in current.children:
            add(child, idx, bond_order(child.head))
        return idx

    add(node, None, 1)
    return mol


def autocrop(image: np.ndarray, margin: int = 2) -> np.ndarray:
    """Trim fully-transparent borders from an RGBA image, keeping a small margin."""
    opaque = np.where(image[:, :, 3] > 0)
    if opaque[0].size == 0:
        return image
    y0, y1 = opaque[0].min(), opaque[0].max()
    x0, x1 = opaque[1].min(), opaque[1].max()
    h, w = image.shape[:2]
    y0, x0 = max(y0 - margin, 0), max(x0 - margin, 0)
    y1, x1 = min(y1 + margin, h - 1), min(x1 + margin, w - 1)
    return image[y0 : y1 + 1, x0 : x1 + 1]


def render_molecule_diagram(entry: ExprNode):
    """Build a skeletal molecular diagram for a learned library entry.

    Returns a matplotlib ``OffsetImage`` of the RDKit depiction, cropped to its
    content and scaled at a shared zoom so every diagram uses the same bond
    length regardless of molecule size.
    """
    from io import BytesIO
    import matplotlib.image as mpimg
    from matplotlib.offsetbox import OffsetImage
    from rdkit import Chem
    from rdkit.Chem import AllChem
    from rdkit.Chem.Draw import rdMolDraw2D

    node = with_implicit_root(entry).normalize_labels()
    mol = build_rdkit_mol(node).GetMol()
    Chem.SanitizeMol(mol)
    mol = Chem.RemoveHs(mol)  # collapse to skeletal form (implicit H on carbons)
    AllChem.Compute2DCoords(mol)

    drawer = rdMolDraw2D.MolDraw2DCairo(600, 600)
    opts = drawer.drawOptions()
    opts.clearBackground = False  # transparent background
    opts.fixedBondLength = BOND_LENGTH_PX
    opts.bondLineWidth = 2
    rdMolDraw2D.PrepareAndDrawMolecule(drawer, mol)
    drawer.FinishDrawing()

    image = autocrop(mpimg.imread(BytesIO(drawer.GetDrawingText()), format="png"))
    return OffsetImage(image, zoom=DIAGRAM_ZOOM)


def annotate_diagram(ax, x: float, y: float, entry: ExprNode, offset_y: int) -> None:
    """Attach a compact molecular diagram to a data point."""
    from matplotlib.offsetbox import AnnotationBbox

    ax.add_artist(
        AnnotationBbox(
            render_molecule_diagram(entry),
            (x, y),
            xybox=(0, offset_y),
            xycoords="data",
            boxcoords="offset points",
            frameon=False,
            box_alignment=(0.5, 0.5),
            pad=0.0,
            zorder=5,
        )
    )


def perfile_record(saved: dict, domain: str, method: str) -> dict | None:
    """Extract the single per-file result dict for a (family, method) from
    table5's nested ``domains -> runs -> [reps][files]`` shape.

    Uses the first repeat (a representative run). Returns None if the method is
    absent or its first repeat has no files (e.g. a tool that didn't run here).
    """
    runs = saved["domains"][domain].get("runs", {})
    reps = runs.get(METHOD_DATA_KEY[method])
    if not reps or not reps[0]:
        return None
    return reps[0][0]  # one file per molecule family


def step_points(
    perfile: dict, max_step: int
) -> tuple[list[int], list[float], list[ExprNode | str], float]:
    """Return x/y points, labels, and the wall-clock time for one method.

    Step 0 is the normalized starting point ``(0, 1)``. Step ``n`` uses the
    ``n - 1``-th entry from ``cost_at_end_of_each_iter`` and ``library``. The
    time callout uses the run's total ``elapsed_secs`` (the renderer only ever
    showed the time after the final iteration).
    """
    initial_cost = perfile["initial_cost"]
    costs = perfile["cost_at_end_of_each_iter"]
    library = perfile["library"]

    limit = min(max_step, len(costs), len(library))
    xs = [0]
    ys = [1.0]
    labels = [""]
    for step in range(1, limit + 1):
        xs.append(step)
        ys.append(compression_ratio(initial_cost, costs[step - 1]))
        labels.append(library[step - 1])
    return xs, ys, unroll_labels(labels), perfile["elapsed_secs"]


def unroll_labels(labels: list[str]) -> list[ExprNode | str]:
    """Expand each ``name: body`` label into a fully-substituted expression.

    Labels that don't name a library entry (e.g. the empty step-0 label) are
    returned unchanged as strings.
    """
    label_map = {}
    for label in labels:
        if ": " in label:
            name, _ = label.split(": ")
            label_map[name] = parse_library_expr(label)
    for label in label_map:
        label_map[label] = label_map[label].subst(label_map)
    exprs = [label_map.get(label.split(":")[0], label) for label in labels]
    return exprs


def annotate_series(
    ax,
    xs: list[int],
    ys: list[float],
    labels: list[str],
    color: str,
    time_value: float,
    *,
    median_ys: list[float],
) -> None:
    """Annotate a series with molecular diagrams and a final time callout."""

    for step, (x, y, label, median_y) in enumerate(zip(xs, ys, labels, median_ys)):
        above_median = y > median_y

        if step == 0:
            continue

        if label:
            offset = 18
            yoff = offset if above_median else -offset
            annotate_diagram(ax, x, y, label, yoff)

    ax.annotate(
        f"t={time_value:.2f}s",
        xy=(xs[-1], ys[-1]),
        xytext=(8, 0),
        textcoords="offset points",
        ha="left",
        va="center",
        fontsize=TIME_FONTSIZE,
        color=color,
        bbox={"facecolor": "white", "alpha": 0.82, "edgecolor": "none", "pad": 0.2},
    )


def plot_domain(ax, saved: dict, domain: str, max_step: int) -> tuple[list, list]:
    """Plot one family's trajectory onto ``ax``.

    Returns the legend ``(handles, labels)`` for the series drawn, so that a
    caller can render a single shared legend elsewhere (e.g. in a spare panel of
    a 2x2 grid).
    """
    methods, results = [], []
    for method in METHODS:
        perfile = perfile_record(saved, domain, method)
        if perfile is None or perfile.get("cost_at_end_of_each_iter") is None:
            continue  # method absent or has no per-iteration trajectory here
        methods.append(method)
        results.append(step_points(perfile, max_step))

    median_ys = np.median([ys for _, ys, _, _ in results], axis=0)

    handles, labels = [], []
    for method, (xs, ys, point_labels, last_time) in zip(methods, results):
        (line,) = ax.plot(
            xs,
            ys,
            color=METHOD_COLORS[method],
            marker=METHOD_MARKERS[method],
            linewidth=2.0,
            markersize=6,
            label=METHOD_COMMON_NAME[method],
        )
        handles.append(line)
        labels.append(METHOD_COMMON_NAME[method])
        annotate_series(
            ax, xs, ys, point_labels, METHOD_COLORS[method], last_time,
            median_ys=median_ys,
        )

    ax.set_xlim(-0.2, max_step + 1.4)
    ax.set_xticks(list(range(max_step + 1)))
    ax.tick_params(axis="both", labelsize=TICK_FONTSIZE)
    ax.tick_params(axis="x", labelbottom=True)
    ax.set_xlabel("Step", fontsize=LABEL_FONTSIZE)
    ax.set_ylabel("Compression", fontsize=LABEL_FONTSIZE)
    ax.grid(True, which="both", linewidth=0.35, alpha=0.35)
    from matplotlib.ticker import MultipleLocator

    lo, hi = ax.get_ylim()
    r = hi - lo
    ax.set_ylim(lo - r * 0.08, hi + r * 0.20)
    ax.yaxis.set_major_locator(MultipleLocator(1))

    family = domain.split(":", 1)[1] if ":" in domain else domain
    ax.set_title(family.capitalize(), fontsize=TITLE_FONTSIZE)
    return handles, labels


def render_combined(saved: dict, out_path: Path, max_step: int) -> None:
    """Render all families as a 2x2 grid, with a shared legend in the spare panel.

    The figure is deliberately compact in inches and saved at a high DPI: this
    keeps the point-sized text large relative to each panel, so that the grid
    stays legible when placed at \\textwidth in the paper.
    """
    import matplotlib.pyplot as plt

    domains = [d for d in TABLE5_DOMAINS if d in saved["domains"]]

    fig, axes = plt.subplots(
        2, 2, figsize=(8.0, 4.8), constrained_layout=True
    )
    fig.get_layout_engine().set(h_pad=0.02, hspace=0.02)
    flat = list(axes.flatten())

    legend_handles: list = []
    legend_labels: list = []
    for ax, domain in zip(flat, domains):
        handles, labels = plot_domain(ax, saved, domain, max_step)
        if len(labels) > len(legend_labels):
            legend_handles, legend_labels = handles, labels

    # Any panel not used by a family becomes the legend panel; the last spare
    # one carries the shared legend, and the rest (if any) are hidden.
    for ax in flat[len(domains):]:
        ax.axis("off")
    if len(flat) > len(domains):
        flat[-1].legend(
            legend_handles,
            legend_labels,
            loc="center",
            frameon=True,
            fontsize=LEGEND_FONTSIZE,
        )

    fig.savefig(out_path, dpi=400)
    plt.close(fig)


def render_all(saved: dict, out_dir: Path, max_step: int = DEFAULT_MAX_STEP) -> None:
    """Render the combined 2x2 scramble figure into ``out_dir``."""
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / "search-progress.png"
    render_combined(saved, out_path, max_step)
    print(f"wrote {out_path}", file=sys.stderr)


def main() -> None:
    """Parse CLI arguments and render one graph per domain."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--input", type=Path, default=RESULTS_PATH, help="Path to table5.json"
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUT_DIR,
        help="Output directory for the rendered graphs",
    )
    parser.add_argument(
        "--max-step", type=int, default=DEFAULT_MAX_STEP,
        help="Maximum step index to plot, inclusive",
    )
    args = parser.parse_args()

    saved = load_results(args.input)
    render_all(saved, args.output, args.max_step)


if __name__ == "__main__":
    main()

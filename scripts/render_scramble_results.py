#!/usr/bin/env python3
"""Render scramble experiment results as one subplot per domain.

The input JSON is expected to have the shape written by
``scripts/molecules/run_scramble_experiment.py``:

``{domain: {method: {initial_cost, cost_at_end_of_each_iter, library,
iteration_times}}}``

This renderer plots only the ``DSR-canon`` and ``search-DSR`` methods.
Each domain gets its own subplot, with compression ratio on the y axis and
step number on the x axis. Step 0 is the normalized starting point
``(0, 1)``; steps 1-4 use the first four learned library functions. Each
point is annotated with the corresponding learned library entry, and the
final plotted point gets a time callout.
"""

from __future__ import annotations

import argparse
import json
import math
import re
from dataclasses import dataclass
from pathlib import Path
import numpy as np

PROJECT_ROOT = Path(__file__).resolve().parent.parent
RESULTS_PATH = PROJECT_ROOT / "results" / "scramble_results.json"
DEFAULT_OUT_PATH = PROJECT_ROOT / "figures" / "scramble_results.png"

METHODS = ("DSR-canon", "search-DSR")
METHOD_COLORS = {
    "DSR-canon": "#6300CC",
    "search-DSR": "#d55e00",
}
METHOD_MARKERS = {
    "DSR-canon": "o",
    "search-DSR": "s",
}

METOD_COMMON_NAME = {
    "DSR-canon": "Stitch on E-Graph min term",
    "search-DSR": "E-Stitch",
}

ATOM_COLORS = {
    "C": "#000000",
    "H": "#555555",
    "O": "#c81e1e",
    "R": "#00823c",
    "R1": "#00823c",
    "R2": "#00823c",
}


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
        assert len(r_labels) <= 3, "too many R labels to normalize"
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
    if label.startswith("fn_"):
        return "V" + label.removeprefix("fn_")
    if label.startswith("?#"):
        return "R" + str(int(label.removeprefix("?#")) + 1)
    return label


def parse_expr(tokens: list[str], pos: int = 0) -> tuple[ExprNode, int]:
    """Parse a rooted list expression into nested ``ExprNode`` values."""
    token = tokens[pos]
    if token != "(":
        return ExprNode(head=token, label=display_label(token), children=()), pos + 1

    head = tokens[pos + 1]
    if re.fullmatch(r"[msdt]\d+", head):
        label = display_label(tokens[pos + 2])
        children = []
        pos += 3
    else:
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
    if label.startswith("t"):
        return 3
    return 1


def with_implicit_root(node: ExprNode) -> ExprNode:
    """Add a dummy attachment root for non-``m`` top-level expressions."""
    if node.head.startswith("m"):
        return node
    return ExprNode(head="m1", label="R", children=(node,))


def layout_tree(
    node: ExprNode,
) -> tuple[dict[int, tuple[float, float]], dict[int, tuple[int, int]], list[int]]:
    """Lay out a rooted tree using leaf order on x and depth on y."""
    positions: dict[int, tuple[float, float]] = {}
    edges: dict[int, tuple[int, int]] = {}
    order: list[int] = []
    leaf_x = 0

    def walk(current: ExprNode, depth: int, parent: ExprNode | None) -> float:
        nonlocal leaf_x
        node_id = id(current)
        order.append(node_id)
        if not current.children:
            x = float(leaf_x)
            leaf_x += 1
        else:
            xs = [walk(child, depth + 1, current) for child in current.children]
            x = sum(xs) / len(xs)
        positions[node_id] = (x, -float(depth))
        if parent is not None:
            edges[node_id] = (id(parent), bond_order(current.head))
        return x

    walk(node, 0, None)
    return positions, edges, order


def normalize_positions(
    positions: dict[int, tuple[float, float]],
) -> dict[int, tuple[float, float]]:
    """Scale a layout into the unit box."""
    xs = [p[0] for p in positions.values()]
    ys = [p[1] for p in positions.values()]
    min_x, max_x = min(xs), max(xs)
    min_y, max_y = min(ys), max(ys)
    x_span = max_x - min_x
    y_span = max_y - min_y

    def place_within(value: float, min_val: float, span: float) -> float:
        if span > 0:
            return (value - min_val) / span
        else:
            return 0.5

    return {
        node_id: (
            place_within(x, min_x, x_span),
            place_within(y, min_y, y_span),
        )
        for node_id, (x, y) in positions.items()
    }


def add_bond_segments(
    drawer, p1: tuple[float, float], p2: tuple[float, float], order: int
) -> None:
    """Draw one, two, or three parallel bond segments."""
    from matplotlib.lines import Line2D

    x1, y1 = p1
    x2, y2 = p2
    dx = x2 - x1
    dy = y2 - y1
    length = math.hypot(dx, dy) or 1.0
    px = -dy / length
    py = dx / length
    offsets = {1: [0.0], 2: [-0.06, 0.06], 3: [-0.09, 0.0, 0.09]}.get(order, [0.0])
    for offset in offsets:
        drawer.add_artist(
            Line2D(
                [x1 + px * offset, x2 + px * offset],
                [y1 + py * offset, y2 + py * offset],
                color="black",
                linewidth=1.0 if order == 1 else 0.9,
                solid_capstyle="round",
            )
        )


def render_molecule_diagram(entry: ExprNode):
    """Build a compact molecular diagram for a learned library entry."""
    from matplotlib.offsetbox import DrawingArea
    from matplotlib.patches import Circle
    from matplotlib.text import Text

    node = with_implicit_root(entry).normalize_labels()
    positions, edges, order = layout_tree(node)
    positions = normalize_positions(positions)

    width = 74
    height = 52
    pad_x = 7
    pad_y = 6
    scale_x = width - 2 * pad_x
    scale_y = height - 2 * pad_y
    drawer = DrawingArea(width, height, 0, 0)

    for child_id, (parent_id, bond) in edges.items():
        add_bond_segments(
            drawer,
            (
                pad_x + positions[parent_id][0] * scale_x,
                pad_y + positions[parent_id][1] * scale_y,
            ),
            (
                pad_x + positions[child_id][0] * scale_x,
                pad_y + positions[child_id][1] * scale_y,
            ),
            bond,
        )

    for node_id in order:
        current = None
        stack = [node]
        while stack:
            candidate = stack.pop()
            if id(candidate) == node_id:
                current = candidate
                break
            stack.extend(candidate.children)
        if current is None:
            continue

        x, y = positions[node_id]
        x = pad_x + x * scale_x
        y = pad_y + y * scale_y
        atom_color = ATOM_COLORS[current.label]

        gray = sum(int(atom_color[i : i + 2], 16) for i in (1, 3, 5)) / 3
        if gray < 128:
            text = "white"
        else:
            text = "black"
        radius = 4
        drawer.add_artist(
            Circle(
                (x, y),
                radius=radius,
                facecolor=atom_color,
                linewidth=1.0,
            )
        )
        drawer.add_artist(
            Text(
                x,
                y,
                current.label,
                fontsize=5.8,
                ha="center",
                va="center",
                color=text,
            )
        )

    return drawer


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


def step_points(
    payload: dict, max_step: int
) -> tuple[list[int], list[float], list[str], float]:
    """Return x/y points, labels, and the last plotted iteration time.

    Step 0 is the normalized starting point ``(0, 1)``. Step ``n`` uses the
    ``n - 1``-th entry from ``cost_at_end_of_each_iter`` and ``library``.
    """
    initial_cost = payload["initial_cost"]
    costs = payload.get("cost_at_end_of_each_iter", [])
    library = payload.get("library", [])
    times = payload.get("iteration_times", [])

    limit = min(max_step, len(costs), len(library), len(times))
    xs = [0]
    ys = [1.0]
    labels = [""]
    for step in range(1, limit + 1):
        xs.append(step)
        ys.append(compression_ratio(initial_cost, costs[step - 1]))
        labels.append(library[step - 1])
    last_time = times[limit - 1] if limit > 0 else math.nan
    return xs, ys, unroll_labels(labels), last_time


def unroll_labels(labels: list[str]) -> list[str]:
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
            offset = 30
            yoff = offset if above_median else -offset
            annotate_diagram(ax, x, y, label, yoff)

    ax.annotate(
        f"t={time_value:.3f}s",
        xy=(xs[-1], ys[-1]),
        xytext=(8, 0),
        textcoords="offset points",
        ha="left",
        va="center",
        fontsize=7,
        color=color,
        bbox={"facecolor": "white", "alpha": 0.82, "edgecolor": "none", "pad": 0.2},
    )


def render_domain(saved: dict, domain: str, out_path: Path, max_step: int) -> None:
    """Render one domain figure and write it to ``out_path``."""
    import matplotlib.pyplot as plt

    fig, ax = plt.subplots(figsize=(8.8, 6.0), tight_layout=True)

    domain_payload = saved[domain]

    results = []

    for method in METHODS:
        payload = domain_payload.get(method)
        results.append(step_points(payload, max_step))

    median_ys = np.median([ys for _, ys, _, _ in results], axis=0)

    for method, (xs, ys, labels, last_time) in zip(METHODS, results):
        ax.plot(
            xs,
            ys,
            color=METHOD_COLORS[method],
            marker=METHOD_MARKERS[method],
            linewidth=2.0,
            markersize=6,
            label=METOD_COMMON_NAME[method]
        )
        annotate_series(
            ax, xs, ys, labels, METHOD_COLORS[method], last_time, median_ys=median_ys
        )

    ax.set_xlim(-0.2, max_step + 0.35)
    ax.set_xticks(list(range(max_step + 1)))
    ax.tick_params(axis="x", labelbottom=True)
    ax.set_xlabel("Step")
    ax.set_ylabel("Compression")
    ax.grid(True, which="both", linewidth=0.35, alpha=0.35)
    ax.legend(loc="upper left")
    lo, hi = ax.get_ylim()
    r = hi - lo
    ax.set_ylim(lo - r * 0.05, hi + r * 0.15)

    ax.set_title(f"Scramble results: {domain}")
    out_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(out_path, dpi=300)
    plt.close(fig)


def main() -> None:
    """Parse CLI arguments and render one graph per domain."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--input", type=Path, default=RESULTS_PATH, help="Path to scramble_results.json"
    )
    parser.add_argument(
        "--output",
        type=Path,
        default=DEFAULT_OUT_PATH,
        help="Output file path or directory for the rendered graphs",
    )
    parser.add_argument(
        "--max-step", type=int, default=4, help="Maximum step index to plot, inclusive"
    )
    args = parser.parse_args()

    saved = load_results(args.input)
    out_dir = args.output if args.output.suffix == "" else args.output.with_suffix("")
    out_dir.mkdir(parents=True, exist_ok=True)
    for domain in saved:
        render_domain(saved, domain, out_dir / f"{domain}.png", args.max_step)


if __name__ == "__main__":
    main()

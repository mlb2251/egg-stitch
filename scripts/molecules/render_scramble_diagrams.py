#!/usr/bin/env python3
"""Render scramble results with compact molecular diagrams at each step.

This is a molecule-specific companion to the plain text renderer. It keeps the
same per-domain compression-vs-step layout, but instead of labeling points with
the raw learned function text it renders a small molecular diagram for each
learned library entry.

The JSON format is the one written by ``scripts/molecules/run_scramble_experiment.py``:
``{domain: {method: {initial_cost, cost_at_end_of_each_iter, library,
iteration_times}}}``.
"""

from __future__ import annotations

import argparse
import json
import math
import re
from dataclasses import dataclass
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent.parent
RESULTS_PATH = PROJECT_ROOT / "results" / "scramble_results.json"
DEFAULT_OUT_PATH = PROJECT_ROOT / "figures" / "scramble_results_molecules.png"

METHODS = ("DSR-canon", "search-DSR")
METHOD_COLORS = {
    "DSR-canon": "#1f77b4",
    "search-DSR": "#d62728",
}
METHOD_MARKERS = {
    "DSR-canon": "o",
    "search-DSR": "s",
}

ATOM_COLORS = {
    "C": "#2d2d2d",
    "H": "#6b7280",
    "N": "#2563eb",
    "O": "#dc2626",
    "S": "#ca8a04",
    "P": "#d97706",
    "F": "#16a34a",
    "Cl": "#16a34a",
    "Br": "#b45309",
    "I": "#7c3aed",
}


@dataclass(frozen=True)
class ExprNode:
    """A parsed expression node from a learned library entry."""

    head: str
    label: str
    children: tuple["ExprNode", ...]


def compression_ratio(initial_cost: float, cost: float) -> float:
    """Return the compression ratio for a cost value."""
    return initial_cost / cost


def load_results(path: Path) -> dict:
    """Load the scramble results JSON from ``path``."""
    with open(path) as fh:
        return json.load(fh)


def step_points(payload: dict, max_step: int) -> tuple[list[int], list[float], list[str], float]:
    """Return x/y points, diagram labels, and the last plotted iteration time."""
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
    return xs, ys, labels, last_time


def subplot_grid(n_items: int) -> tuple[int, int]:
    """Choose a compact rows/cols layout for ``n_items`` subplots."""
    if n_items <= 2:
        return 1, n_items
    cols = 2
    rows = math.ceil(n_items / cols)
    return rows, cols


def tokenize_expr(expr: str) -> list[str]:
    """Split an expression string into parentheses and atom-like tokens."""
    return re.findall(r"\(|\)|[^\s()]+", expr)


def parse_expr(tokens: list[str], pos: int = 0) -> tuple[ExprNode, int]:
    """Parse a rooted list expression into nested ``ExprNode`` values."""
    token = tokens[pos]
    if token != "(":
        return ExprNode(head=token, label=token, children=()), pos + 1

    head = tokens[pos + 1]
    if re.fullmatch(r"[msdt]\d+", head):
        label = tokens[pos + 2]
        children = []
        pos += 3
    else:
        label = head
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


def bond_order(head: str) -> int:
    """Infer the bond order encoded in an expression head."""
    if head.startswith("d"):
        return 2
    if head.startswith("t"):
        return 3
    return 1


def display_label(label: str) -> tuple[str, str]:
    """Map a raw label to a display label and a style class.

    Learned function variables and metavariables get rendered as variable-like
    nodes so the diagram reads more like a molecule with R-groups.
    """
    fn_match = re.fullmatch(r"fn_(\d+)", label)
    if fn_match:
        return f"R{fn_match.group(1)}", "var"
    var_match = re.fullmatch(r"\?#(\d+)", label)
    if var_match:
        return f"X{var_match.group(1)}", "var"
    return label, "atom"


def with_implicit_root(node: ExprNode) -> ExprNode:
    """Add a displayed attachment root when the expression is rooted at s/d/t.

    The top-level molecule terms in this experiment are usually rooted at an
    ``m*`` node, but the helper also handles rooted subterms by inserting a
    dummy attachment node above them so the depiction keeps its open valence.
    """
    if node.head.startswith("m"):
        return node
    return ExprNode(head="m1", label="R", children=(node,))


def layout_tree(node: ExprNode) -> tuple[dict[int, tuple[float, float]], dict[int, tuple[int, int]], list[int]]:
    """Compute a simple rooted-tree layout and return node positions.

    Leaves are spaced left-to-right; internal nodes sit above the average x of
    their children. The return value is ``(positions, edges, order)`` where
    ``positions`` maps ``id(node)`` to normalized coordinates, ``edges`` maps a
    child node id to ``(parent_id, bond_order)`` and ``order`` preserves the
    draw order.
    """

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
            edges[node_id] = (id(parent), bond_order(current.label))
        return x

    walk(node, 0, None)
    return positions, edges, order


def normalize_positions(positions: dict[int, tuple[float, float]]) -> dict[int, tuple[float, float]]:
    """Scale layout coordinates into the unit box.

    The tree renderer later expands the unit box into a small annotation area.
    """
    xs = [p[0] for p in positions.values()]
    ys = [p[1] for p in positions.values()]
    min_x, max_x = min(xs), max(xs)
    min_y, max_y = min(ys), max(ys)
    x_span = max(max_x - min_x, 1.0)
    y_span = max(max_y - min_y, 1.0)

    return {
        node_id: ((x - min_x) / x_span, (y - min_y) / y_span)
        for node_id, (x, y) in positions.items()
    }


def add_bond_segments(drawer, p1: tuple[float, float], p2: tuple[float, float], order: int, color: str) -> None:
    """Add one, two, or three parallel bond segments to ``drawer``."""
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
        line = Line2D(
            [x1 + px * offset, x2 + px * offset],
            [y1 + py * offset, y2 + py * offset],
            color=color,
            linewidth=1.0 if order == 1 else 0.9,
            solid_capstyle="round",
        )
        drawer.add_artist(line)


def render_molecule_diagram(entry: str, color: str):
    """Build a small molecular diagram artist for ``entry``.

    The return value is a ``DrawingArea`` suitable for use inside an
    ``AnnotationBbox``.
    """
    from matplotlib.patches import Circle
    from matplotlib.offsetbox import DrawingArea
    from matplotlib.text import Text

    node = parse_library_expr(entry)
    node = with_implicit_root(node)
    positions, edges, order = layout_tree(node)
    positions = normalize_positions(positions)

    width = 56
    height = 40
    pad_x = 6
    pad_y = 5
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
            color,
        )

    for node_id in order:
        x, y = positions[node_id]
        x = pad_x + x * scale_x
        y = pad_y + y * scale_y
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
        disp_label, kind = display_label(current.label)
        atom_color = ATOM_COLORS.get(disp_label, "#64748b") if kind == "atom" else "#64748b"
        facecolor = "#fffdf7" if kind == "atom" else "#f3f4f6"
        radius = 2.9 if current.children else 2.4
        circle = Circle((x, y), radius=radius, facecolor=facecolor, edgecolor=atom_color, linewidth=1.0)
        drawer.add_artist(circle)
        text = Text(x, y, disp_label, fontsize=4.8, ha="center", va="center", color=atom_color)
        drawer.add_artist(text)

    return drawer


def annotate_diagram(ax, x: float, y: float, entry: str, color: str, offset_y: int) -> None:
    """Attach a molecular-diagram annotation to the given data point."""
    from matplotlib.offsetbox import AnnotationBbox

    box = render_molecule_diagram(entry, color)
    ann = AnnotationBbox(
        box,
        (x, y),
        xybox=(0, offset_y),
        xycoords="data",
        boxcoords="offset points",
        frameon=False,
        box_alignment=(0.5, 0.5),
        pad=0.0,
        zorder=5,
    )
    ax.add_artist(ann)


def render(saved: dict, out_path: Path, max_step: int) -> None:
    """Render the full scramble-results figure to ``out_path``."""
    import matplotlib.pyplot as plt
    from matplotlib.lines import Line2D

    domains = list(saved.keys())
    rows, cols = subplot_grid(len(domains))
    fig, axes = plt.subplots(rows, cols, figsize=(6.8 * cols, 4.9 * rows), sharex=True, sharey=True)

    if hasattr(axes, "ravel"):
        axes = list(axes.ravel())
    else:
        axes = [axes]

    legend_handles = [
        Line2D(
            [], [],
            color=METHOD_COLORS[m],
            marker=METHOD_MARKERS[m],
            linewidth=1.8,
            markersize=5,
            label=m,
        )
        for m in METHODS
    ]

    for ax, domain in zip(axes, domains):
        domain_payload = saved[domain]
        for method in METHODS:
            payload = domain_payload.get(method)
            if not payload:
                continue
            xs, ys, labels, last_time = step_points(payload, max_step)
            ax.plot(
                xs,
                ys,
                color=METHOD_COLORS[method],
                marker=METHOD_MARKERS[method],
                linewidth=1.8,
                markersize=5,
                label=method,
                zorder=2,
            )
            for step, (x, y, label) in enumerate(zip(xs, ys, labels)):
                if step == 0:
                    continue
                offset_y = 12 if step % 2 else -18
                annotate_diagram(ax, x, y, label, METHOD_COLORS[method], offset_y)
            ax.annotate(
                f"t={last_time:.3f}s",
                xy=(xs[-1], ys[-1]),
                xytext=(10, 0),
                textcoords="offset points",
                ha="left",
                va="center",
                fontsize=7,
                color=METHOD_COLORS[method],
                bbox={"facecolor": "white", "alpha": 0.85, "edgecolor": "none", "pad": 0.2},
                zorder=6,
            )

        ax.set_title(domain)
        ax.set_xlim(-0.2, max_step + 0.35)
        ax.set_xticks(list(range(max_step + 1)))
        ax.tick_params(axis="x", labelbottom=True)
        ax.set_xlabel("Step")
        ax.set_ylabel("Compression")
        ax.grid(True, which="both", linewidth=0.35, alpha=0.35)

    for ax in axes[len(domains):]:
        ax.axis("off")

    fig.legend(handles=legend_handles, loc="upper center", ncol=len(METHODS), frameon=False, bbox_to_anchor=(0.5, 0.985))
    fig.suptitle("Scramble results: compression over learned steps", y=1.03)
    fig.tight_layout(rect=(0, 0, 1, 0.95))
    out_path.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(out_path, dpi=300)
    plt.close(fig)


def main() -> None:
    """Parse CLI arguments and render the molecular-diagram figure."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=RESULTS_PATH,
                        help="Path to scramble_results.json")
    parser.add_argument("--output", type=Path, default=DEFAULT_OUT_PATH,
                        help="Where to write the rendered PNG")
    parser.add_argument("--max-step", type=int, default=4,
                        help="Maximum step index to plot, inclusive")
    args = parser.parse_args()

    saved = load_results(args.input)
    render(saved, args.output, args.max_step)


if __name__ == "__main__":
    main()
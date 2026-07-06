#!/usr/bin/env python3
"""Render ``results/arity.json`` as arity-vs-time plots.

For each domain, plots search time (y, log) against the abstraction-arity cap
(x, log), one line per method (BFS, Stitch). A curve that exceeds its timeout
ends at an "x" at the timeout height. A shaded band marks the arities whose
converged compression matches the unbounded (10^6) optimum.

Writes ``figures/arity/<domain>.png`` per domain.
"""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from expts.render_common import aggregate_cr, aggregate_time, has_dnf  # noqa: E402
from render_tables import METHOD_PLOT_LABELS  # noqa: E402
from render_molecules import METHOD_COLORS as _MOL_COLORS  # noqa: E402

# Reuse the molecules figure's palette: E-Stitch (our BFS) in orange, Stitch in
# purple, so the two figures read as one colour scheme.
METHOD_COLORS = {"enum": _MOL_COLORS["search-DSR"], "stitch": _MOL_COLORS["DSR-canon"]}

# Presentation-figure blue, used to shade the optimal-abstraction arity range.
OPTIMAL_COLOR = "#80cdff"

PROJECT_ROOT = Path(__file__).resolve().parent.parent
RESULTS_JSON = PROJECT_ROOT / "results" / "arity.json"
FIGURES_DIR = PROJECT_ROOT / "figures"

# Draw BFS first, Stitch on top; both keys exist in render_tables' maps.
METHOD_ORDER = ["enum", "stitch"]
DOMAIN_TITLES = {"wheels": "Wheels", "furniture": "Furniture"}

# The sweep is dense over 1..20 then jumps to a single "unbounded" arity, so the
# x axis is broken: a wide panel for 1..MAIN_ARITY_MAX and a narrow one for the
# large point, separated by diagonal break marks.
MAIN_ARITY_MAX = 20
LARGE_ARITY = 1_000_000


def method_curve(arity_map: dict[str, list]) -> tuple[list, tuple | None]:
    """Reduce ``{arity_str: repeats}`` to a plottable curve.

    Returns ``(points, dnf)`` where ``points`` is a sorted list of
    ``(arity, time, cr)`` for every converged arity and ``dnf`` is
    ``(arity, budget_seconds)`` for the arity that timed out (or None).
    """
    points: list[tuple[int, float, float | None]] = []
    dnf: tuple[int, float] | None = None
    for a_str, reps in sorted(arity_map.items(), key=lambda kv: int(kv[0])):
        a = int(a_str)
        if has_dnf(reps):
            # First timed-out arity; the DNF sentinel stored the wall-clock
            # budget as elapsed_secs (domains are single-file, so reps[0][0]).
            if dnf is None:
                dnf = (a, reps[0][0]["elapsed_secs"])
            continue
        points.append((a, aggregate_time(reps), aggregate_cr(reps)))
    return points, dnf


def assert_cr_agreement(methods: dict[str, dict], domain: str, tol: float = 1e-3) -> None:
    """Assert BFS and Stitch report the same compression at every arity where
    both converged.

    Both search a single abstraction to convergence under the same cost metric,
    so their CR should match. :func:`optimal_arity` trusts BFS's CR as canonical;
    fail loudly if that ever breaks.
    """
    if "enum" not in methods or "stitch" not in methods:
        return
    ec = {a: cr for a, _t, cr in method_curve(methods["enum"])[0]}
    sc = {a: cr for a, _t, cr in method_curve(methods["stitch"])[0]}
    for a in sorted(set(ec) & set(sc)):
        assert abs(ec[a] - sc[a]) <= tol * ec[a], (
            f"{domain} a={a}: BFS CR {ec[a]:.4f} != Stitch CR {sc[a]:.4f} "
            "-- tools disagree on the optimal single abstraction"
        )


def optimal_arity(methods: dict[str, dict]) -> int | None:
    """Smallest bounded arity whose converged compression matches the unbounded
    (10^6) optimum.

    Both tools run a single abstraction to convergence under the same cost
    metric, so their CR curves agree; use whichever reaches the large arity (BFS,
    else Stitch). Returns None if no such point exists.
    """
    for method in ("enum", "stitch"):
        pts = [p for p in method_curve(methods.get(method, {}))[0] if p[2] is not None]
        if pts:
            break
    else:
        return None
    big = [cr for a, _t, cr in pts if a > MAIN_ARITY_MAX]
    if not big:
        return None
    target = big[0]
    for a, _t, cr in pts:
        if a <= MAIN_ARITY_MAX and abs(cr - target) <= target * 0.0005:
            return a
    return None


def _draw_curves(ax, axb, methods: dict[str, dict]) -> None:
    """Plot every method's curve + timeout marker on both the main (``ax``,
    arities 1..MAIN_ARITY_MAX) and break (``axb``, the large arity) panels.

    The 1..20 region is a solid line on the main panel; a converged 10^6 point is
    a dot on the break panel (labelled with its time in seconds), no connector. A
    timed-out method ends at an "x" at its first timed-out arity, reached by a
    dashed line from the last converged point.
    """
    for method in METHOD_ORDER:
        if method not in methods:
            continue
        color = METHOD_COLORS.get(method, "black")
        label = METHOD_PLOT_LABELS.get(method, method)
        points, dnf = method_curve(methods[method])
        main_pts = [(a, t) for a, t, _cr in points if a <= MAIN_ARITY_MAX]
        big_pts = [(a, t) for a, t, _cr in points if a > MAIN_ARITY_MAX]

        # Solid line through the dense 1..20 region.
        if main_pts:
            ax.plot([a for a, _ in main_pts], [t for _, t in main_pts], "-o",
                    color=color, markersize=4, linewidth=1.4, label=label, zorder=2)
        # Converged 10^6 point(s) as dots on the break panel (no connector),
        # each labelled with its convergence time in seconds.
        for a, t in big_pts:
            axb.plot([a], [t], "o", color=color, markersize=7, zorder=3)
            axb.annotate(f"{t:.0f}s", (a, t), textcoords="offset points",
                         xytext=(8, 0), ha="left", va="center", fontsize=7,
                         color=color, zorder=4)

        # Timeout: an "x" at the first timed-out arity, reached by a dashed line
        # from the last converged point.
        if dnf is not None:
            first, budget = dnf
            panel = ax if first <= MAIN_ARITY_MAX else axb
            panel.scatter([first], [budget], color=color, marker="x", s=60,
                          linewidths=1.8, zorder=3, label=label if not main_pts else None)
            if main_pts:
                la, lt = main_pts[-1]
                for p in (ax, axb):
                    p.plot([la, first], [lt, budget], "--", color=color, linewidth=1.2, zorder=2)


def _draw_optimal(container, ax, axb, methods: dict[str, dict]) -> None:
    """Shade the arity range that reaches the unbounded optimum -- from the
    smallest such arity, across the axis-break gap, into the 10^6 cell. A
    matching legend entry names it (no on-graph label)."""
    from matplotlib.patches import Rectangle

    opt = optimal_arity(methods)
    if opt is None:
        return
    ax.axvspan(opt - 0.5, MAIN_ARITY_MAX + 0.5, facecolor=OPTIMAL_COLOR, alpha=0.35, zorder=0)
    axb.axvspan(LARGE_ARITY * 0.1, LARGE_ARITY * 10, facecolor=OPTIMAL_COLOR, alpha=0.35, zorder=0)
    # Bridge the inter-panel break with a container-level patch (behind the axes,
    # so only the otherwise-blank gap shows it).
    p0, p1 = ax.get_position(), axb.get_position()
    container.add_artist(Rectangle(
        (p0.x1, p0.y0), p1.x0 - p0.x1, p0.height, transform=container.transSubfigure,
        facecolor=OPTIMAL_COLOR, alpha=0.35, linewidth=0, zorder=0))


def _break_marks(left, right) -> None:
    """Draw the diagonal axis-break marks on a (left, right) panel pair."""
    left.spines["right"].set_visible(False)
    right.spines["left"].set_visible(False)
    d = 0.5  # slant of the marks
    kw = dict(marker=[(-1, -d), (1, d)], markersize=8, linestyle="none",
              color="k", mec="k", mew=1, clip_on=False)
    left.plot([1, 1], [0, 1], transform=left.transAxes, **kw)
    right.plot([0, 0], [0, 1], transform=right.transAxes, **kw)


def render_domain_panel(container, methods: dict[str, dict], title: str):
    """Draw one domain onto ``container`` (a Figure or SubFigure): a broken-x
    time-vs-arity plot (wide 1..20 panel + narrow 10^6 panel), with the
    optimal-abstraction arity range shaded."""
    from matplotlib.ticker import FixedLocator, FixedFormatter, MultipleLocator

    assert_cr_agreement(methods, title)

    gs = container.add_gridspec(1, 2, width_ratios=[8, 1], wspace=0.08)
    ax = container.add_subplot(gs[0, 0])
    axb = container.add_subplot(gs[0, 1], sharey=ax)

    _draw_optimal(container, ax, axb, methods)
    _draw_curves(ax, axb, methods)

    for a in (ax, axb):
        a.set_yscale("log")  # time spans several decades; keep it log
        a.grid(True, which="major", axis="y", linewidth=0.5, alpha=0.7)
    ax.set_xlim(0.5, MAIN_ARITY_MAX + 0.5)
    axb.set_xlim(LARGE_ARITY * 0.4, LARGE_ARITY * 3.0)  # right room for the time label
    ax.set_ylabel("Time (s)")
    axb.tick_params(labelleft=False, left=False, which="both")  # y ticks (incl. minor) belong to the main panel

    ax.xaxis.set_major_locator(FixedLocator([1, 5, 10, 15, 20]))
    ax.xaxis.set_minor_locator(MultipleLocator(1))
    axb.xaxis.set_major_locator(FixedLocator([LARGE_ARITY]))
    axb.xaxis.set_major_formatter(FixedFormatter([r"$10^6$"]))

    _break_marks(ax, axb)

    from matplotlib.patches import Patch
    handles, _ = ax.get_legend_handles_labels()
    if optimal_arity(methods) is not None:
        handles.append(Patch(facecolor=OPTIMAL_COLOR, alpha=0.35, label="optimal abstraction"))
    ax.legend(handles=handles, loc="upper left")
    container.suptitle(title)
    container.supxlabel("Max arity")
    return ax, axb


def main() -> None:
    if not RESULTS_JSON.exists():
        sys.exit(f"{RESULTS_JSON} not found -- run `python3 run.py arity_experiment` first")
    import matplotlib.pyplot as plt

    with open(RESULTS_JSON) as f:
        data = json.load(f)
    domains = list(data["domains"])
    FIGURES_DIR.mkdir(parents=True, exist_ok=True)
    (FIGURES_DIR / "arity").mkdir(parents=True, exist_ok=True)

    # Per-domain figures.
    for domain in domains:
        fig = plt.figure(figsize=(6, 4.5))
        render_domain_panel(fig, data["domains"][domain]["methods"], DOMAIN_TITLES.get(domain, domain))
        out = FIGURES_DIR / "arity" / f"{domain}.png"
        fig.savefig(out, dpi=300)
        plt.close(fig)
        print(f"wrote {out}")


if __name__ == "__main__":
    main()

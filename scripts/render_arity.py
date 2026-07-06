#!/usr/bin/env python3
"""Render ``results/arity.json`` as arity-vs-time plots.

For each domain, plots search time (y, log) against the abstraction-arity cap
(x, log) as one line per method (BFS, Stitch). Each curve stops where the tool
first exceeded its timeout; that stopping arity is marked with an "x" at the
timeout height. The arities at which the single learned abstraction's
compression ratio jumps (i.e. a higher-arity abstraction becomes optimal) are
annotated as vertical guides along the x-axis.

Writes ``figures/arity/<domain>.png`` per domain plus ``figures/arity.png``
(both domains side by side).
"""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from expts.render_common import aggregate_cr, aggregate_time, has_dnf  # noqa: E402
from render_tables import METHOD_COLORS, METHOD_PLOT_LABELS  # noqa: E402

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
            # First timeout arity (the sweep stops here). Since convergence time
            # is monotone in arity, every larger arity times out too; the DNF
            # sentinel stored the wall-clock budget as elapsed_secs.
            if dnf is None:
                dnf = (a, reps[0][0]["elapsed_secs"])
            continue
        points.append((a, aggregate_time(reps), aggregate_cr(reps)))
    return points, dnf


def compression_jumps(methods: dict[str, dict]) -> list[tuple[int, float]]:
    """Arities at which the optimal single abstraction's CR strictly increases.

    Both tools run to convergence, so their CR-vs-arity curves agree; use
    whichever reaches the higher arity (BFS) as canonical, falling back to
    Stitch. Returns ``(arity, new_cr)`` for each jump, in arity order.
    """
    for method in ("enum", "stitch"):
        pts = [p for p in method_curve(methods.get(method, {}))[0] if p[2] is not None]
        if pts:
            break
    else:
        return []
    jumps: list[tuple[int, float]] = []
    prev = None
    for a, _t, cr in pts:
        if prev is None or cr > prev * 1.0005:  # 0.05% guard against float noise
            jumps.append((a, cr))
        prev = cr
    return jumps


def _draw_curves(ax, axb, methods: dict[str, dict], sweep_arities: list[int]) -> None:
    """Plot every method's curve + timeout markers on both the main (``ax``,
    arities 1..MAIN_ARITY_MAX) and break (``axb``, the large arity) panels.

    Each series is drawn on both axes and clipped by their x-limits, so the
    1..20 detail and the 10^6 point read as one broken line. Once a method
    times out, an "x" is drawn at every sweep arity from there on (they all
    time out too), including the 10^6 point on the break panel.
    """
    for a, cr in compression_jumps(methods):
        target = ax if a <= MAIN_ARITY_MAX else axb
        target.axvline(a, color="0.6", linestyle=":", linewidth=0.8, zorder=0)
        target.annotate(
            f"a={a}\nCR {cr:.2f}", xy=(a, 0), xytext=(0, 2),
            xycoords=("data", "axes fraction"), textcoords="offset points",
            ha="center", va="bottom", fontsize=6, color="0.35",
        )

    for method in METHOD_ORDER:
        if method not in methods:
            continue
        color = METHOD_COLORS.get(method, "black")
        label = METHOD_PLOT_LABELS.get(method, method)
        points, dnf = method_curve(methods[method])
        if points:
            xs = [a for a, _t, _cr in points]
            ys = [t for _a, t, _cr in points]
            ax.plot(xs, ys, "-o", color=color, markersize=4, linewidth=1.4,
                    label=label, zorder=2)
            axb.plot(xs, ys, "-o", color=color, markersize=4, linewidth=1.4, zorder=2)
        if dnf is not None:
            # An "x" at the timeout budget for the first timed-out arity and
            # every larger sweep arity (all monotonically time out too). Legend
            # entry goes on the first x only when there's no line to carry it.
            first, budget = dnf
            needs_label = not points
            for a in sweep_arities:
                if a < first:
                    continue
                panel = ax if a <= MAIN_ARITY_MAX else axb
                lbl = label if (needs_label and panel is ax) else None
                panel.scatter([a], [budget], color=color, marker="x", s=60,
                              linewidths=1.8, zorder=3, label=lbl)
                if lbl:
                    needs_label = False


def render_domain_panel(container, methods: dict[str, dict], title: str,
                        sweep_arities: list[int]):
    """Draw one domain onto ``container`` (a Figure or SubFigure) as a broken-x
    pair: a wide 1..20 panel and a narrow 10^6 panel with diagonal break marks."""
    from matplotlib.ticker import FixedLocator, FixedFormatter, MultipleLocator

    gs = container.add_gridspec(1, 2, width_ratios=[8, 1], wspace=0.08)
    ax = container.add_subplot(gs[0, 0])
    axb = container.add_subplot(gs[0, 1], sharey=ax)

    _draw_curves(ax, axb, methods, sweep_arities)

    for a in (ax, axb):
        a.set_yscale("log")  # time spans several decades; keep it log
        a.grid(True, which="both", linewidth=0.3, alpha=0.5)
    ax.set_xlim(0.5, MAIN_ARITY_MAX + 0.5)
    axb.set_xlim(LARGE_ARITY * 0.4, LARGE_ARITY * 1.6)
    ax.set_ylabel("Time (s)")

    # Main panel: linear arity axis, labelled every 5 with integer minor ticks.
    ax.xaxis.set_major_locator(FixedLocator([1, 5, 10, 15, 20]))
    ax.xaxis.set_minor_locator(MultipleLocator(1))
    # Break panel: a single 10^6 tick, no y ticks (shared with the main panel).
    axb.xaxis.set_major_locator(FixedLocator([LARGE_ARITY]))
    axb.xaxis.set_major_formatter(FixedFormatter([r"$10^6$"]))
    axb.tick_params(labelleft=False, left=False, which="both")

    # Hide the facing spines and draw the diagonal break marks.
    ax.spines["right"].set_visible(False)
    axb.spines["left"].set_visible(False)
    d = 0.5  # slant of the break marks
    kw = dict(marker=[(-1, -d), (1, d)], markersize=8, linestyle="none",
              color="k", mec="k", mew=1, clip_on=False)
    ax.plot([1, 1], [0, 1], transform=ax.transAxes, **kw)
    axb.plot([0, 0], [0, 1], transform=axb.transAxes, **kw)

    ax.legend(title="Method", loc="upper left")
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
    sweep = data["config"]["arities"]
    FIGURES_DIR.mkdir(parents=True, exist_ok=True)
    (FIGURES_DIR / "arity").mkdir(parents=True, exist_ok=True)

    # Per-domain figures.
    for domain in domains:
        fig = plt.figure(figsize=(6, 4.5))
        render_domain_panel(fig, data["domains"][domain]["methods"], DOMAIN_TITLES.get(domain, domain), sweep)
        out = FIGURES_DIR / "arity" / f"{domain}.png"
        fig.savefig(out, dpi=300)
        plt.close(fig)
        print(f"wrote {out}")

    # Combined figure: one broken-axis sub-figure per domain, side by side.
    fig = plt.figure(figsize=(6 * len(domains), 4.5))
    subfigs = fig.subfigures(1, len(domains), wspace=0.08, squeeze=False)[0]
    for sf, domain in zip(subfigs, domains):
        render_domain_panel(sf, data["domains"][domain]["methods"], DOMAIN_TITLES.get(domain, domain), sweep)
    out = FIGURES_DIR / "arity.png"
    fig.savefig(out, dpi=300)
    plt.close(fig)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()

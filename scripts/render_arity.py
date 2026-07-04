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
DOMAIN_TITLES = {"wheels": "Wheels", "dials": "Dials"}


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
            # The DNF sentinel stored the wall-clock budget as elapsed_secs.
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


def plot_domain(ax, methods: dict[str, dict], title: str) -> None:
    """Draw one domain's arity-vs-time curves and compression-jump guides."""
    for a, cr in compression_jumps(methods):
        ax.axvline(a, color="0.6", linestyle=":", linewidth=0.8, zorder=0)
        ax.annotate(
            f"a={a}\nCR {cr:.2f}", xy=(a, 0), xytext=(0, 2),
            xycoords=("data", "axes fraction"), textcoords="offset points",
            ha="center", va="bottom", fontsize=6, color="0.35", rotation=0,
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
        if dnf is not None:
            # Mark where the tool blew the timeout: an "x" at the budget height.
            ax.scatter([dnf[0]], [dnf[1]], color=color, marker="x", s=60,
                       linewidths=1.8, zorder=3,
                       label=None if points else label)

    ax.set_xscale("log")
    ax.set_yscale("log")
    ax.set_xlabel("Max arity")
    ax.set_ylabel("Time to convergence (s)")
    ax.set_title(title)
    ax.grid(True, which="both", linewidth=0.3, alpha=0.5)
    ax.legend(title="Method", loc="upper left")


def main() -> None:
    if not RESULTS_JSON.exists():
        sys.exit(f"{RESULTS_JSON} not found -- run `python3 run.py arity_experiment` first")
    import matplotlib.pyplot as plt
    from matplotlib.ticker import ScalarFormatter, NullFormatter

    with open(RESULTS_JSON) as f:
        data = json.load(f)
    domains = list(data["domains"])
    FIGURES_DIR.mkdir(parents=True, exist_ok=True)
    (FIGURES_DIR / "arity").mkdir(parents=True, exist_ok=True)

    def _plain_axes(ax):
        for axis in (ax.xaxis, ax.yaxis):
            axis.set_major_formatter(ScalarFormatter())
            axis.set_minor_formatter(NullFormatter())

    # Per-domain figures.
    for domain in domains:
        fig, ax = plt.subplots(figsize=(6, 4.5))
        plot_domain(ax, data["domains"][domain]["methods"], DOMAIN_TITLES.get(domain, domain))
        _plain_axes(ax)
        fig.tight_layout()
        out = FIGURES_DIR / "arity" / f"{domain}.png"
        fig.savefig(out, dpi=300)
        plt.close(fig)
        print(f"wrote {out}")

    # Combined side-by-side figure.
    fig, axes = plt.subplots(1, len(domains), figsize=(6 * len(domains), 4.5), squeeze=False)
    for ax, domain in zip(axes[0], domains):
        plot_domain(ax, data["domains"][domain]["methods"], DOMAIN_TITLES.get(domain, domain))
        _plain_axes(ax)
    fig.tight_layout()
    out = FIGURES_DIR / "arity.png"
    fig.savefig(out, dpi=300)
    plt.close(fig)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()

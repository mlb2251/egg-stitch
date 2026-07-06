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
from render_tables import METHOD_PLOT_LABELS  # noqa: E402
from render_molecules import METHOD_COLORS as _MOL_COLORS  # noqa: E402

# Reuse the molecules figure's palette: E-Stitch (our BFS) in orange, Stitch in
# purple, so the two figures read as one colour scheme.
METHOD_COLORS = {"enum": _MOL_COLORS["search-DSR"], "stitch": _MOL_COLORS["DSR-canon"]}

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


def assert_cr_agreement(methods: dict[str, dict], domain: str, tol: float = 1e-3) -> None:
    """Assert BFS and Stitch report the same compression at every arity where
    both converged.

    Both search a *single* abstraction to convergence under the same cost metric
    (egg-stitch's unit-per-node op-children cost == Stitch's matched ``--cost``
    flags == the uniform ``ast_size`` the runner re-costs with), so they must
    find an equally-optimal abstraction -> identical CR. The single ``cr_segments``
    ribbon depends on this; fail loudly if it ever breaks.
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


def cr_segments(methods: dict[str, dict]) -> list[tuple[int, int, float]]:
    """Contiguous arity ranges that share one converged compression ratio.

    Both tools run to convergence, so their CR-vs-arity curves agree (see
    :func:`assert_cr_agreement`); use whichever reaches the higher arity (BFS) as
    canonical, falling back to Stitch. Returns ``(lo, hi, cr)`` per constant-CR
    run, in arity order (CR is monotone non-decreasing, so runs are contiguous).
    """
    for method in ("enum", "stitch"):
        pts = [p for p in method_curve(methods.get(method, {}))[0] if p[2] is not None]
        if pts:
            break
    else:
        return []
    segs: list[list] = []
    for a, _t, cr in pts:
        if segs and abs(cr - segs[-1][2]) <= segs[-1][2] * 0.0005:  # same CR
            segs[-1][1] = a
        else:
            segs.append([a, a, cr])
    return [(lo, hi, cr) for lo, hi, cr in segs]


def _draw_curves(ax, axb, methods: dict[str, dict]) -> None:
    """Plot every method's curve + timeout marker on both the main (``ax``,
    arities 1..MAIN_ARITY_MAX) and break (``axb``, the large arity) panels.

    The dense 1..20 region is a solid line on the main panel; a 10^6 point that
    converged is shown as a star on the break panel, with no connecting line. A
    method that times out ends at an "x" at its first timed-out arity, reached by
    a dashed line from the last converged point.
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
        # Converged 10^6 point(s) as stars on the break panel (no connector).
        for a, t in big_pts:
            axb.plot([a], [t], "*", color=color, markersize=13, zorder=3)

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


def _draw_cr_ribbon(rax, rbx, methods: dict[str, dict]) -> None:
    """Fill a thin strip below the plot with boxes spanning each constant-CR
    arity range, labelled with just that compression ratio.

    Alternating shades separate neighbours; a box is drawn (clipped) on both
    ribbon panels so a range that plateaus across the break shows on each. The
    label goes on the panel holding the wider part of the range.
    """
    for i, (lo, hi, cr) in enumerate(cr_segments(methods)):
        shade = "0.90" if i % 2 else "white"
        # Main ribbon: the 1..20 portion of this constant-CR range. A range that
        # continues past 20 has its right edge pushed off-panel so no border cuts
        # across the break (it resumes in the 10^6 cell).
        if lo <= MAIN_ARITY_MAX:
            right = hi + 0.5 if hi <= MAIN_ARITY_MAX else MAIN_ARITY_MAX + 1.0
            rax.axvspan(lo - 0.5, right, facecolor=shade,
                        edgecolor="0.55", linewidth=0.5, zorder=1)
        # Break ribbon: if the range reaches 10^6, fill the whole cell -- edges
        # pushed outside the x-limits so neither border cuts through the label.
        if hi > MAIN_ARITY_MAX:
            rbx.axvspan(LARGE_ARITY * 0.1, LARGE_ARITY * 10, facecolor=shade,
                        edgecolor="none", zorder=1)
        # Label the wider part: the 1..20 span, or the centre of the 10^6 cell.
        if hi <= MAIN_ARITY_MAX:
            rax.text((lo + hi) / 2, 0.5, f"{cr:.2f}", ha="center", va="center",
                     rotation=90, fontsize=6, color="0.15", zorder=2)
        else:
            rbx.text(LARGE_ARITY, 0.5, f"{cr:.2f}", ha="center", va="center",
                     rotation=90, fontsize=6, color="0.15", zorder=2)


def _shade_regions(ax, axb, methods: dict[str, dict]) -> None:
    """Shade the plot columns matching the gray (odd-indexed) CR-ribbon cells,
    so each constant-CR band lines up with its box in the ribbon below."""
    for i, (lo, hi, cr) in enumerate(cr_segments(methods)):
        if i % 2 == 0:  # gray cells are the odd-indexed ones (see _draw_cr_ribbon)
            continue
        if lo <= MAIN_ARITY_MAX:
            right = hi + 0.5 if hi <= MAIN_ARITY_MAX else MAIN_ARITY_MAX + 1.0
            ax.axvspan(lo - 0.5, right, facecolor="0.90", edgecolor="none", zorder=0)
        if hi > MAIN_ARITY_MAX:
            axb.axvspan(LARGE_ARITY * 0.1, LARGE_ARITY * 10, facecolor="0.90",
                        edgecolor="none", zorder=0)


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
    time-vs-arity plot (wide 1..20 panel + narrow 10^6 panel) over a thin
    compression-ratio ribbon that shares the same broken x axis."""
    from matplotlib.ticker import FixedLocator, FixedFormatter, MultipleLocator

    assert_cr_agreement(methods, title)

    gs = container.add_gridspec(2, 2, width_ratios=[8, 1], height_ratios=[6, 0.7],
                                wspace=0.08, hspace=0.3)
    ax = container.add_subplot(gs[0, 0])
    axb = container.add_subplot(gs[0, 1], sharey=ax)
    rax = container.add_subplot(gs[1, 0], sharex=ax)
    rbx = container.add_subplot(gs[1, 1], sharex=axb, sharey=rax)

    _shade_regions(ax, axb, methods)
    _draw_curves(ax, axb, methods)
    _draw_cr_ribbon(rax, rbx, methods)

    for a in (ax, axb):
        a.set_yscale("log")  # time spans several decades; keep it log
        a.grid(True, which="major", axis="y", linewidth=0.5, alpha=0.7)
    ax.set_xlim(0.5, MAIN_ARITY_MAX + 0.5)
    axb.set_xlim(LARGE_ARITY * 0.4, LARGE_ARITY * 1.6)
    ax.set_ylabel("Time (s)")
    axb.tick_params(labelleft=False, left=False, which="both")  # y ticks (incl. minor) belong to the main panel

    # The plot row carries the arity tick labels; the ribbon sits *below* them.
    ax.xaxis.set_major_locator(FixedLocator([1, 5, 10, 15, 20]))
    ax.xaxis.set_minor_locator(MultipleLocator(1))
    axb.xaxis.set_major_locator(FixedLocator([LARGE_ARITY]))
    axb.xaxis.set_major_formatter(FixedFormatter([r"$10^6$"]))

    # Ribbon row: pure CR boxes -- no ticks of its own.
    rax.set_ylim(0, 1)
    rax.set_yticks([])
    rbx.set_yticks([])
    rax.set_ylabel("CR", rotation=0, ha="right", va="center", fontsize=8)
    for r in (rax, rbx):
        r.tick_params(bottom=False, labelbottom=False)

    _break_marks(ax, axb)
    _break_marks(rax, rbx)

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

    # Combined figure: one broken-axis sub-figure per domain, side by side.
    fig = plt.figure(figsize=(6 * len(domains), 4.5))
    subfigs = fig.subfigures(1, len(domains), wspace=0.08, squeeze=False)[0]
    for sf, domain in zip(subfigs, domains):
        render_domain_panel(sf, data["domains"][domain]["methods"], DOMAIN_TITLES.get(domain, domain))
    out = FIGURES_DIR / "arity.png"
    fig.savefig(out, dpi=300)
    plt.close(fig)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Render Table 1-4 JSON result files as LaTeX tabulars and PNG plots.

Picks the newest timestamped run under
``viz/results/tableN/<timestamp>/tableN.json`` and writes
``figures/tableN.tex`` (LaTeX tabular) plus ``figures/tableN.png`` (log-log
scatter of compression ratio vs time; color = method, marker = domain).
Compression ratio and time are aggregated across runs with a geometric mean,
matching ``expts.table1.print_table1``.
"""

import argparse
import json
import math
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
RESULTS_DIR = PROJECT_ROOT / "viz" / "results"
FIGURES_DIR = PROJECT_ROOT / "figures"

TABLE_DOMAINS = ["nuts-bolts", "dials", "wheels", "furniture"]
DOMAIN_LABELS = {
    "nuts-bolts": "Nuts \\& Bolts",
    "dials": "Dials",
    "wheels": "Wheels",
    "furniture": "Furniture",
}
METHODS = ["enum", "smc", "babble", "stitch"]
METHOD_LABELS = {"enum": "Enum", "smc": "SMC", "babble": "babble", "stitch": "Stitch"}
TABLE_TITLES = {
    1: "Compression Using Rewrites",
    2: "Compression Without Rewrites",
    3: "Compression Using Rewrites, Stacked Abstractions",
    4: "Compression Without Rewrites, Stacked Abstractions",
}
# Tables that include an "E-graph min term size" column (runs with DSRs).
TABLES_WITH_EGRAPH_MIN = {1, 3}

# Plot styling: each method gets a color, each domain a marker. Keeping these
# as module-level dicts makes it easy to extend with new methods/domains.
THEME_COLORS = [
    "#80cdff",  # blue
    "#ffca80",  # orange
    "#60e37a",  # green
    "#ff80b1",  # pink
    "#bd80ff",  # purple
    "#000000",  # black
]


def modify_color(color: str, saturation_change: float, value_change: float):
    """Scale HSV saturation (toward full) and value of ``color``.

    Saturation transforms as ``s -> 1 - (1 - s) * saturation_change``, so
    ``saturation_change < 1`` pushes the color closer to fully saturated;
    ``value_change`` is a straight multiplier on V.
    """
    import matplotlib.colors as mcolors
    hsv = mcolors.rgb_to_hsv(mcolors.ColorConverter().to_rgb(color))
    hsv[1] = 1 - (1 - hsv[1]) * saturation_change
    hsv[2] *= value_change
    return mcolors.hsv_to_rgb(hsv)


def line_color(i: int):
    """Color for the i-th plotted series — darker, more saturated than theme."""
    return modify_color(THEME_COLORS[i], 0.5, 0.9)


# Plot uses a "line" variant of the pastel theme for readability on white.
METHOD_COLORS = {m: line_color(i) for i, m in enumerate(METHODS)}
DOMAIN_MARKERS = {"nuts-bolts": "s", "dials": "^", "wheels": "D", "furniture": "v"}
DOMAIN_PLOT_LABELS = {
    "nuts-bolts": "Nuts & Bolts",
    "dials": "Dials",
    "wheels": "Wheels",
    "furniture": "Furniture",
}


def latest_json(table: int) -> Path:
    """Return ``tableN.json`` from the newest timestamped subfolder."""
    root = RESULTS_DIR / f"table{table}"
    subdirs = [p for p in root.iterdir() if p.is_dir()]
    if not subdirs:
        sys.exit(f"no runs found under {root}")
    newest = max(subdirs, key=lambda p: p.name)
    path = newest / f"table{table}.json"
    if not path.exists():
        sys.exit(f"missing {path}")
    return path


def geomean_of(runs: dict, method: str, key: str) -> float | None:
    """Geometric mean of ``key`` across ``runs[method]``, or None if missing."""
    rs = runs.get(method)
    if not rs:
        return None
    xs = [r[key] for r in rs]
    return math.exp(sum(math.log(x) for x in xs) / len(xs))


def fmt(x: float | None, spec: str, na: str = "N/A") -> str:
    """Format a scalar with ``spec`` or return ``na`` when ``x`` is None."""
    return na if x is None else format(x, spec)


def geomean_col(xs: list[float | None]) -> float | None:
    """Geometric mean over non-None entries of ``xs``; None if all missing."""
    vs = [x for x in xs if x is not None]
    if not vs:
        return None
    return math.exp(sum(math.log(v) for v in vs) / len(vs))


def bold_best(xs: list[float | None], spec: str,
              higher_is_better: bool) -> list[str]:
    """Format each value, wrapping the best one(s) in ``\\textbf{}``."""
    vs = [x for x in xs if x is not None]
    best = max(vs) if higher_is_better and vs else (min(vs) if vs else None)
    out = []
    for x in xs:
        if x is None:
            out.append("N/A")
        else:
            s = format(x, spec)
            out.append(f"\\textbf{{{s}}}" if x == best else s)
    return out


def render(saved: dict, table: int) -> str:
    """Return a LaTeX ``tabular`` string for the given loaded results dict."""
    domains = saved["domains"]
    # Tables 1 & 3 run with DSRs (which Stitch doesn't accept); show the
    # Stitch column anyway with N/A so the layout matches Table 2.
    methods = METHODS
    n = len(methods)
    has_egraph_min = table in TABLES_WITH_EGRAPH_MIN

    # Column layout: domain, original size, (egraph-min for DSR tables,) CRs, times.
    extra_col = "r" if has_egraph_min else ""
    col_spec = "l r " + extra_col + " " + ("r" * n) + " " + ("r" * n)

    lines = []
    lines.append(f"% {TABLE_TITLES[table]}: generated from results JSON")
    lines.append("\\begin{tabular}{" + col_spec.strip() + "}")
    lines.append("\\toprule")

    # Header row 1: group spans.
    size_cols = 2 if has_egraph_min else 1
    lines.append(
        f"& \\multicolumn{{{size_cols}}}{{c}}{{Size}} "
        f"& \\multicolumn{{{n}}}{{c}}{{Compression Ratio}} "
        f"& \\multicolumn{{{n}}}{{c}}{{Time (s)}} \\\\"
    )
    # cmidrules: cols start at 2.
    mid = [f"\\cmidrule(lr){{2-{1 + size_cols}}}"]
    start = 2 + size_cols
    mid.append(f"\\cmidrule(lr){{{start}-{start + n - 1}}}")
    start += n
    mid.append(f"\\cmidrule(lr){{{start}-{start + n - 1}}}")
    lines.append(" ".join(mid))

    # Header row 2: column names.
    size_hdr = "Original & E-graph min" if has_egraph_min else "Original"
    method_hdr = " & ".join(METHOD_LABELS[m] for m in methods)
    lines.append(
        f"Domain & {size_hdr} & {method_hdr} & {method_hdr} \\\\"
    )
    lines.append("\\midrule")

    # Collect per-domain aggregates so we can bold the best cell in each row
    # and compute a geometric-mean summary row across benchmarks.
    rows: list[tuple[str, int, int | None, list[float | None], list[float | None]]] = []
    for domain in TABLE_DOMAINS:
        if domain not in domains:
            continue
        d = domains[domain]
        runs = d.get("runs", {})
        any_run = (runs.get("enum") or next(iter(runs.values())))[0]
        original = any_run["initial_cost"]
        label = DOMAIN_LABELS.get(domain, domain)
        crs = [geomean_of(runs, m, "compression_ratio") for m in methods]
        ts = [geomean_of(runs, m, "elapsed_secs") for m in methods]
        rows.append((label, original, d.get("egraph_min_size"), crs, ts))

    def emit(label: str, size_cells: list[str],
             crs: list[float | None], ts: list[float | None]) -> str:
        """Render one data row with the best CR (max) and time (min) bolded."""
        cr_strs = bold_best(crs, ".2f", higher_is_better=True)
        t_strs = bold_best(ts, ".3f", higher_is_better=False)
        return " & ".join([label, *size_cells, *cr_strs, *t_strs]) + " \\\\"

    for label, original, egraph_min, crs, ts in rows:
        size_cells = [fmt(original, "d")]
        if has_egraph_min:
            size_cells.append(fmt(egraph_min, "d"))
        lines.append(emit(label, size_cells, crs, ts))

    # Geometric mean across benchmarks (per method, skipping missing cells).
    if rows:
        lines.append("\\midrule")
        agg_cr = [geomean_col([r[3][i] for r in rows]) for i in range(n)]
        agg_t = [geomean_col([r[4][i] for r in rows]) for i in range(n)]
        size_cells = [""] * (2 if has_egraph_min else 1)
        lines.append(emit("Geo. mean", size_cells, agg_cr, agg_t))

    lines.append("\\bottomrule")
    lines.append("\\end{tabular}")
    return "\n".join(lines)


def plot(saved: dict, table: int, out_path: Path) -> None:
    """Save a log-log scatter of compression ratio vs elapsed time.

    Each individual run contributes one point; color encodes the method,
    marker encodes the domain.
    """
    import matplotlib.pyplot as plt

    fig, ax = plt.subplots(figsize=(6, 4.5))
    domains = saved["domains"]
    methods_seen: set[str] = set()
    # Collect per-method per-domain geomeans so we can also plot a
    # cross-benchmark geomean for each method.
    by_method: dict[str, list[tuple[float, float]]] = {m: [] for m in METHODS}
    for domain in TABLE_DOMAINS:
        if domain not in domains:
            continue
        marker = DOMAIN_MARKERS.get(domain, "x")
        runs_by_method = domains[domain].get("runs", {})
        for method in METHODS:
            cr = geomean_of(runs_by_method, method, "compression_ratio")
            t = geomean_of(runs_by_method, method, "elapsed_secs")
            if cr is None or t is None:
                continue
            methods_seen.add(method)
            by_method[method].append((cr, t))
            ax.scatter(
                [cr], [t],
                color=METHOD_COLORS.get(method, "black"),
                marker=marker,
                s=25, edgecolors="none",
            )

    # Cross-benchmark geomean per method, drawn larger with a star marker.
    for method in METHODS:
        pts = by_method[method]
        if not pts:
            continue
        cr = math.exp(sum(math.log(p[0]) for p in pts) / len(pts))
        t = math.exp(sum(math.log(p[1]) for p in pts) / len(pts))
        ax.scatter(
            [cr], [t],
            color=METHOD_COLORS.get(method, "black"),
            marker="o", s=75, edgecolors="none",
        )

    ax.set_xscale("log")
    ax.set_yscale("log")
    # Plain numbers on the log axes (e.g. "1.4", "10") instead of "1.4 × 10^0".
    # The compression-ratio axis spans less than a decade, so also label minor
    # ticks (else only "1" would show); the time axis spans several decades so
    # major-only is plenty.
    from matplotlib.ticker import ScalarFormatter, NullFormatter
    ax.xaxis.set_major_formatter(ScalarFormatter())
    ax.xaxis.set_minor_formatter(ScalarFormatter())
    ax.yaxis.set_major_formatter(ScalarFormatter())
    ax.yaxis.set_minor_formatter(NullFormatter())
    ax.set_xlabel("Compression ratio")
    ax.set_ylabel("Time (s)")
    ax.set_title(TABLE_TITLES[table])
    ax.grid(True, which="both", linewidth=0.3, alpha=0.5)

    # Two legends: one for method colors, one for domain markers.
    from matplotlib.lines import Line2D
    method_handles = [
        Line2D([], [], linestyle="none", marker="o",
               color=METHOD_COLORS[m], label=METHOD_LABELS[m])
        for m in METHODS if m in methods_seen
    ]
    domain_handles = [
        Line2D([], [], linestyle="none", marker=DOMAIN_MARKERS[d],
               color="gray", label=DOMAIN_PLOT_LABELS[d])
        for d in TABLE_DOMAINS
    ]
    domain_handles.append(
        Line2D([], [], linestyle="none", marker="o", color="gray",
               markersize=7, label="Geo. mean")
    )
    # Put both legends outside the axes so they don't cover points.
    leg1 = ax.legend(handles=method_handles, title="Method",
                     loc="upper left", bbox_to_anchor=(1.02, 1.0),
                     borderaxespad=0.0)
    ax.add_artist(leg1)
    ax.legend(handles=domain_handles, title="Domain",
              loc="upper left", bbox_to_anchor=(1.02, 0.55),
              borderaxespad=0.0)

    fig.tight_layout()
    fig.savefig(out_path, dpi=300)
    plt.close(fig)


def main() -> None:
    """Render each table as a LaTeX file and PNG plot under ``figures/``."""
    argparse.ArgumentParser(description=__doc__).parse_args()

    FIGURES_DIR.mkdir(exist_ok=True)
    for table in (1, 2, 3, 4):
        path = latest_json(table)
        with open(path) as f:
            saved = json.load(f)
        tex_path = FIGURES_DIR / f"table{table}.tex"
        tex_path.write_text(f"% source: {path}\n" + render(saved, table) + "\n")
        print(f"wrote {tex_path}", file=sys.stderr)
        plot_path = FIGURES_DIR / f"table{table}.png"
        plot(saved, table, plot_path)
        print(f"wrote {plot_path}", file=sys.stderr)


if __name__ == "__main__":
    main()

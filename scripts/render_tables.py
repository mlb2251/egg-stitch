#!/usr/bin/env python3
"""Render Table 1-4 JSON result files as LaTeX tabulars and PNG plots.

Reads ``results/tableN.json`` (per-file records, list per (method, repeat))
and writes ``figures/tableN.tex`` (LaTeX tabular) plus ``figures/tableN.png``
(log-log scatter of compression ratio against time; time on the x axis,
compression ratio on the y axis; color = method, marker =
domain). Sizes shown for DC (dreamcoder) domains are per-file averages;
cogsci domains have a single file per repeat and show that size directly.
"""

import argparse
import json
import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from expts.render_common import (  # noqa: E402
    aggregate_methods_cr,
    aggregate_methods_time,
    egraph_min_for_domain,
    initial_size_for_domain,
)
from expts.tables import (  # noqa: E402
    BFS_STEP_SWEEP,
    SMC_PARTICLE_SWEEP,
    TABLE5_BFS_SWEEP,
    TABLE5_DOMAINS,
    TABLE5_ENUM_POINT,
)

PROJECT_ROOT = Path(__file__).resolve().parent.parent
RESULTS_DIR = PROJECT_ROOT / "results"
FIGURES_DIR = PROJECT_ROOT / "figures"

# Tables 1/3 (DSR runs) only include domains that have a babble equational
# theory; tables 2/4 (no-DSR runs) include text/logo/towers as well.
TABLE_DOMAINS_DSR = ["nuts-bolts", "dials", "wheels", "furniture", "list", "physics"]
TABLE_DOMAINS_NO_DSR = TABLE_DOMAINS_DSR + ["text", "logo", "towers"]


def domains_for_table(table: int) -> list[str]:
    return TABLE_DOMAINS_DSR if table in TABLES_WITH_EGRAPH_MIN else TABLE_DOMAINS_NO_DSR
DOMAIN_LABELS = {
    "nuts-bolts": "Nuts \\& Bolts",
    "dials": "Dials",
    "wheels": "Wheels",
    "furniture": "Furniture",
    "list": "List",
    "physics": "Physics",
    "text": "Text",
    "logo": "Logo",
    "towers": "Towers",
}
METHODS = ["enum", "smc", "babble", "stitch"]
# Table cells use the bare search-strategy name; plot legends spell out the
# E-Stitch prefix so each series is unambiguous standalone.
METHOD_LABELS = {"enum": "BFS", "smc": "SMC", "babble": "babble", "stitch": "Stitch"}
METHOD_PLOT_LABELS = {
    "enum": "E-Stitch: BFS",
    "smc": "E-Stitch: SMC",
    "babble": "babble",
    "stitch": "Stitch",
}
# The single sweep point each base method contributes to the table cells.
# Plots use the full sweep regardless.
TABLE_BFS_STEPS = 10000
TABLE_SMC_PARTICLES = 1000
TABLE_DATA_KEYS = {
    "enum": f"enum-{TABLE_BFS_STEPS}",
    "smc": f"smc-{TABLE_SMC_PARTICLES}",
    "babble": "babble",
    "stitch": "stitch",
}
TABLE_TITLES = {
    1: "Compression Using Rewrites",
    2: "Compression Without Rewrites",
    3: "Compression Using Rewrites, Stacked Abstractions",
    4: "Compression Without Rewrites, Stacked Abstractions",
}
# Tables that include an "E-graph min term size" column (runs with DSRs).
TABLES_WITH_EGRAPH_MIN = {1, 3}
# DSR tables borrow Stitch numbers from the matching no-DSR table (Stitch
# doesn't accept DSRs); cells/markers are starred to flag the mismatch.
NO_DSR_COUNTERPART = {1: 2, 3: 4}
STITCH_STAR = "$^{\\star}$"

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
DOMAIN_PLOT_LABELS = {
    "nuts-bolts": "Nuts & Bolts",
    "dials": "Dials",
    "wheels": "Wheels",
    "furniture": "Furniture",
    "list": "List",
    "physics": "Physics",
    "text": "Text",
    "logo": "Logo",
    "towers": "Towers",
}


def results_json(table: int) -> Path:
    """Return the path to ``results/tableN.json`` (the canonical checked-in copy)."""
    path = RESULTS_DIR / f"table{table}.json"
    if not path.exists():
        sys.exit(f"missing {path}")
    return path


def fmt(x: float | None, spec: str, na: str = "N/A") -> str:
    """Format a scalar with ``spec`` or return ``na`` when ``x`` is None / NaN."""
    if x is None or (isinstance(x, float) and math.isnan(x)):
        return na
    return format(x, spec)


def geomean_col(xs: list[float | None]) -> float | None:
    """Geometric mean over non-None entries of ``xs``; None if all missing."""
    vs = [x for x in xs if x is not None]
    if not vs:
        return None
    return math.exp(sum(math.log(v) for v in vs) / len(vs))


def bold_best(xs: list[float | None], spec: str,
              higher_is_better: bool) -> list[str]:
    """Format each value, wrapping the best one(s) in ``\\textbf{}``.

    Ties are judged on the *rendered* value: any cell whose ``spec``-formatted
    string matches the best value's formatted string is bolded, so two cells
    that round to the same display (e.g. 7.191 and 7.192 both showing 7.19)
    are both bolded even though their underlying values differ.
    """
    vs = [x for x in xs if x is not None]
    best = max(vs) if higher_is_better and vs else (min(vs) if vs else None)
    best_str = format(best, spec) if best is not None else None
    out = []
    for x in xs:
        if x is None:
            out.append("N/A")
        else:
            s = format(x, spec)
            out.append(f"\\textbf{{{s}}}" if s == best_str else s)
    return out


def _stitch_no_dsr_maps(table: int) -> dict[str, tuple[float | None, float | None]]:
    """``{domain: (cr, time)}`` for Stitch from the matching no-DSR table.

    Returns an empty dict if ``table`` has no counterpart or the JSON is
    missing — callers treat that as "no starred values to inject."
    """
    other = NO_DSR_COUNTERPART.get(table)
    if other is None:
        return {}
    path = RESULTS_DIR / f"table{other}.json"
    if not path.exists():
        return {}
    with open(path) as fh:
        other_saved = json.load(fh)
    out: dict[str, tuple[float | None, float | None]] = {}
    for domain, payload in other_saved.get("domains", {}).items():
        runs = payload.get("runs", {})
        cr = aggregate_methods_cr(runs).get(TABLE_DATA_KEYS["stitch"])
        t = aggregate_methods_time(runs).get(TABLE_DATA_KEYS["stitch"])
        out[domain] = (cr, t)
    return out


def _collect_rows(
    saved: dict, table: int
) -> list[tuple[str, float | None, float | None, list[float | None], list[float | None]]]:
    """Per-domain ``(domain, original_size, egraph_min, crs, ts)`` for the table.

    ``crs``/``ts`` hold the single table-point value per method (in ``METHODS``
    order); on DSR tables the Stitch entry is spliced from the no-DSR
    counterpart. Shared by the LaTeX table and the bar-chart renderers.
    """
    domains = saved["domains"]
    stitch_no_dsr = _stitch_no_dsr_maps(table)
    stitch_idx = METHODS.index("stitch")
    rows = []
    for domain in domains_for_table(table):
        if domain not in domains:
            continue
        runs = domains[domain].get("runs", {})
        cr_map = aggregate_methods_cr(runs)
        t_map = aggregate_methods_time(runs)
        crs = [cr_map.get(TABLE_DATA_KEYS[m]) for m in METHODS]
        ts = [t_map.get(TABLE_DATA_KEYS[m]) for m in METHODS]
        if domain in stitch_no_dsr:
            crs[stitch_idx], ts[stitch_idx] = stitch_no_dsr[domain]
        rows.append((domain, initial_size_for_domain(runs), egraph_min_for_domain(runs), crs, ts))
    return rows


# Percent of the method's named color mixed with white for column shading;
# small so columns read as subtle background bands, not loud fills.
COLUMN_TINT_PERCENT = 9
# Named LaTeX colors (assumed \definecolor'd in the document preamble) per
# method: the two E-Stitch search methods share the highlight color, while
# the babble/Stitch baselines get their own.
PRESENTATION_COLORS = {
    "enum": "estitchHighlight",
    "smc": "estitchHighlight",
    "babble": "ecorange",
    "stitch": "ecblue",
}


def _shade_cells(cells: list[str], methods: list[str]) -> list[str]:
    """Prefix each cell with a faint ``\\cellcolor`` tint of its method color.

    The fill is ``<named color>!COLUMN_TINT_PERCENT`` (the rest white), so each
    method's column reads as a subtle background band. Requires the
    ``colortbl`` and ``xcolor`` packages plus the ``\\definecolor`` names in
    ``PRESENTATION_COLORS`` in the document preamble.
    """
    return [
        f"\\cellcolor{{{PRESENTATION_COLORS[m]}!{COLUMN_TINT_PERCENT}}}{cell}"
        for cell, m in zip(cells, methods)
    ]


def render(saved: dict, table: int, presentation: bool = False) -> str:
    """Return a LaTeX ``tabular`` string for the given loaded results dict.

    With ``presentation=True`` the Size/E-graph-min columns are dropped and
    every method column gets a faint background tint of its plot color.
    """
    domains = saved["domains"]
    # Tables 1 & 3 run with DSRs (which Stitch doesn't accept); fill the
    # Stitch column from the matching no-DSR table and star those cells.
    methods = METHODS
    if presentation and table == 3:
        methods = [m for m in methods if m != "stitch"]
    n = len(methods)
    has_egraph_min = table in TABLES_WITH_EGRAPH_MIN
    show_size = not presentation
    stitch_no_dsr = _stitch_no_dsr_maps(table)
    # The data rows always follow global METHODS; we only need this index
    # to star the Stitch cell (if it hasn't been filtered out).
    raw_stitch_idx = METHODS.index("stitch")

    # Column layout: domain, (size cols,) CRs, times. Presentation drops sizes
    # and adds a single vertical rule between the CR and Time groups.
    size_cols = (2 if has_egraph_min else 1) if show_size else 0
    size_spec = ("r " + ("r" if has_egraph_min else "")) if show_size else ""
    mid_sep = "|" if presentation else " "
    col_spec = ("l " + size_spec + " " + ("r" * n) + mid_sep + ("r" * n))

    lines = []
    lines.append(f"% {TABLE_TITLES[table]}: generated from results JSON")
    lines.append("\\begin{tabular}{" + " ".join(col_spec.split()) + "}")
    lines.append("\\toprule")

    # Header row 1: group spans.
    groups = []
    if show_size:
        groups.append(f"\\multicolumn{{{size_cols}}}{{c}}{{Size}}")
    cr_grp_fmt = "c|" if presentation else "c"
    groups.append(f"\\multicolumn{{{n}}}{{{cr_grp_fmt}}}{{Compression Ratio}}")
    groups.append(f"\\multicolumn{{{n}}}{{c}}{{Time (s)}}")
    lines.append("& " + " & ".join(groups) + " \\\\")
    # cmidrules under each group; data columns start after the Domain column.
    mid = []
    col = 2
    if show_size:
        mid.append(f"\\cmidrule(lr){{{col}-{col + size_cols - 1}}}")
        col += size_cols
    mid.append(f"\\cmidrule(lr){{{col}-{col + n - 1}}}")
    col += n
    mid.append(f"\\cmidrule(lr){{{col}-{col + n - 1}}}")
    lines.append(" ".join(mid))

    # Header row 2: column names (method labels tinted in presentation mode).
    method_hdr_cells = [METHOD_LABELS[m] for m in methods]
    if presentation:
        method_hdr_cells = _shade_cells(method_hdr_cells, methods)
    method_hdr = " & ".join(method_hdr_cells)
    hdr = ["Domain"]
    if show_size:
        hdr.append("Original & E-graph min" if has_egraph_min else "Original")
    hdr += [method_hdr, method_hdr]
    lines.append(" & ".join(hdr) + " \\\\")
    lines.append("\\midrule")

    # Collect per-domain aggregates so we can bold the best cell in each row
    # and compute a geometric-mean summary row across benchmarks. Sizes are
    # the per-file geomean within the domain (so DC domains with many files
    # are directly comparable to single-file cogsci domains).
    rows = _collect_rows(saved, table)

    def emit(label: str, size_cells: list[str],
             crs: list[float | None], ts: list[float | None]) -> str:
        """Render one data row with the best CR (max) and time (min) bolded.

        For DSR tables, Stitch cells come from the no-DSR run and get a
        trailing star to flag the mismatch.
        """
        # If we are dropping Stitch for this table, filter it out before bolding
        # so that the bolding is relative to the remaining methods.
        if presentation and table == 3:
            crs = [v for i, v in enumerate(crs) if i != raw_stitch_idx]
            ts = [v for i, v in enumerate(ts) if i != raw_stitch_idx]

        cr_strs = bold_best(crs, ".2f", higher_is_better=True)
        t_strs = bold_best(ts, ".3f", higher_is_better=False)

        # Star Stitch if it's still in the method list.
        if stitch_no_dsr and "stitch" in methods:
            stitch_idx = methods.index("stitch")
            if crs[stitch_idx] is not None:
                cr_strs[stitch_idx] += STITCH_STAR
            if ts[stitch_idx] is not None:
                t_strs[stitch_idx] += STITCH_STAR

        if presentation:
            cr_strs = _shade_cells(cr_strs, methods)
            t_strs = _shade_cells(t_strs, methods)
        return " & ".join([label, *size_cells, *cr_strs, *t_strs]) + " \\\\"

    for domain, original, egraph_min, crs, ts in rows:
        label = DOMAIN_LABELS.get(domain, domain)
        size_cells = []
        if show_size:
            size_cells = [fmt(original, ".0f")]
            if has_egraph_min:
                size_cells.append(fmt(egraph_min, ".0f"))
        lines.append(emit(label, size_cells, crs, ts))

    # Geometric mean across benchmarks (per method, skipping missing cells).
    if rows:
        lines.append("\\midrule")
        agg_cr = [geomean_col([r[3][i] for r in rows]) for i in range(len(METHODS))]
        agg_t = [geomean_col([r[4][i] for r in rows]) for i in range(len(METHODS))]
        size_cells = [""] * size_cols
        lines.append(emit("Geo. mean", size_cells, agg_cr, agg_t))

    lines.append("\\bottomrule")
    lines.append("\\end{tabular}")
    return "\n".join(lines)


# Sweep map for the two ours-search methods. Other methods (babble, stitch)
# are single points; sweep methods become lines connecting one point per
# parameter value.
SWEEP_FOR_METHOD: dict[str, tuple[int, ...]] = {
    "enum": BFS_STEP_SWEEP,
    "smc": SMC_PARTICLE_SWEEP,
}
# Sweep value that gets a filled marker (the one shown in the LaTeX table).
TABLE_SWEEP_POINT: dict[str, int] = {
    "enum": TABLE_BFS_STEPS,
    "smc": TABLE_SMC_PARTICLES,
}


def plot_cr_vs_time(cr_map: dict, t_map: dict, title: str, out_path: Path,
                    stitch_starred: bool = False, *,
                    methods: list[str] = METHODS,
                    sweep_for_method: dict = SWEEP_FOR_METHOD,
                    sweep_point: dict = TABLE_SWEEP_POINT,
                    method_colors: dict = METHOD_COLORS,
                    method_plot_labels: dict = METHOD_PLOT_LABELS) -> None:
    """Save a log-log plot of CR vs time given ``method-key -> value`` maps.

    Sweep methods (those in ``sweep_for_method``) contribute one line each,
    with one point per swept hyperparameter value; the rest contribute single
    points. Color encodes method. The ``methods``/``sweep_*``/``method_*``
    arguments default to the Table 1-4 roster but are overridden for table5
    (molecule families, different method set). ``stitch_starred`` swaps
    Stitch's marker to a star and stars its legend label (used on DSR tables
    where Stitch's number comes from the no-DSR run).
    """
    import matplotlib.pyplot as plt
    from matplotlib.ticker import ScalarFormatter, NullFormatter
    from matplotlib.lines import Line2D

    fig, ax = plt.subplots(figsize=(6, 4.5))
    methods_seen: set[str] = set()
    # Sweep-point labels to draw, in plot order. Deferred so we can drop any
    # whose text box would land on top of an already-placed one (keep first).
    sweep_labels: list[tuple[float, float, str, object]] = []

    for method in methods:
        color = method_colors.get(method, "black")
        sweep = sweep_for_method.get(method)
        if sweep is None:
            cr = cr_map.get(method)
            t = t_map.get(method)
            if cr is None or t is None:
                continue
            methods_seen.add(method)
            marker = "*" if (method == "stitch" and stitch_starred) else "o"
            size = 120 if marker == "*" else 50
            ax.scatter([t], [cr], color=color, marker=marker, s=size, zorder=2)
            continue
        # Sweep method: collect (cr, t, param) tuples, sorted by parameter
        # so the connecting line follows the sweep order.
        pts: list[tuple[float, float, int]] = []
        for n in sweep:
            key = f"{method}-{n}"
            cr = cr_map.get(key)
            t = t_map.get(key)
            if cr is None or t is None:
                continue
            pts.append((cr, t, n))
        if not pts:
            continue
        methods_seen.add(method)
        crs = [p[0] for p in pts]
        ts = [p[1] for p in pts]
        ax.plot(ts, crs, "-", color=color, linewidth=1.2, zorder=2)
        table_n = sweep_point[method]
        for cr, t, n in pts:
            if n == table_n:
                ax.scatter([t], [cr], color=color, marker="o", s=50, zorder=3)
            sweep_labels.append((t, cr, str(n), color))

    ax.set_xscale("log")
    ax.set_yscale("log")
    # Plain numbers on the log axes; the CR axis (now y) can span less than a
    # decade so label its minor ticks too. See the original plot() for the
    # rationale.
    ax.xaxis.set_major_formatter(ScalarFormatter())
    ax.xaxis.set_minor_formatter(NullFormatter())
    ax.yaxis.set_major_formatter(ScalarFormatter())
    ax.yaxis.set_minor_formatter(ScalarFormatter())
    ax.set_xlabel("Time (s)")
    ax.set_ylabel("Compression ratio")
    ax.set_title(title)
    ax.grid(True, which="both", linewidth=0.3, alpha=0.5)

    method_handles = [
        Line2D(
            [], [],
            linestyle="-" if m in sweep_for_method else "none",
            marker="*" if (m == "stitch" and stitch_starred) else "o",
            markersize=12 if (m == "stitch" and stitch_starred) else 6,
            color=method_colors[m],
            label=(method_plot_labels[m] + r"$^{\star}$"
                   if (m == "stitch" and stitch_starred) else method_plot_labels[m]),
        )
        for m in methods if m in methods_seen
    ]
    ax.legend(handles=method_handles, title="Method",
              loc="upper left", bbox_to_anchor=(1.02, 1.0),
              borderaxespad=0.0)

    # Draw sweep labels, skipping any whose box overlaps one already placed.
    # A draw() is needed first so the axes transform / text extents are final.
    fig.canvas.draw()
    renderer = fig.canvas.get_renderer()
    placed_boxes = []
    for t, cr, text, color in sweep_labels:
        ann = ax.annotate(text, xy=(t, cr), xytext=(3, 3),
                          textcoords="offset points", fontsize=7, color=color)
        bb = ann.get_window_extent(renderer=renderer)
        if any(bb.overlaps(pb) for pb in placed_boxes):
            ann.remove()
        else:
            placed_boxes.append(bb)

    fig.tight_layout()
    fig.savefig(out_path, dpi=300)
    plt.close(fig)


def plot_domain(saved: dict, table: int, domain: str, out_path: Path) -> None:
    """Plot CR vs time for a single domain.

    On DSR tables, splice in the matching no-DSR Stitch point so readers
    can see where regular Stitch lands; the marker/legend get a star.
    """
    runs = saved["domains"][domain].get("runs", {})
    cr_map = aggregate_methods_cr(runs)
    t_map = aggregate_methods_time(runs)
    stitch_no_dsr = _stitch_no_dsr_maps(table)
    starred = False
    if domain in stitch_no_dsr:
        cr, t = stitch_no_dsr[domain]
        if cr is not None and t is not None:
            cr_map[TABLE_DATA_KEYS["stitch"]] = cr
            t_map[TABLE_DATA_KEYS["stitch"]] = t
            starred = True
    title = f"{TABLE_TITLES[table]}\n{DOMAIN_PLOT_LABELS.get(domain, domain)}"
    plot_cr_vs_time(cr_map, t_map, title, out_path, stitch_starred=starred)


def plot_geomean(saved: dict, table: int, out_path: Path) -> None:
    """Plot CR vs time using geomeans (across the table's domains) per key."""
    domains = [d for d in domains_for_table(table) if d in saved["domains"]]
    per_cr = [aggregate_methods_cr(saved["domains"][d].get("runs", {})) for d in domains]
    per_t = [aggregate_methods_time(saved["domains"][d].get("runs", {})) for d in domains]
    stitch_no_dsr = _stitch_no_dsr_maps(table)
    starred = False
    if stitch_no_dsr:
        key = TABLE_DATA_KEYS["stitch"]
        for d, cm, tm in zip(domains, per_cr, per_t):
            if d in stitch_no_dsr:
                cr, t = stitch_no_dsr[d]
                if cr is not None and t is not None:
                    cm[key] = cr
                    tm[key] = t
                    starred = True
    keys = {k for m in per_cr for k in m} | {k for m in per_t for k in m}
    cr_map = {k: geomean_col([m.get(k) for m in per_cr]) for k in keys}
    t_map = {k: geomean_col([m.get(k) for m in per_t]) for k in keys}
    plot_cr_vs_time(cr_map, t_map,
                    f"{TABLE_TITLES[table]}\nGeo. mean across domains",
                    out_path, stitch_starred=starred)


# ─── table5: molecule scramble subset ────────────────────────────────────────
# Different roster from tables 1-4: the two ours sweeps (enum extended to 100k),
# the dsrs-only-at-start baseline (a single best-first point), and babble. No
# Stitch (it can't take DSRs) and no e-graph-min table column.
TABLE5_TITLE = "Molecule Scramble Compression (DSRs)"
TABLE5_DOMAIN_LABELS = {
    "molecules:hexyl": "Hexyl",
    "molecules:ester": "Ester",
    "molecules:glycol": "Glycol",
}
# Method order: the two sweeps, then the two single-point methods. The baseline
# keys on its data label ("enum-dsrs-at-start") so the single-point branch of
# plot_cr_vs_time finds it directly.
TABLE5_PLOT_METHODS = ["enum", "smc", "enum-dsrs-at-start", "babble"]
TABLE5_SWEEP_FOR_METHOD = {"enum": TABLE5_BFS_SWEEP, "smc": SMC_PARTICLE_SWEEP}
TABLE5_SWEEP_POINT = {"enum": TABLE5_ENUM_POINT, "smc": TABLE_SMC_PARTICLES}
TABLE5_METHOD_COLORS = {
    "enum": line_color(0),
    "smc": line_color(1),
    "babble": line_color(2),
    "enum-dsrs-at-start": line_color(3),
}
TABLE5_METHOD_PLOT_LABELS = {
    "enum": "E-Stitch: BFS",
    "smc": "E-Stitch: SMC",
    "babble": "babble",
    "enum-dsrs-at-start": "E-Stitch: BFS (DSRs at start)",
}

# LaTeX-table columns: the same four methods, each at the single representative
# operating point the plot marks with a filled dot. ``enum``/``smc`` map to
# their sweep point; the baseline and babble key on their own labels.
TABLE5_TABLE_METHODS = ["enum", "smc", "enum-dsrs-at-start", "babble"]
TABLE5_DATA_KEYS = {
    "enum": f"enum-{TABLE5_ENUM_POINT}",
    "smc": f"smc-{TABLE_SMC_PARTICLES}",
    "enum-dsrs-at-start": "enum-dsrs-at-start",
    "babble": "babble",
}
TABLE5_COL_LABELS = {
    "enum": "BFS",
    "smc": "SMC",
    "enum-dsrs-at-start": "BFS@start",
    "babble": "babble",
}


def render_table5_tex(saved: dict) -> str:
    """Return a LaTeX ``tabular`` for table5: molecule families × methods, with
    Compression Ratio and Time (s) groups and a geomean row.

    No e-graph-min or Stitch columns (table5 has neither). A method that timed
    out / OOM'd on a family has ``compression_ratio: null`` for that cell; it's
    rendered ``DNF`` and excluded from that row's bolding and the geomean.
    """
    domains = saved["domains"]
    methods = TABLE5_TABLE_METHODS
    n = len(methods)

    def cells(values: list[float | None], spec: str, higher_is_better: bool) -> list[str]:
        """bold_best, but render None as DNF (not N/A)."""
        out = bold_best(values, spec, higher_is_better)
        return [("DNF" if v is None else s) for v, s in zip(values, out)]

    col_spec = "l r " + ("r" * n) + " " + ("r" * n)
    lines = [
        f"% {TABLE5_TITLE}: generated from results JSON",
        "\\begin{tabular}{" + col_spec + "}",
        "\\toprule",
        "& \\multicolumn{1}{c}{Size} "
        f"& \\multicolumn{{{n}}}{{c}}{{Compression Ratio}} "
        f"& \\multicolumn{{{n}}}{{c}}{{Time (s)}} \\\\",
        f"\\cmidrule(lr){{2-2}} \\cmidrule(lr){{3-{2 + n}}} "
        f"\\cmidrule(lr){{{3 + n}-{2 + 2 * n}}}",
    ]
    method_hdr = " & ".join(TABLE5_COL_LABELS[m] for m in methods)
    lines.append(f"Family & Original & {method_hdr} & {method_hdr} \\\\")
    lines.append("\\midrule")

    # Per-family rows, plus columns of values for the geomean row.
    cr_cols: list[list[float | None]] = [[] for _ in methods]
    t_cols: list[list[float | None]] = [[] for _ in methods]
    for domain in TABLE5_DOMAINS:
        if domain not in domains:
            continue
        runs = domains[domain].get("runs", {})
        cr_map = aggregate_methods_cr(runs)
        t_map = aggregate_methods_time(runs)
        crs = [cr_map.get(TABLE5_DATA_KEYS[m]) for m in methods]
        ts = [t_map.get(TABLE5_DATA_KEYS[m]) for m in methods]
        # A DNF records the timeout as elapsed; drop that time so it isn't
        # mistaken for a real measurement (keep cr/time over the same set).
        ts = [t if c is not None else None for c, t in zip(crs, ts)]
        for i in range(n):
            cr_cols[i].append(crs[i])
            t_cols[i].append(ts[i])
        label = TABLE5_DOMAIN_LABELS.get(domain, domain)
        original = fmt(initial_size_for_domain(runs), ".0f")
        cr_strs = cells(crs, ".2f", higher_is_better=True)
        t_strs = cells(ts, ".2f", higher_is_better=False)
        lines.append(" & ".join([label, original, *cr_strs, *t_strs]) + " \\\\")

    # Geomean row across families. Only computed for a method that finished
    # *every* family — a geomean over a subset isn't comparable to the others,
    # so a method with any DNF (e.g. babble on hexyl) is left blank here.
    lines.append("\\midrule")
    complete = lambda col: all(v is not None for v in col)
    agg_cr = [geomean_col(col) if complete(col) else None for col in cr_cols]
    agg_t = [geomean_col(col) if complete(col) else None for col in t_cols]
    cr_strs = [("" if v is None else s) for v, s in zip(agg_cr, bold_best(agg_cr, ".2f", True))]
    t_strs = [("" if v is None else s) for v, s in zip(agg_t, bold_best(agg_t, ".2f", False))]
    lines.append(" & ".join(["Geo. mean", "", *cr_strs, *t_strs]) + " \\\\")

    lines += ["\\bottomrule", "\\end{tabular}"]
    return "\n".join(lines)


def _plot_table5(cr_map: dict, t_map: dict, title: str, out_path: Path) -> None:
    """``plot_cr_vs_time`` with table5's method roster wired in."""
    plot_cr_vs_time(
        cr_map, t_map, title, out_path,
        methods=TABLE5_PLOT_METHODS,
        sweep_for_method=TABLE5_SWEEP_FOR_METHOD,
        sweep_point=TABLE5_SWEEP_POINT,
        method_colors=TABLE5_METHOD_COLORS,
        method_plot_labels=TABLE5_METHOD_PLOT_LABELS,
    )


def plot_table5_domain(saved: dict, domain: str, out_path: Path) -> None:
    """CR-vs-time plot for one molecule family."""
    runs = saved["domains"][domain].get("runs", {})
    title = f"{TABLE5_TITLE}\n{TABLE5_DOMAIN_LABELS.get(domain, domain)}"
    _plot_table5(aggregate_methods_cr(runs), aggregate_methods_time(runs), title, out_path)


def plot_table5_geomean(saved: dict, out_path: Path) -> None:
    """CR-vs-time plot using geomeans across the molecule families per key.

    A method that DNFs on any family (e.g. babble, which DNFs on hexyl) is
    dropped from the geomean plot entirely — a geomean over a subset of
    families isn't comparable to one over all of them. This matches the LaTeX
    table, which blanks the geomean cell for any method with a DNF."""
    domains = [d for d in TABLE5_DOMAINS if d in saved["domains"]]
    per_cr = [aggregate_methods_cr(saved["domains"][d].get("runs", {})) for d in domains]
    per_t = [aggregate_methods_time(saved["domains"][d].get("runs", {})) for d in domains]
    keys = {k for m in per_cr for k in m} | {k for m in per_t for k in m}
    # Only plot a method's geomean point if it finished *every* family — a
    # geomean over a subset isn't comparable, so a method with any DNF (e.g.
    # babble on hexyl) is dropped from the geomean plot entirely.
    def _complete_geomean(vals: list) -> float | None:
        return geomean_col(vals) if all(v is not None for v in vals) else None

    cr_map = {k: _complete_geomean([m.get(k) for m in per_cr]) for k in keys}
    t_map = {k: _complete_geomean([m.get(k) for m in per_t]) for k in keys}
    _plot_table5(cr_map, t_map, f"{TABLE5_TITLE}\nGeo. mean across families", out_path)


def render_table5(saved: dict) -> None:
    """Write per-family + geomean CR-vs-time PNGs for table5 under figures/table5/."""
    domain_dir = FIGURES_DIR / "table5"
    domain_dir.mkdir(parents=True, exist_ok=True)
    for domain in TABLE5_DOMAINS:
        if domain not in saved["domains"]:
            continue
        plot_path = domain_dir / f"{domain.split(':', 1)[1]}.png"
        plot_table5_domain(saved, domain, plot_path)
        print(f"wrote {plot_path}", file=sys.stderr)
    geomean_path = FIGURES_DIR / "table5_geomean.png"
    plot_table5_geomean(saved, geomean_path)
    print(f"wrote {geomean_path}", file=sys.stderr)


def main() -> None:
    """Render each table as a LaTeX file and PNG plot under ``figures/``."""
    argparse.ArgumentParser(description=__doc__).parse_args()

    FIGURES_DIR.mkdir(exist_ok=True)
    for table in (1, 2, 3, 4):
        path = RESULTS_DIR / f"table{table}.json"
        if not path.exists():
            print(f"skipping table{table}: {path} not present", file=sys.stderr)
            continue
        with open(path) as f:
            saved = json.load(f)
        tex_path = FIGURES_DIR / f"table{table}.tex"
        tex_path.write_text(f"% source: {path}\n" + render(saved, table) + "\n")
        print(f"wrote {tex_path}", file=sys.stderr)
        # Presentation variants of the stacked-abstraction tables: no size
        # columns, method columns tinted to match the plots.
        if table in (3, 4):
            pres_path = FIGURES_DIR / f"table{table}_presentation.tex"
            pres_path.write_text(
                f"% source: {path}\n"
                + render(saved, table, presentation=True) + "\n")
            print(f"wrote {pres_path}", file=sys.stderr)
        # Drop the previous single-PNG-per-table output; the per-domain
        # files below replace it. Silent if it was already gone.
        stale = FIGURES_DIR / f"table{table}.png"
        stale.unlink(missing_ok=True)
        domain_dir = FIGURES_DIR / f"table{table}"
        domain_dir.mkdir(exist_ok=True)
        for domain in domains_for_table(table):
            if domain not in saved["domains"]:
                continue
            plot_path = domain_dir / f"{domain}.png"
            plot_domain(saved, table, domain, plot_path)
            print(f"wrote {plot_path}", file=sys.stderr)
        geomean_path = FIGURES_DIR / f"table{table}_geomean.png"
        plot_geomean(saved, table, geomean_path)
        print(f"wrote {geomean_path}", file=sys.stderr)

    # Table 5 (molecule scramble subset) has a different roster/shape, so it
    # gets its own plot path — PNGs only, no LaTeX table.
    table5_path = RESULTS_DIR / "table5.json"
    if table5_path.exists():
        with open(table5_path) as f:
            saved = json.load(f)
        tex_path = FIGURES_DIR / "table5.tex"
        tex_path.write_text(f"% source: {table5_path}\n" + render_table5_tex(saved) + "\n")
        print(f"wrote {tex_path}", file=sys.stderr)
        render_table5(saved)
    else:
        print(f"skipping table5: {table5_path} not present", file=sys.stderr)


if __name__ == "__main__":
    main()

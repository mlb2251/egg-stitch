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
sys.path.insert(0, str(Path(__file__).resolve().parent))
from render_molecules import render_all as render_molecules  # noqa: E402
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
    TABLE7_BFS_SWEEP,
    TABLE7_DOMAINS,
    TABLE7_SMC_SWEEP,
)
from dataclasses import dataclass  # noqa: E402

PROJECT_ROOT = Path(__file__).resolve().parent.parent
RESULTS_DIR = PROJECT_ROOT / "results"
FIGURES_DIR = PROJECT_ROOT / "figures"

# Tables 1/3 (DSR runs) only include domains that have a babble equational
# theory; tables 2/4 (no-DSR runs) include text/logo/towers as well.
TABLE_DOMAINS_DSR = ["nuts-bolts", "dials", "wheels", "furniture", "list", "physics"]
TABLE_DOMAINS_NO_DSR = TABLE_DOMAINS_DSR + ["text", "logo", "towers"]


def domains_for_table(table: int) -> list[str]:
    return TABLE_DOMAINS_DSR if table in TABLES_WITH_EGRAPH_MIN else TABLE_DOMAINS_NO_DSR


def methods_for_table(table: int) -> list[str]:
    """Ordered method columns for the table.

    DSR tables (1 & 3) drop Stitch (it can't take DSRs) and add the
    dsrs-only-at-start "BFS@start" baseline; no-DSR tables (2 & 4) keep Stitch
    and have no baseline.
    """
    if table in TABLES_WITH_EGRAPH_MIN:
        return ["enum", "smc", BASELINE_METHOD, "babble"]
    return ["enum", "smc", "babble", "stitch"]
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
# DSR tables (1 & 3) drop Stitch (it can't take DSRs) and add a
# "dsrs-only-at-start" baseline: best-first that canonicalises with the DSRs
# once instead of keeping them live (the "BFS@start" column, same as table5).
BASELINE_METHOD = "enum-dsrs-at-start"
# Table cells use the bare search-strategy name; plot legends spell out the
# E-Stitch prefix so each series is unambiguous standalone.
METHOD_LABELS = {"enum": "BFS", "smc": "SMC", "babble": "babble",
                 "stitch": "Stitch", BASELINE_METHOD: "BFS/MT"}
METHOD_PLOT_LABELS = {
    "enum": "E-Stitch: BFS",
    "smc": "E-Stitch: SMC",
    "babble": "babble",
    "stitch": "Stitch",
    BASELINE_METHOD: "E-Stitch: BFS (DSRs at start)",
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
    BASELINE_METHOD: BASELINE_METHOD,
}
TABLE_TITLES = {
    1: "Compression Using Rewrites",
    2: "Compression Without Rewrites",
    3: "Compression Using Rewrites, Stacked Abstractions",
    4: "Compression Without Rewrites, Stacked Abstractions",
}
# Tables that include an "E-graph min term size" column (runs with DSRs).
TABLES_WITH_EGRAPH_MIN = {1, 3}
# DSR tables (1 & 3) don't run Stitch (it doesn't accept DSRs) and omit the
# Stitch column entirely (see ``render``), so neither borrows numbers from a
# no-DSR counterpart. Kept as a map in case a future table wants to.
NO_DSR_COUNTERPART: dict[int, int] = {}
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


# Plot uses a "line" variant of the pastel theme for readability on white. The
# dsrs-only-at-start baseline (DSR tables only, where Stitch isn't plotted)
# gets its own color past the four base series.
METHOD_COLORS = {m: line_color(i) for i, m in enumerate(METHODS)}
METHOD_COLORS[BASELINE_METHOD] = line_color(4)
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

    ``crs``/``ts`` hold the single table-point value per method, in
    ``methods_for_table`` order; on no-DSR tables the Stitch entry is spliced
    from the no-DSR counterpart. Shared by the LaTeX table and the bar-chart
    renderers.
    """
    domains = saved["domains"]
    methods = methods_for_table(table)
    stitch_no_dsr = _stitch_no_dsr_maps(table)
    rows = []
    for domain in domains_for_table(table):
        if domain not in domains:
            continue
        runs = domains[domain].get("runs", {})
        cr_map = aggregate_methods_cr(runs)
        t_map = aggregate_methods_time(runs)
        crs = [cr_map.get(TABLE_DATA_KEYS[m]) for m in methods]
        ts = [t_map.get(TABLE_DATA_KEYS[m]) for m in methods]
        if "stitch" in methods and domain in stitch_no_dsr:
            si = methods.index("stitch")
            crs[si], ts[si] = stitch_no_dsr[domain]
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
    # The dsrs-only-at-start baseline is an E-Stitch search variant too.
    BASELINE_METHOD: "estitchHighlight",
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
    # DSR tables (1 & 3) drop Stitch and add the dsrs-only-at-start baseline;
    # no-DSR tables (2 & 4) keep Stitch. ``_collect_rows`` returns cells in this
    # same order, so no per-method filtering is needed below.
    methods = methods_for_table(table)
    n = len(methods)
    has_egraph_min = table in TABLES_WITH_EGRAPH_MIN
    show_size = not presentation
    stitch_no_dsr = _stitch_no_dsr_maps(table)

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

        ``crs``/``ts`` are already in ``methods`` order (Stitch dropped on DSR
        tables), so bolding is relative to exactly the displayed columns.
        """
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
        agg_cr = [geomean_col([r[3][i] for r in rows]) for i in range(n)]
        agg_t = [geomean_col([r[4][i] for r in rows]) for i in range(n)]
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
    plot_cr_vs_time(cr_map, t_map, title, out_path, stitch_starred=starred,
                    methods=methods_for_table(table))


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
                    out_path, stitch_starred=starred,
                    methods=methods_for_table(table))


# ─── family tables (table5/table7): same shape, different rosters ─────────────
# Both share table5's layout — the two ours sweeps + single-point baselines, no
# Stitch (can't take DSRs) and no e-graph-min table column — but differ in their
# domains and which non-sweep methods they carry, so each is described by a
# FamilySpec consumed by render_family_tex / render_family below.
@dataclass(frozen=True)
class FamilySpec:
    """Everything render_family_tex / render_family need for one family table.

    ``plot_methods``/``table_methods`` order the columns/series; ``data_keys``
    maps each to the single representative run key (the sweep methods' filled
    plot marker); ``sweep_*`` drive the swept lines.
    """
    title: str
    fig_subdir: str
    domains: list[str]
    domain_labels: dict[str, str]
    plot_methods: list[str]
    table_methods: list[str]
    data_keys: dict[str, str]
    col_labels: dict[str, str]
    sweep_for_method: dict[str, tuple[int, ...]]
    sweep_point: dict[str, int]
    method_colors: dict[str, object]
    method_plot_labels: dict[str, str]

    @classmethod
    def estitch_roster(
        cls,
        *,
        title: str,
        fig_subdir: str,
        domains: list[str],
        domain_labels: dict[str, str],
        enum_point: int,
        enum_sweep: tuple[int, ...],
        smc_sweep: tuple[int, ...],
        extras: list[tuple[str, str, str]],
    ) -> "FamilySpec":
        """Build a spec for the standard E-Stitch roster: the enum (BFS) and smc
        (SMC) sweeps plus the dsrs-only-at-start baseline (BFS/MT), followed by
        the family-specific ``extras`` -- each a ``(method_key, col_label,
        plot_label)`` triple (babble and/or the no-rules Enum baseline BFS/NR).
        The shared three methods -- their order, labels, colors, and the smc
        representative point -- live here so the per-family specs can't drift on
        them; only the bits that genuinely differ are args.
        """
        methods = ["enum", "smc", "enum-dsrs-at-start", *(e[0] for e in extras)]
        # Base methods take color slots 0/1/3; extras fill 2 then 4, 5, ... so a
        # single-extra roster (table7) keeps its original slot-2 color.
        extra_slots = [2, 4, 5][: len(extras)]
        return cls(
            title=title,
            fig_subdir=fig_subdir,
            domains=domains,
            domain_labels=domain_labels,
            # The two sweeps, then the single-point methods. Each single-point
            # method keys on its own data label so plot_cr_vs_time finds it directly.
            plot_methods=methods,
            table_methods=methods,
            data_keys={
                "enum": f"enum-{enum_point}",
                "smc": f"smc-{TABLE_SMC_PARTICLES}",
                "enum-dsrs-at-start": "enum-dsrs-at-start",
                **{e[0]: e[0] for e in extras},
            },
            col_labels={
                "enum": "BFS",
                "smc": "SMC",
                "enum-dsrs-at-start": "BFS/MT",
                **{e[0]: e[1] for e in extras},
            },
            sweep_for_method={"enum": enum_sweep, "smc": smc_sweep},
            sweep_point={"enum": enum_point, "smc": TABLE_SMC_PARTICLES},
            method_colors={
                "enum": line_color(0),
                "smc": line_color(1),
                "enum-dsrs-at-start": line_color(3),
                **{e[0]: line_color(slot) for e, slot in zip(extras, extra_slots)},
            },
            method_plot_labels={
                "enum": "E-Stitch: BFS",
                "smc": "E-Stitch: SMC",
                "enum-dsrs-at-start": "E-Stitch: BFS (DSRs at start)",
                **{e[0]: e[2] for e in extras},
            },
        )


# table5: molecule scramble subset. The two ours sweeps (enum extended to 100k),
# the dsrs-only-at-start baseline (a single best-first point), babble, and the
# no-rules Enum baseline (BFS/NR).
TABLE5_SPEC = FamilySpec.estitch_roster(
    title="Molecule Scramble Compression (DSRs)",
    fig_subdir="table5",
    domains=TABLE5_DOMAINS,
    domain_labels={
        "molecules:hexyl": "Hexyl",
        "molecules:ester": "Ester",
        "molecules:glycol": "Glycol",
    },
    enum_point=TABLE5_ENUM_POINT,
    enum_sweep=TABLE5_BFS_SWEEP,
    smc_sweep=SMC_PARTICLE_SWEEP,
    extras=[
        ("babble", "babble", "babble"),
        ("enum-baseline", "BFS/NR", "E-Stitch: BFS (no rules)"),
    ],
)

# table7: EPFL circuits with the factoring DSRs. Same roster as table5 (babble +
# a no-rules Enum baseline "enum-baseline"), so the three-way baseline/live/
# at-start contrast plus babble all show. babble runs via its ``circuits`` binary
# (boolean and/or/not over ``$N`` inputs). Enum DNFs here (best-first can't search
# the rule-saturated e-graph; see TABLE7_BFS_SWEEP).
TABLE7_SPEC = FamilySpec.estitch_roster(
    title="EPFL Circuit Compression (Factoring DSRs)",
    fig_subdir="table7",
    domains=TABLE7_DOMAINS,
    domain_labels={
        "epfl-circuits:multiplier": "Multiplier",
        "epfl-circuits:square": "Square",
        "epfl-circuits:log2": "Log2",
        "epfl-circuits:hyp": "Hypotenuse",
        "epfl-circuits:voter": "Voter",
    },
    enum_point=TABLE_BFS_STEPS,
    enum_sweep=TABLE7_BFS_SWEEP,
    smc_sweep=TABLE7_SMC_SWEEP,
    extras=[
        ("babble", "babble", "babble"),
        ("enum-baseline", "BFS/NR", "E-Stitch: BFS (no rules)"),
    ],
)


# Unit names for the two sweep methods, used only in the kick-down notices.
SWEEP_UNIT = {"enum": "steps", "smc": "particles"}


def _kicked_data_keys(saved: dict, spec: "FamilySpec") -> tuple[dict[str, str], list[str]]:
    """Pick the representative table key for each method, kicking series methods
    down their sweep when the configured point DNFs at the geomean.

    A series method (Enum/SMC) normally shows its configured operating point
    (``spec.sweep_point``). If that point DNFs on any family — so its geomean is
    blank — step down the sweep to the highest point that finishes *every*
    family and show that instead. Returns ``(data_keys, notices)`` where
    ``data_keys`` overrides ``spec.data_keys`` for any kicked-down method and
    ``notices`` is a human-readable line per kick-down (empty if none)."""
    domains = [d for d in spec.domains if d in saved["domains"]]
    cr_by_domain = {
        d: aggregate_methods_cr(saved["domains"][d].get("runs", {})) for d in domains
    }

    def finishes_every_family(key: str) -> bool:
        return bool(domains) and all(cr_by_domain[d].get(key) is not None for d in domains)

    data_keys = dict(spec.data_keys)
    notices: list[str] = []
    for method, sweep in spec.sweep_for_method.items():
        point = spec.sweep_point[method]
        if finishes_every_family(f"{method}-{point}"):
            continue
        # Highest sweep point at or below the configured one that finishes.
        chosen = next(
            (v for v in sorted((s for s in sweep if s <= point), reverse=True)
             if finishes_every_family(f"{method}-{v}")),
            None,
        )
        if chosen is None:
            continue  # nothing finishes; leave the configured point to render DNF
        data_keys[method] = f"{method}-{chosen}"
        notices.append(
            f"{spec.fig_subdir} {spec.col_labels[method]}: kicked down "
            f"{point} -> {chosen} {SWEEP_UNIT[method]} (geomean DNF at {point})"
        )
    return data_keys, notices


def render_family_tex(saved: dict, spec: "FamilySpec") -> tuple[str, list[str]]:
    """Return ``(latex_tabular, notices)`` for a family table (table5/table7):
    one row per family × method, with Compression Ratio and Time (s) groups and
    a geomean row. ``notices`` lists any series-method sweep-point kick-downs.

    No e-graph-min or Stitch columns (these tables have neither). A method that
    timed out / OOM'd on any run of a family has no comparable number, so it's
    rendered ``DNF`` and excluded from that row's bolding and the geomean. A
    series method (Enum/SMC) whose representative sweep point DNFs at the geomean
    is first kicked down to the highest point that finishes every family (see
    ``_kicked_data_keys``)."""
    domains = saved["domains"]
    methods = spec.table_methods
    n = len(methods)
    data_keys, notices = _kicked_data_keys(saved, spec)

    def cells(values: list[float | None], fmt_spec: str, higher_is_better: bool) -> list[str]:
        """bold_best, but render a missing value (DNF) as ``DNF``."""
        out = bold_best(values, fmt_spec, higher_is_better)
        return [("DNF" if v is None else s) for v, s in zip(values, out)]

    col_spec = "l r " + ("r" * n) + " " + ("r" * n)
    lines = [
        f"% {spec.title}: generated from results JSON",
        "\\begin{tabular}{" + col_spec + "}",
        "\\toprule",
        "& \\multicolumn{1}{c}{Size} "
        f"& \\multicolumn{{{n}}}{{c}}{{Compression Ratio}} "
        f"& \\multicolumn{{{n}}}{{c}}{{Time (s)}} \\\\",
        f"\\cmidrule(lr){{2-2}} \\cmidrule(lr){{3-{2 + n}}} "
        f"\\cmidrule(lr){{{3 + n}-{2 + 2 * n}}}",
    ]
    method_hdr = " & ".join(spec.col_labels[m] for m in methods)
    lines.append(f"Family & Original & {method_hdr} & {method_hdr} \\\\")
    lines.append("\\midrule")

    # Per-family rows, plus columns of values for the geomean row.
    cr_cols: list[list[float | None]] = [[] for _ in methods]
    t_cols: list[list[float | None]] = [[] for _ in methods]
    for domain in spec.domains:
        if domain not in domains:
            continue
        runs = domains[domain].get("runs", {})
        cr_map = aggregate_methods_cr(runs)
        t_map = aggregate_methods_time(runs)
        crs = [cr_map.get(data_keys[m]) for m in methods]
        ts = [t_map.get(data_keys[m]) for m in methods]
        # A DNF records the timeout as elapsed; drop that time so it isn't
        # mistaken for a real measurement (keep cr/time over the same set).
        ts = [t if c is not None else None for c, t in zip(crs, ts)]
        for i in range(n):
            cr_cols[i].append(crs[i])
            t_cols[i].append(ts[i])
        label = spec.domain_labels.get(domain, domain)
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
    return "\n".join(lines), notices


def _plot_family(cr_map: dict, t_map: dict, title: str, out_path: Path,
                 spec: "FamilySpec") -> None:
    """``plot_cr_vs_time`` with a family table's method roster wired in."""
    plot_cr_vs_time(
        cr_map, t_map, title, out_path,
        methods=spec.plot_methods,
        sweep_for_method=spec.sweep_for_method,
        sweep_point=spec.sweep_point,
        method_colors=spec.method_colors,
        method_plot_labels=spec.method_plot_labels,
    )


def plot_family_domain(saved: dict, spec: "FamilySpec", domain: str, out_path: Path) -> None:
    """CR-vs-time plot for one family member."""
    runs = saved["domains"][domain].get("runs", {})
    title = f"{spec.title}\n{spec.domain_labels.get(domain, domain)}"
    _plot_family(aggregate_methods_cr(runs), aggregate_methods_time(runs), title, out_path, spec)


def plot_family_geomean(saved: dict, spec: "FamilySpec", out_path: Path) -> None:
    """CR-vs-time plot using geomeans across the families per key.

    A method that DNFs on any family (e.g. babble on table5's hexyl) is dropped
    from the geomean plot entirely — a geomean over a subset of families isn't
    comparable to one over all of them. This matches the LaTeX table, which
    blanks the geomean cell for any method with a DNF."""
    domains = [d for d in spec.domains if d in saved["domains"]]
    per_cr = [aggregate_methods_cr(saved["domains"][d].get("runs", {})) for d in domains]
    per_t = [aggregate_methods_time(saved["domains"][d].get("runs", {})) for d in domains]
    keys = {k for m in per_cr for k in m} | {k for m in per_t for k in m}
    # Only plot a method's geomean point if it finished *every* family — a
    # geomean over a subset isn't comparable, so a method with any DNF is
    # dropped from the geomean plot entirely.
    def _complete_geomean(vals: list) -> float | None:
        return geomean_col(vals) if all(v is not None for v in vals) else None

    cr_map = {k: _complete_geomean([m.get(k) for m in per_cr]) for k in keys}
    t_map = {k: _complete_geomean([m.get(k) for m in per_t]) for k in keys}
    _plot_family(cr_map, t_map, f"{spec.title}\nGeo. mean across families", out_path, spec)


def render_family(saved: dict, spec: "FamilySpec") -> None:
    """Write per-family + geomean CR-vs-time PNGs under figures/<fig_subdir>/."""
    domain_dir = FIGURES_DIR / spec.fig_subdir
    domain_dir.mkdir(parents=True, exist_ok=True)
    for domain in spec.domains:
        if domain not in saved["domains"]:
            continue
        plot_path = domain_dir / f"{domain.split(':', 1)[1]}.png"
        plot_family_domain(saved, spec, domain, plot_path)
        print(f"wrote {plot_path}", file=sys.stderr)
    geomean_path = FIGURES_DIR / f"{spec.fig_subdir}_geomean.png"
    plot_family_geomean(saved, spec, geomean_path)
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

    # Tables 5 & 7 (molecule scramble / EPFL circuit subsets) share a roster
    # shape distinct from tables 1-4 (family rows, DSR baselines, no Stitch), so
    # they go through the FamilySpec renderers: a LaTeX table plus per-family and
    # geomean PNGs.
    notices: list[str] = []  # series-method sweep kick-downs, surfaced at the end
    for spec in (TABLE5_SPEC, TABLE7_SPEC):
        path = RESULTS_DIR / f"{spec.fig_subdir}.json"
        if not path.exists():
            print(f"skipping {spec.fig_subdir}: {path} not present", file=sys.stderr)
            continue
        with open(path) as f:
            saved = json.load(f)
        tex_path = FIGURES_DIR / f"{spec.fig_subdir}.tex"
        tex, spec_notices = render_family_tex(saved, spec)
        notices += spec_notices
        tex_path.write_text(f"% source: {path}\n" + tex + "\n")
        print(f"wrote {tex_path}", file=sys.stderr)
        render_family(saved, spec)
        # table5 also gets per-family molecule scramble trajectory figures.
        if spec is TABLE5_SPEC:
            render_molecules(saved, FIGURES_DIR / "molecules")

    # Loudly surface every sweep-point kick-down so a reduced operating point in
    # a table is never silently mistaken for the configured one.
    if notices:
        bar = "!" * 78
        print(f"\n{bar}\n!! SWEEP-POINT KICK-DOWNS ({len(notices)}):", file=sys.stderr)
        for notice in notices:
            print(f"!!   {notice}", file=sys.stderr)
        print(f"{bar}", file=sys.stderr)


if __name__ == "__main__":
    main()

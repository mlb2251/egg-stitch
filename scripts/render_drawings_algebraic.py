#!/usr/bin/env python3
"""Render the standalone drawings-algebraic experiment (not a paper table).

Reads ``results/table_drawings_algebraic.json`` (produced by
``expts.table_drawings_algebraic.table_drawings_algebraic()``) and writes
``figures/table_drawings_algebraic.tex`` plus the per-domain and geomean PNGs
under ``figures/table_drawings_algebraic/``, reusing the FamilySpec machinery in
render_tables.py. Kept off the main render_tables path since the experiment isn't
in the paper for now.
"""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from render_tables import (  # noqa: E402
    FIGURES_DIR,
    RESULTS_DIR,
    FamilySpec,
    render_family,
    render_family_tex,
)
from expts.tables import BFS_STEP_SWEEP, SMC_PARTICLE_SWEEP, TABLE_BFS_STEPS  # noqa: E402

# cogsci drawing domains with our algebraic drawing DSRs. Same roster shape as
# table5/7 -- the two ours sweeps (live DSRs) and the dsrs-only-at-start baseline
# -- but no fourth method: babble can't parse the constant_folding/matmul rules,
# so it has no column here. The live-vs-at-start contrast is BFS vs BFS/MT.
DOMAINS = [f"drawings:{d}" for d in ("nuts-bolts", "dials", "wheels", "furniture")]
SPEC = FamilySpec.estitch_roster(
    title="Drawing-Domain Compression (Algebraic DSRs)",
    fig_subdir="table_drawings_algebraic",
    domains=DOMAINS,
    domain_labels={
        "drawings:nuts-bolts": "Nuts \\& Bolts",
        "drawings:dials": "Dials",
        "drawings:wheels": "Wheels",
        "drawings:furniture": "Furniture",
    },
    enum_point=TABLE_BFS_STEPS,
    enum_sweep=BFS_STEP_SWEEP,
    smc_sweep=SMC_PARTICLE_SWEEP,
    extras=[],  # no babble/no-rules columns: babble can't parse the algebraic rules
)


def main() -> None:
    """Render the drawings-algebraic .tex + PNGs from its results JSON."""
    path = RESULTS_DIR / f"{SPEC.fig_subdir}.json"
    if not path.exists():
        sys.exit(f"missing {path} (run expts.table_drawings_algebraic first)")
    with open(path) as f:
        saved = json.load(f)
    tex, notices = render_family_tex(saved, SPEC)
    tex_path = FIGURES_DIR / f"{SPEC.fig_subdir}.tex"
    tex_path.write_text(f"% source: {path}\n" + tex + "\n")
    print(f"wrote {tex_path}", file=sys.stderr)
    render_family(saved, SPEC)
    for notice in notices:
        print(f"!! sweep-point kick-down: {notice}", file=sys.stderr)


if __name__ == "__main__":
    main()

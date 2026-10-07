#!/usr/bin/env python3
"""Delete the cached E-Stitch results that appear as cells in the tables in
``TABLES``, so that re-running those tables recomputes only them.

Babble/Stitch caches and the rest of the BFS/SMC sweeps are kept. The files are
checked into git, so ``git restore results/`` undoes this.
"""

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from expts.folders import SUMMARY_RESULTS_DIR  # noqa: E402
from expts.render_common import reported_sweep_point  # noqa: E402
from expts.tables import (  # noqa: E402
    SMC_PARTICLE_SWEEP,
    MOLECULES_BFS_SWEEP,
    MOLECULES_ENUM_POINT,
    CIRCUITS_BFS_SWEEP,
    CIRCUITS_SMC_SWEEP,
    TABLE_BFS_STEPS,
    TABLE_SMC_PARTICLES,
)

# table -> (enum point, enum sweep, smc sweep). The babble comparisons never kick down from
# their configured point, so their sweeps are just that point.
TABLES = {
    "babble_comparison_rewrites_single": (TABLE_BFS_STEPS, (TABLE_BFS_STEPS,), (TABLE_SMC_PARTICLES,)),
    "babble_comparison_no_rewrites_single": (TABLE_BFS_STEPS, (TABLE_BFS_STEPS,), (TABLE_SMC_PARTICLES,)),
    "babble_comparison_rewrites": (TABLE_BFS_STEPS, (TABLE_BFS_STEPS,), (TABLE_SMC_PARTICLES,)),
    "babble_comparison_no_rewrites": (TABLE_BFS_STEPS, (TABLE_BFS_STEPS,), (TABLE_SMC_PARTICLES,)),
    "molecules": (MOLECULES_ENUM_POINT, MOLECULES_BFS_SWEEP, SMC_PARTICLE_SWEEP),
    "circuits": (TABLE_BFS_STEPS, CIRCUITS_BFS_SWEEP, CIRCUITS_SMC_SWEEP),
}


def table_labels(table: str) -> set[str]:
    """Cache labels of the E-Stitch cells ``table`` shows: the configured BFS/SMC
    points, the point the current results kicked down to if any, and the two
    single-point baselines."""
    enum_point, enum_sweep, smc_sweep = TABLES[table]
    labels = {f"enum-{enum_point}", f"smc-{TABLE_SMC_PARTICLES}",
              "enum-dsrs-at-start", "enum-baseline"}
    summary = SUMMARY_RESULTS_DIR / f"{table}.json"
    if summary.exists():
        domain_runs = [d["runs"] for d in json.loads(summary.read_text())["domains"].values()]
        for method, sweep, point in [("enum", enum_sweep, enum_point),
                                     ("smc", smc_sweep, TABLE_SMC_PARTICLES)]:
            labels.add(f"{method}-{reported_sweep_point(domain_runs, method, sweep, point)}")
    return labels


def main() -> None:
    """Delete every table's E-Stitch cell caches, printing each one removed."""
    for table in TABLES:
        for label in sorted(table_labels(table)):
            path = SUMMARY_RESULTS_DIR / table / f"{label}.json"
            if path.exists():
                path.unlink()
                print(f"deleted {path.relative_to(SUMMARY_RESULTS_DIR.parent)}")


if __name__ == "__main__":
    main()

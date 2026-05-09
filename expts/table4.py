"""Table 4 experiment: same as Table 2 but with ``num_abstractions=20``.

Thin wrapper around :func:`expts.table2.table2`; switches the results folder,
output filename, and title, forwards ``num_abstractions=20`` so each run
stacks 20 abstractions, and patches :data:`expts.bench.OURS_REBUILD_EGRAPH`
so egg-stitch rebuilds the e-graph between successive abstractions.
"""

from pathlib import Path

from . import bench
from .table2 import print_table2, table2

NUM_ABSTRACTIONS = 20

TABLE4_TITLE = (
    f"Table 4: Ours (SMC and Enum) vs Babble vs Stitch on benchmarks "
    f"without DSRs, stacking {NUM_ABSTRACTIONS} abstractions"
)


def table4(**kwargs) -> Path:
    """Run the Table 2 setup with ``num_abstractions={NUM_ABSTRACTIONS}``."""
    kwargs.setdefault("num_abstractions", NUM_ABSTRACTIONS)
    kwargs.setdefault("folder_prefix", "table4")
    kwargs.setdefault("output_name", "table4.json")
    kwargs.setdefault("title", TABLE4_TITLE)
    saved = bench.OURS_REBUILD_EGRAPH
    bench.OURS_REBUILD_EGRAPH = True
    try:
        return table2(**kwargs)
    finally:
        bench.OURS_REBUILD_EGRAPH = saved


def print_table4(path: str | Path) -> None:
    """Pretty-print a saved Table 4 JSON (reuses Table 2's renderer)."""
    print_table2(path)

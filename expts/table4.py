"""Table 4 experiment: same as Table 2 but with ``num_abstractions=20``.

Thin wrapper that delegates to :func:`expts.table2.table2`, swapping the
results folder, output filename, title, and forwarding
``num_abstractions=20`` so each run stacks 20 abstractions sequentially
(rather than the single abstraction used by Table 2).
"""

from pathlib import Path

from .table2 import print_table2, table2

NUM_ABSTRACTIONS = 20

TABLE4_TITLE = (
    f"Table 4: Ours (SMC and Enum) vs Babble vs Stitch on benchmarks "
    f"without DSRs, stacking {NUM_ABSTRACTIONS} abstractions"
)


def table4(**kwargs) -> Path:
    """Run the Table 2 setup with ``num_abstractions={NUM_ABSTRACTIONS}``.

    Any keyword arguments accepted by :func:`table2` can be passed through.
    """
    kwargs.setdefault("num_abstractions", NUM_ABSTRACTIONS)
    kwargs.setdefault("rebuild_egraph", True)
    kwargs.setdefault("folder_prefix", "table4")
    kwargs.setdefault("output_name", "table4.json")
    kwargs.setdefault("title", TABLE4_TITLE)
    return table2(**kwargs)


def print_table4(path: str | Path) -> None:
    """Pretty-print a saved Table 4 JSON (reuses Table 2's renderer)."""
    print_table2(path)

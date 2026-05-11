"""Table 3 experiment: same as Table 1 but with ``num_abstractions=20``.

Thin wrapper around :func:`expts.table1.table1`; switches the results folder,
output filename, and title, and forwards ``num_abstractions=20``.
"""

from pathlib import Path

from .table1 import print_table1, table1

NUM_ABSTRACTIONS = 20

TABLE3_TITLE = (
    f"Table 3: Ours (SMC and Enum) vs Babble on benchmarks with "
    f"domain-specific rewrites, stacking {NUM_ABSTRACTIONS} abstractions"
)


def table3(**kwargs) -> Path:
    """Run the Table 1 setup with ``num_abstractions={NUM_ABSTRACTIONS}``."""
    kwargs.setdefault("num_abstractions", NUM_ABSTRACTIONS)
    kwargs.setdefault("folder_prefix", "table3")
    kwargs.setdefault("output_name", "table3.json")
    kwargs.setdefault("title", TABLE3_TITLE)
    return table1(**kwargs)


def print_table3(path: str | Path) -> None:
    """Pretty-print a saved Table 3 JSON (reuses Table 1's renderer)."""
    print_table1(path)

"""Table 1 experiment: compare Ours (Enum + SMC) against babble on the four
cogsci drawing domains plus the dreamcoder benchmarks that ship with DSRs.

Runs each method on every domain *with* rewrite rules turned on, stores
results as raw per-file records (``list[PerFileResult]`` per method-repeat)
in a single JSON under ``results/`` (checked into git), and offers
``print_table1`` to render a text table matching the paper layout.
"""

from pathlib import Path

from ._table_common import NUM_RUNS, print_table, run_table
from .run_models import Babble, OursBf, OursSmc

# Order matches the Table 1 screenshot, with the dreamcoder benchmarks that
# ship with DSRs appended after the four cogsci drawing domains. text/logo/
# towers are excluded: babble has no equational theory for them, so a "with
# DSRs" comparison is not defined.
TABLE1_DOMAINS = ["nuts-bolts", "dials", "wheels", "furniture", "list", "physics"]


DEFAULT_TABLE1_TITLE = "Table 1: Ours (SMC and Enum) vs Babble on benchmarks with domain-specific rewrites"


def table1(
    *,
    num_abstractions: int = 1,
    folder_prefix: str = "table1",
    output_name: str = "table1.json",
    title: str = DEFAULT_TABLE1_TITLE,
    enum: OursBf = OursBf(),
    smc: OursSmc = OursSmc(),
    babble: Babble = Babble(),
) -> Path:
    """Run Enum, SMC, and babble on the Table 1 domains with DSRs.

    Each runner is a dataclass instance carrying its own hyperparameters
    (``num_steps``, ``num_particles``, ``temperature``, …). Pass overrides
    as kwargs at construction — e.g. ``smc=OursSmc(num_steps=50)`` —
    rather than mutating module state.
    """
    return run_table(
        domains=TABLE1_DOMAINS,
        runners=(("enum", enum), ("smc", smc), ("babble", babble)),
        num_abstractions=num_abstractions,
        use_dsrs=True,
        folder_prefix=folder_prefix,
        output_name=output_name,
        title=title,
        show_egraph_min=True,
    )


def print_table1(path: str | Path) -> None:
    """Pretty-print a saved Table 1 JSON in the layout from the paper."""
    print_table(
        path,
        domains=TABLE1_DOMAINS,
        default_title=DEFAULT_TABLE1_TITLE,
        show_egraph_min=True,
    )

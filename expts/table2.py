"""Table 2 experiment: Ours (Enum + SMC) vs babble vs Stitch, no DSRs.

Same cogsci domains as Table 1 plus the dreamcoder benchmarks without DSRs
(text/logo/towers); every method runs *without* any domain-specific
rewrites, and Stitch is included (Table 1 uses DSRs, which Stitch doesn't
accept). Results land at ``results/table2.json`` (checked into git).
"""

from pathlib import Path

from ._table_common import print_table, run_table
from .run_models import Babble, OursBf, OursSmc, Stitch

# Table 2 is the no-DSR comparison, so it includes the dreamcoder domains
# without rewrite files (text/logo/towers) in addition to everything in
# Table 1.
TABLE2_DOMAINS = ["nuts-bolts", "dials", "wheels", "furniture", "list", "physics", "text", "logo", "towers"]


DEFAULT_TABLE2_TITLE = "Table 2: Ours (SMC and Enum) vs Babble vs Stitch on benchmarks without DSRs"


def table2(
    *,
    num_abstractions: int = 1,
    folder_prefix: str = "table2",
    output_name: str = "table2.json",
    title: str = DEFAULT_TABLE2_TITLE,
    enum: OursBf = OursBf(),
    smc: OursSmc = OursSmc(),
    babble: Babble = Babble(),
    stitch: Stitch = Stitch(),
) -> Path:
    """Run Enum, SMC, babble, and Stitch on the Table 2 domains with no DSRs."""
    return run_table(
        domains=TABLE2_DOMAINS,
        runners=(("enum", enum), ("smc", smc), ("babble", babble), ("stitch", stitch)),
        num_abstractions=num_abstractions,
        use_dsrs=False,
        folder_prefix=folder_prefix,
        output_name=output_name,
        title=title,
        show_egraph_min=False,
    )


def print_table2(path: str | Path) -> None:
    """Pretty-print a saved Table 2 JSON."""
    print_table(
        path,
        domains=TABLE2_DOMAINS,
        default_title=DEFAULT_TABLE2_TITLE,
        show_egraph_min=False,
    )

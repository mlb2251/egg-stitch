"""Per-file result record shared by all compression methods.

A :class:`PerFileResult` is the row produced for a single input file by one
tool; the table runners save a ``list[PerFileResult]`` per (method, domain,
repeat); ``scripts/render_tables.py`` aggregates at display time. Cogsci domains have one file per domain, so their
list is length 1; dreamcoder domains have many.
"""

from dataclasses import asdict, dataclass
from math import isnan


@dataclass
class PerFileResult:
    """Compression result for one (method, domain, input file)."""

    method: str
    """One of ``"enum"``, ``"smc"``, ``"babble"``, ``"stitch"``."""

    domain: str
    """The benchmark domain name (e.g. ``"dials"``, ``"list"``)."""

    file: str
    """Stem of the input file (e.g. ``"dials"`` for cogsci, ``"...bench003..."`` for DC)."""

    initial_cost: int
    """AST size of this file before any compression is applied."""

    final_cost: int
    """AST size of this file (plus the abstractions' bodies) after rewriting."""

    compression_ratio: float
    """``initial_cost / final_cost`` for this file."""

    elapsed_secs: float
    """Wall-clock time the tool spent on this file."""

    library: list[str]
    """Human-readable strings for each abstraction learned from this file
    (``"<name>: <body>"``); empty when the tool didn't learn any."""

    egraph_min_term_size: float | None
    """``cost_after_rewrites`` for this file under the DSRs, or None when the
    runner doesn't expose one (i.e. not ours, or DSRs weren't used). Stored as
    None rather than NaN so JSON round-trips cleanly."""

    def to_dict(self) -> dict:
        """Plain-dict representation for JSON serialization."""
        return asdict(self)


def egraph_min_from_bench(cost_after_rewrites: float) -> float | None:
    """Convert a runner's NaN-as-missing sentinel into None for JSON output."""
    return None if isnan(cost_after_rewrites) else cost_after_rewrites

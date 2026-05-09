"""Common result format shared by all compression methods.

A :class:`Result` is the per-(method, domain) record consumed by the table
renderers and the HTML viz. It's produced by :mod:`expts.runner` after
aggregating the per-file :class:`~expts.bench.BenchResult` records.
"""

from dataclasses import asdict, dataclass


@dataclass
class Result:
    """Uniform result record for a single (method, domain) compression run."""

    method: str
    """One of ``"enum"``, ``"smc"``, ``"babble"``, ``"stitch"``."""

    domain: str
    """The benchmark domain name (e.g. ``"dials"``, ``"list"``)."""

    initial_cost: int
    """Corpus AST size before any compression is applied."""

    final_cost: int
    """Corpus AST size after the learned library is applied."""

    compression_ratio: float
    """Compression ratio. For single-file (cogsci) runs this is
    ``initial_cost / final_cost``; for multi-file (dreamcoder) runs it is the
    geometric mean of the per-file ratios (matching the babble paper, Fig. 12)
    and therefore does *not* equal ``initial_cost / final_cost``."""

    elapsed_secs: float
    """Total wall-clock time for the run (sum across files for multi-file)."""

    library: list[str]
    """Human-readable strings for each learned abstraction (``"<name>: <body>"``).
    Empty when the underlying tool didn't learn any abstractions."""

    def to_dict(self) -> dict:
        """Plain-dict representation for JSON serialization."""
        return asdict(self)

    def summary_line(self) -> str:
        """Single-line summary suitable for printing."""
        return (
            f"{self.method}/{self.domain}: "
            f"{self.initial_cost} -> {self.final_cost} "
            f"(ratio {self.compression_ratio:.2f}, time {self.elapsed_secs:.1f}s, "
            f"{len(self.library)} lib)"
        )

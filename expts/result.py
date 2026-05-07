"""Common result format shared by all compression methods (ours/babble/stitch).

Each method wrapper returns a :class:`Result`, which captures the corpus
sizes before/after compression, the learned library, and a wall-clock time
measured uniformly (so cross-method comparison is apples-to-apples).
"""

import math
from dataclasses import dataclass, asdict, field
from typing import Any


@dataclass
class Result:
    """Uniform result record for a single compression run."""

    method: str
    """One of ``"enum"``, ``"smc"``, ``"babble"``, ``"stitch"``."""

    domain: str
    """The cogsci domain name (e.g. ``"dials"``)."""

    initial_cost: int
    """Corpus AST size before any compression is applied."""

    final_cost: int
    """Corpus AST size after the learned library is applied."""

    compression_ratio: float
    """Compression ratio. For single-file runs this is ``initial_cost /
    final_cost``. For multi-file dreamcoder runs aggregated via
    :func:`aggregate_per_file` it is the geometric mean of the per-file
    ratios (matching the babble paper, Fig. 12) and therefore does *not*
    equal ``initial_cost / final_cost`` on the aggregated record."""

    elapsed_secs: float
    """Wall-clock time for the run (subprocess duration)."""

    library: list[str] | None
    """Human-readable strings for each learned abstraction. ``None`` when the
    underlying tool doesn't expose them (e.g. babble's dreamcoder benchmark
    binary, which only emits a count); downstream code that reads this should
    handle ``None`` explicitly rather than silently treating it as empty."""

    extra: dict[str, Any] = field(default_factory=dict)
    """Method-specific fields that don't fit the common schema."""

    def to_dict(self) -> dict:
        """Return the plain-dict representation for JSON serialization."""
        return asdict(self)

    def summary_line(self) -> str:
        """Return a single-line summary suitable for printing."""
        return (
            f"{self.method}/{self.domain}: "
            f"{self.initial_cost} -> {self.final_cost} "
            f"(ratio {self.compression_ratio:.2f}, time {self.elapsed_secs:.1f}s, "
            f"{'?' if self.library is None else len(self.library)} lib)"
        )


def ratio(initial: int, final: int) -> float:
    """Safe division for a compression ratio; returns ``inf`` when ``final == 0``."""
    return float("inf") if final == 0 else initial / final


def aggregate_per_file(per_file: list[Result]) -> Result:
    """Combine per-file Results into a single Result for a multi-file benchmark.

    Costs and time sum across files. ``compression_ratio`` is the geometric
    mean of the per-file ratios — this is how the babble paper (Fig. 12)
    aggregates dreamcoder benchmarks, so reporting it the same way keeps
    our table cells directly comparable. As a consequence, on the returned
    Result ``compression_ratio != initial_cost / final_cost`` in general;
    the per-file values (initial/final cost, ratio, time, library) are
    preserved verbatim under ``extra["per_file"]``.
    """
    assert per_file, "need at least one per-file result to aggregate"
    method = per_file[0].method
    domain = per_file[0].domain
    initial = sum(r.initial_cost for r in per_file)
    final = sum(r.final_cost for r in per_file)
    for r in per_file:
        assert 0 < r.compression_ratio < math.inf, (
            f"per-file compression_ratio={r.compression_ratio} on {r.domain} would make the geomean degenerate"
        )
    geo_cr = math.exp(sum(math.log(r.compression_ratio) for r in per_file) / len(per_file))
    elapsed = sum(r.elapsed_secs for r in per_file)
    library: list[str] = []
    for r in per_file:
        library.extend(r.library)
    return Result(
        method=method,
        domain=domain,
        initial_cost=initial,
        final_cost=final,
        compression_ratio=geo_cr,
        elapsed_secs=elapsed,
        library=library,
        extra={
            "num_files": len(per_file),
            "per_file": [r.to_dict() for r in per_file],
        },
    )

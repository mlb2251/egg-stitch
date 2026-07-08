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

    final_cost: int | None
    """AST size of this file (plus the abstractions' bodies) after rewriting.
    None when the run timed out before producing output."""

    compression_ratio: float | None
    """``initial_cost / final_cost`` for this file; None when timed out."""

    elapsed_secs: float
    """Wall-clock time the tool spent on this file (the timeout when timed out)."""

    library: list[str]
    """Human-readable strings for each abstraction learned from this file
    (``"<name>: <body>"``); empty when the tool didn't learn any."""

    egraph_min_term_size: float | None
    """Raw ``cost_after_rewrites`` for this file under the DSRs, or None when
    the runner doesn't expose one (i.e. not ours, or DSRs weren't used).
    Stored as None rather than NaN so JSON round-trips cleanly. Includes the
    ``(programs …)`` wrapper; renderers subtract 1 to match ``initial_cost``."""

    timed_out: bool = False
    """True when the tool exceeded its wall-clock budget and was killed; the
    cost/ratio fields are then None."""

    cost_at_end_of_each_iter: list[int] | None = None
    """egg-stitch's per-iteration best cost (native units, prior library bodies
    folded in), one entry per learned abstraction. None for other tools or a
    timed-out run. The scramble renderer uses it for the step trajectory."""

    num_steps_run: int | None = None
    """egg-stitch's search-work count for the first abstraction (best-first heap
    pops, cut short by --compression-limit, or SMC steps run). None for other
    tools, a timed-out run, or when no abstraction was found. Used by the
    ablation study as a hardware-independent complement to wall-clock."""

    egg_compression_ratio: float | None = None
    """egg-stitch's own reported ``compression_ratio`` for the first abstraction —
    the exact metric ``--compression-limit`` checks. None for other tools, a
    timed-out run, or when no abstraction was found. See
    :attr:`expts.bench.BenchResult.egg_compression_ratio` for why the ablation
    feeds this back rather than the harness's own ic/fc."""

    @classmethod
    def timed_out_result(cls, *, method: str, domain: str, file: str,
                         initial_cost: int, timeout: float) -> "PerFileResult":
        """Build a sentinel result for a run that exceeded its time budget."""
        return cls(
            method=method, domain=domain, file=file,
            initial_cost=initial_cost, final_cost=None, compression_ratio=None,
            elapsed_secs=timeout, library=[], egraph_min_term_size=None,
            timed_out=True,
        )

    def to_dict(self) -> dict:
        """Plain-dict representation for JSON serialization."""
        return asdict(self)


def egraph_min_from_bench(cost_after_rewrites: float) -> float | None:
    """Convert a runner's NaN-as-missing sentinel into None for JSON output."""
    return None if isnan(cost_after_rewrites) else cost_after_rewrites

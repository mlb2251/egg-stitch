"""Tool-agnostic types for the per-file compression bench.

Each tool wrapper lives under :mod:`expts.run_models` and returns a
:class:`BenchResult`; the runner aggregates those into per-(method, domain)
:class:`~expts.result.Result` records. Tool-specific constants
(SMC/BF settings, babble beam size, …) live alongside their wrappers; only
the truly cross-tool dial — :data:`MAX_ARITY` — is here.
"""

import math
from dataclasses import dataclass, field
from typing import Literal


Weighting = Literal["no-apps", "apps-equal"]
"""Selects the corpus shape and the per-tool flags that score it consistently:

- ``"no-apps"`` — flat cogsci-style s-expressions (operators take all children
  directly, no curried application nodes). Maps to egg-stitch
  ``--language op-children`` and to babble's ``drawings`` binary.
- ``"apps-equal"`` — curried dreamcoder-style s-expressions where every
  application is a binary ``App`` node. Maps to egg-stitch
  ``--language lambda-calc`` and to babble's ``benchmark`` binary.
"""


@dataclass
class Abstraction:
    """A single learned abstraction, in the tool's native s-expression form."""

    name: str
    body: str


@dataclass
class BenchResult:
    """Per-file output of one tool invocation, in a tool-agnostic shape.

    ``initial_corpus`` and ``final_corpus`` are the program strings before and
    after the tool's rewrite/compression step. The runner recomputes costs from
    these uniformly via :func:`expts.runner.ast_size`, so the four tools
    contribute apples-to-apples numbers regardless of their internal cost
    metrics.
    """

    elapsed_secs: float
    initial_corpus: list[str]
    final_corpus: list[str]
    abstractions: list[Abstraction]
    # The minimum AST size reachable in the e-graph after DSR rewrites are
    # applied, before any abstraction is found. Only egg-stitch (with DSRs)
    # reports a real number; everything else leaves the default NaN, which
    # propagates through ``sum(...)`` so a mixed batch automatically yields
    # NaN at the aggregate level.
    cost_after_rewrites: float = field(default=math.nan)
    # egg-stitch's per-iteration best cost (in its native cost units, with the
    # prior library bodies folded in), one entry per learned abstraction. Only
    # egg-stitch fills this; other tools leave it None. Used by the scramble
    # renderer to draw the step-by-step compression trajectory.
    cost_at_end_of_each_iter: list[int] | None = field(default=None)


# Maximum arity of learned abstractions — set the same across all tools so the
# table comparison stays apples-to-apples. Per-tool variations belong on the
# tool's own module, not here.
MAX_ARITY = 2

# Per-tool address-space cap (RLIMIT_AS), applied uniformly so a run's memory
# behaviour is reproducible across machines rather than depending on whatever
# RAM happens to be free. A tool that exceeds it is killed and recorded as a
# DNF, the same as a timeout. 20 GiB.
MEM_LIMIT_BYTES = 20 * 1024**3

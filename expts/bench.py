"""Tool-agnostic types for the per-file compression bench.

Each tool wrapper lives under :mod:`expts.run_models` and returns a
:class:`BenchResult`; the runner aggregates those into per-(method, domain)
:class:`~expts.result.Result` records. Tool-specific constants
(SMC/BF settings, babble beam size, …) live alongside their wrappers; only
the truly cross-tool dial — :data:`MAX_ARITY` — is here.
"""

from dataclasses import dataclass
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
    # Optional: the minimum AST size reachable in the e-graph after DSR
    # rewrites are applied, before any abstraction is found. Only egg-stitch
    # reports this (it falls out of the e-graph extraction it does anyway);
    # other tools leave it ``None``.
    cost_after_rewrites: int | None = None


# Maximum arity of learned abstractions — set the same across all tools so the
# table comparison stays apples-to-apples. Per-tool variations belong on the
# tool's own module, not here.
MAX_ARITY = 2

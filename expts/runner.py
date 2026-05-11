"""Domain dispatch + multi-file aggregation for the four bench wrappers.

Sits between :mod:`expts.bench` (per-file subprocess wrappers, returning
:class:`~expts.bench.BenchResult`) and the table runners
(:mod:`expts.table1`/:mod:`expts.table2` etc.). Owns:

- the domain → input files + rewrites mapping (``input_files``,
  ``rewrites_path``, ``weighting_for``);
- the per-file loop over a single tool, with cost recomputation via a uniform
  :func:`ast_size` so all four tools' numbers are comparable;
- aggregation across files into a single :class:`~expts.result.Result`
  (sums for cost/time, geomean of per-file ratios — matching the babble paper).
"""

from __future__ import annotations

import math
from pathlib import Path
from typing import Protocol, runtime_checkable

from s_expression_parser import parse, ParserConfig, Pair, nil

from . import COGSCI_DOMAINS, DREAMCODER_DOMAINS
from .bench import Abstraction, BenchResult, Weighting
from .result import Result
from .run_models import babble as _babble
from .run_models import ours as _ours

# The ours and babble model files own their respective project roots; pull
# them in here so domain-path resolution lives in a single place.
EGG_STITCH_DIR = _ours.EGG_STITCH_DIR
BABBLE_DIR = _babble.BABBLE_DIR


@runtime_checkable
class Runner(Protocol):
    """The shape :func:`run_method` expects from any tool runner.

    Implemented by the dataclasses in :mod:`expts.run_models`. Concrete
    runners carry their hyperparameters as fields and expose ``method`` /
    ``is_ours`` class constants for downstream bookkeeping.
    """

    method: str
    is_ours: bool

    def __call__(self, rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult: ...


# ─── domain helpers ────────────────────────────────────────────────────────


def domain_type(domain: str) -> str:
    """Return ``"cogsci"`` or ``"dreamcoder"`` for a known domain."""
    if domain in DREAMCODER_DOMAINS:
        return "dreamcoder"
    if domain in COGSCI_DOMAINS:
        return "cogsci"
    raise ValueError(f"Unknown domain '{domain}'")


def weighting_for(domain: str) -> Weighting:
    """``"no-apps"`` for cogsci (flat s-exprs), ``"apps-equal"`` for dreamcoder
    (curried lambda-calc)."""
    return "no-apps" if domain_type(domain) == "cogsci" else "apps-equal"


def input_files(domain: str) -> list[Path]:
    """Absolute paths of the corpus files for a domain.

    Cogsci domains have a single file; dreamcoder domains have one file per
    benchmark iteration. Order is sorted so re-runs are deterministic.
    """
    if domain_type(domain) == "cogsci":
        return [EGG_STITCH_DIR / "data" / "domains" / "cogsci" / f"{domain}.json"]
    d = EGG_STITCH_DIR / "data" / "domains" / domain
    return sorted(p for p in d.iterdir() if p.is_file() and p.suffix == ".json")


def rewrites_path(domain: str) -> str | None:
    """Path (relative to egg-stitch's cwd) to the babble rewrite file for
    ``domain``, or ``None`` when no DSRs ship for it.

    Cogsci files live under ``drawings.<domain>.rewrites``; dreamcoder ones at
    ``<domain>.rewrites``. ``text``/``logo``/``towers`` have no DSRs.
    """
    dt = domain_type(domain)
    if dt == "dreamcoder":
        path = BABBLE_DIR / "harness" / "data" / "benchmark-dsrs" / f"{domain}.rewrites"
        return f"../babble/harness/data/benchmark-dsrs/{domain}.rewrites" if path.exists() else None
    return f"../babble/harness/data/benchmark-dsrs/drawings.{domain}.rewrites"


# ─── uniform cost ──────────────────────────────────────────────────────────


_PARSER_CONFIG = ParserConfig(prefix_symbols={}, dots_are_cons=False)


def _node_cost(node, weighting: Weighting) -> int:
    """Recursive cost of a parsed s-expression node.

    Atoms count as 1. For a list ``(head c1 ... cn)`` the cost is the sum of
    child costs plus, under ``apps-equal``, one extra App per child position
    — except for ``lam`` which is a primitive Lam node (no surrounding Apps)
    in egg-stitch's lambda-calc grammar.
    """
    if isinstance(node, str):
        return 1
    children: list = []
    while node is not nil:
        children.append(node.car)
        node = node.cdr
    if not children:
        return 1
    head, *rest = children
    body = _node_cost(head, weighting) + sum(_node_cost(c, weighting) for c in rest)
    if weighting == "apps-equal" and head != "lam":
        body += len(rest)  # one App node per child position (curried application)
    return body


def ast_size(programs: list[str], weighting: Weighting) -> int:
    """Total cost of a corpus under the given weighting.

    Walks each parsed program; ``no-apps`` counts every atom; ``apps-equal``
    additionally charges one App node per application child (matching
    egg-stitch's ``Weights{1,1,1}`` on lambda-calc with a special case for the
    ``lam`` binder, which is itself a node with no implicit App).
    """
    total = 0
    for prog in programs:
        [tree] = parse(prog, _PARSER_CONFIG)
        total += _node_cost(tree, weighting)
    return total


def _bench_cost(b: BenchResult, weighting: Weighting) -> tuple[int, int]:
    """``(initial_cost, final_cost)`` recomputed uniformly from ``b``'s corpora.

    ``final_cost`` includes the abstractions' bodies — they're part of the
    library the rewritten corpus references, so omitting them would
    artificially favour tools that learn larger abstractions.
    """
    initial = ast_size(b.initial_corpus, weighting)
    final = ast_size(b.final_corpus, weighting) + ast_size([a.body for a in b.abstractions], weighting)
    return initial, final


# ─── runner ────────────────────────────────────────────────────────────────


def _ratio(initial: int, final: int) -> float:
    """Compression ratio with ``inf`` for the degenerate ``final == 0`` case."""
    return float("inf") if final == 0 else initial / final


def _aggregate(method: str, domain: str, per_file: list[tuple[BenchResult, int, int]]) -> Result:
    """Combine per-file results into a single :class:`Result`.

    Costs and time sum across files; ``compression_ratio`` is the geometric
    mean of the per-file ratios (Fig. 12 in the babble paper). The library is
    the concatenation of per-file abstractions, formatted as
    ``"<name>: <body>"`` to match the existing JSON consumers.
    """
    assert per_file, "need at least one per-file result"
    initial = sum(ic for _, ic, _ in per_file)
    final = sum(fc for _, _, fc in per_file)
    elapsed = sum(b.elapsed_secs for b, _, _ in per_file)
    ratios = [_ratio(ic, fc) for _, ic, fc in per_file]
    for r in ratios:
        assert 0 < r < math.inf, (
            f"per-file compression_ratio={r} on {domain} would make the geomean degenerate"
        )
    geo_cr = math.exp(sum(math.log(r) for r in ratios) / len(ratios))
    library: list[str] = []
    for b, _, _ in per_file:
        library.extend(f"{a.name}: {a.body}" for a in b.abstractions)
    return Result(
        method=method,
        domain=domain,
        initial_cost=initial,
        final_cost=final,
        compression_ratio=geo_cr,
        elapsed_secs=elapsed,
        library=library,
    )


def run_method(
    runner: Runner,
    domain: str,
    *,
    rounds: int,
    use_dsrs: bool,
) -> tuple[Result, float]:
    """Run ``runner`` on every input file of ``domain`` and aggregate.

    The runner instance carries its own hyperparameters; pass overrides as
    kwargs at construction (e.g. ``OursBf(rebuild_egraph=True)``).

    Returns ``(Result, egraph_min_term_size)``. The second element is the
    sum of ``BenchResult.cost_after_rewrites`` across per-file invocations;
    NaN propagates automatically when any file didn't produce one (i.e.
    when the runner isn't ours, or DSRs weren't used).
    """
    weighting = weighting_for(domain)
    rew = rewrites_path(domain) if use_dsrs else None

    per_file: list[tuple[BenchResult, int, int]] = []
    for f in input_files(domain):
        b = runner(rounds, f, rew, weighting)
        ic, fc = _bench_cost(b, weighting)
        per_file.append((b, ic, fc))
    egraph_min_total = sum(b.cost_after_rewrites for b, _, _ in per_file)
    return _aggregate(runner.method, domain, per_file), egraph_min_total

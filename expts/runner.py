"""Domain dispatch for the four bench wrappers.

Sits between :mod:`expts.bench` (per-file subprocess wrappers, returning
:class:`~expts.bench.BenchResult`) and the table runners
(:mod:`expts.tables`). Owns:

- the domain → input files + rewrites mapping (``input_files``,
  ``rewrites_path``, ``weighting_for``);
- the per-file loop over a single tool, with cost recomputation via a uniform
  :func:`ast_size` so all four tools' numbers are comparable.

Aggregation across files is *not* done here: ``run_method`` returns the raw
``list[PerFileResult]`` and readers aggregate at display time.
"""

from __future__ import annotations

from pathlib import Path
from typing import Protocol, runtime_checkable

from s_expression_parser import parse, ParserConfig, Pair, nil

from . import COGSCI_DOMAINS, DREAMCODER_DOMAINS
from .bench import Abstraction, BenchResult, Weighting
from .result import PerFileResult, egraph_min_from_bench
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
    runners carry their hyperparameters as fields; ``str(runner)`` (the
    dataclass repr) is used as the method label for downstream bookkeeping.
    """

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


def run_method(
    runner: Runner,
    domain: str,
    *,
    rounds: int,
    use_dsrs: bool,
) -> list[PerFileResult]:
    """Run ``runner`` on every input file of ``domain`` and return the per-file
    results unaggregated.

    The runner instance carries its own hyperparameters; pass overrides as
    kwargs at construction (e.g. ``OursBf(num_steps=5000)``).

    Each :class:`PerFileResult` carries its own ``egraph_min_term_size``
    (None when the runner isn't ours, or DSRs weren't used). Callers that
    need a domain-level number aggregate across the list themselves.

    Caching is the caller's responsibility — table runners and bench scripts
    own their own cache files at coarser granularity.
    """
    weighting = weighting_for(domain)
    rew = rewrites_path(domain) if use_dsrs else None

    out: list[PerFileResult] = []
    for f in input_files(domain):
        b = runner(rounds, f, rew, weighting)
        ic, fc = _bench_cost(b, weighting)
        assert fc > 0, f"{domain}/{f.name}: final_cost=0 would make compression_ratio undefined"
        out.append(PerFileResult(
            method=str(runner),
            domain=domain,
            file=f.stem,
            initial_cost=ic,
            final_cost=fc,
            compression_ratio=ic / fc,
            elapsed_secs=b.elapsed_secs,
            library=[f"{a.name}: {a.body}" for a in b.abstractions],
            egraph_min_term_size=egraph_min_from_bench(b.cost_after_rewrites),
        ))
    return out

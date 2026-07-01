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

import json
import signal
import subprocess
from dataclasses import dataclass
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


@dataclass(frozen=True)
class FamilyDomain:
    """A ``<name>:<member>`` pseudo-domain family with shipped DSRs.

    Both molecule-scramble and EPFL-circuit domains are single-file op-children
    corpora addressed as ``<name>:<member>``; this holds the only bits that vary
    between them so the dispatch functions below stay family-agnostic. ``name``
    doubles as the :func:`domain_type` label and the prefix before ``:``;
    ``file_template`` is formatted with ``member=`` relative to
    :data:`EGG_STITCH_DIR`.
    """
    name: str
    members: tuple[str, ...]
    file_template: str
    rewrites: str

    def member(self, domain: str) -> str | None:
        """The ``<member>`` for one of this family's domains, else None."""
        prefix = f"{self.name}:"
        return domain[len(prefix):] if domain.startswith(prefix) else None

    @property
    def domains(self) -> list[str]:
        """Every ``<name>:<member>`` domain string in this family."""
        return [f"{self.name}:{m}" for m in self.members]

    def input_file(self, member: str) -> Path:
        """Absolute path of the corpus file for one ``member``."""
        return EGG_STITCH_DIR / self.file_template.format(member=member)


# Molecule scramble families live under data/domains/molecules/scramble/ (real
# PubChem substructure corpora; op-children grammar, symmetry DSRs; see
# data/.../scramble/README.md). EPFL benchmark circuits live under
# data/domains/epfl-circuits/ (k-feasible-cut gate cones, AND/OR DSRs; source
# .aig files and generators in scripts/epfl-circuits/).
MOLECULES = FamilyDomain(
    name="molecules",
    members=("hexyl", "ester", "glycol"),
    file_template="data/domains/molecules/scramble/{member}.scram.json",
    rewrites="data/domains/molecules/molecules.rewrites",
)
EPFL_CIRCUITS = FamilyDomain(
    name="epfl-circuits",
    members=("hyp", "log2", "multiplier", "square", "voter"),
    file_template="data/domains/epfl-circuits/{member}.json",
    rewrites="data/domains/epfl-circuits/and_or_demorgan_factor.rewrites",
)
FAMILY_DOMAINS = (MOLECULES, EPFL_CIRCUITS)


def family_for(domain: str) -> FamilyDomain | None:
    """The :class:`FamilyDomain` owning ``domain``, else None."""
    return next((f for f in FAMILY_DOMAINS if f.member(domain) is not None), None)


def domain_type(domain: str) -> str:
    """Return ``"cogsci"``, ``"dreamcoder"``, ``"molecules"``, or
    ``"epfl-circuits"`` for a known domain."""
    fam = family_for(domain)
    if fam is not None:
        return fam.name
    if domain in DREAMCODER_DOMAINS:
        return "dreamcoder"
    if domain in COGSCI_DOMAINS:
        return "cogsci"
    raise ValueError(f"Unknown domain '{domain}'")


def weighting_for(domain: str) -> Weighting:
    """``"no-apps"`` for cogsci/molecules (flat op-children s-exprs),
    ``"apps-equal"`` for dreamcoder (curried lambda-calc)."""
    return "apps-equal" if domain_type(domain) == "dreamcoder" else "no-apps"


def input_files(domain: str) -> list[Path]:
    """Absolute paths of the corpus files for a domain.

    Cogsci/molecule/circuit domains have a single file; dreamcoder domains have
    one file per benchmark iteration. Order is sorted so re-runs are deterministic.
    """
    fam = family_for(domain)
    if fam is not None:
        return [fam.input_file(fam.member(domain))]
    if domain_type(domain) == "cogsci":
        return [EGG_STITCH_DIR / "data" / "domains" / "cogsci" / f"{domain}.json"]
    d = EGG_STITCH_DIR / "data" / "domains" / domain
    return sorted(p for p in d.iterdir() if p.is_file() and p.suffix == ".json")


def rewrites_path(domain: str) -> str | None:
    """Path (relative to egg-stitch's cwd) to the rewrite file for ``domain``,
    or ``None`` when no DSRs ship for it.

    Cogsci files live under ``drawings.<domain>.rewrites``; dreamcoder ones at
    ``<domain>.rewrites``; molecules/circuits each ship one family-wide file.
    ``text``/``logo``/``towers`` have no DSRs.
    """
    fam = family_for(domain)
    if fam is not None:
        return fam.rewrites
    if domain_type(domain) == "dreamcoder":
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


# Signals whose kill we treat as "ran out of resources" → DNF (see run_method).
_RESOURCE_KILL_SIGNALS = {signal.SIGTERM, signal.SIGKILL, signal.SIGABRT}


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
        try:
            b = runner(rounds, f, rew, weighting)
        except (subprocess.TimeoutExpired, subprocess.CalledProcessError) as e:
            # Record a DNF sentinel rather than aborting the whole table.
            # Cases that all count as "didn't finish within budget":
            #   * TimeoutExpired — the tool blew its own wall-clock cap;
            #   * killed by a resource signal (-SIGTERM external OOM watchdog,
            #     -SIGKILL system/cgroup OOM-killer, -SIGABRT Rust allocation
            #     failure under the RLIMIT_AS cap) — i.e. ran out of the memory
            #     or time budget.
            # Any other non-zero exit (e.g. an unwinding panic, exit 101) is a
            # real failure, so re-raise it.
            if isinstance(e, subprocess.TimeoutExpired):
                budget = e.timeout
            elif e.returncode < 0 and -e.returncode in _RESOURCE_KILL_SIGNALS:
                budget = getattr(runner, "timeout", None) or float("nan")
            else:
                raise
            # initial_cost is recomputed from the (unmodified) input corpus so
            # the row still carries a size.
            with open(f) as fh:
                programs = json.load(fh)
            out.append(PerFileResult.timed_out_result(
                method=str(runner), domain=domain, file=f.stem,
                initial_cost=ast_size(programs, weighting), timeout=budget,
            ))
            continue
        ic, fc = _bench_cost(b, weighting)
        assert fc > 0, f"{domain}/{f.name}: final_cost=0 would make compression_ratio undefined"
        # Freeform firehose: the whole egg-stitch JSON minus the bulky program
        # arrays (already captured as the corpus). Empty dict -> None so non-ours
        # tools and older outputs stay null.
        stats = {k: v for k, v in b.raw.items()
                 if k not in ("original_programs", "rewritten_programs")} or None
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
            cost_at_end_of_each_iter=b.cost_at_end_of_each_iter,
            stats=stats,
        ))
    return out

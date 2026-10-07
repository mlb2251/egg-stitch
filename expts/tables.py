"""Table experiments. Each table is a single configuration of: which domains to
run, which runners participate, whether DSRs are enabled, and
``num_abstractions``.

The babble comparisons run Ours (Enum + SMC) vs babble (vs Stitch) on the cogsci
drawing domains plus the dreamcoder benchmarks:

    babble_comparison_rewrites_single:     with DSRs, 1 abstraction,  Enum/SMC/babble.
    babble_comparison_no_rewrites_single:  no DSRs,   1 abstraction,  Enum/SMC/babble/Stitch.
    babble_comparison_rewrites:            with DSRs, 20 abstractions.
    babble_comparison_no_rewrites:         no DSRs,   20 abstractions.
"""

from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Sequence

from tqdm import tqdm

from . import ALL_DOMAINS
from ._subproc import available_memory_bytes
from .bench import MAX_ARITY, MEM_LIMIT_BYTES
from .folders import SUMMARY_RESULTS_DIR, set_folder, summary_results_path
from .run_models import Babble, OursBf, OursSmc, Stitch
from .runner import EPFL_ALL_MEMBERS, EPFL_CIRCUITS, MOLECULES

# SMC is stochastic, so it needs more repeats to average out run-to-run noise;
# every other method here is deterministic and only needs a few for timing noise.
SMC_NUM_RUNS = 10
NUM_RUNS = 3


def _num_runs_for(label: str) -> int:
    """Repeats to run for a method: ``SMC_NUM_RUNS`` for the stochastic SMC
    sweep (``smc-<particles>``), ``NUM_RUNS`` for every deterministic method."""
    return SMC_NUM_RUNS if label.startswith("smc") else NUM_RUNS

# babble has no equational theory for text/logo/towers, so the "with DSRs"
# comparison excludes them.
REWRITES_DOMAINS = ["nuts-bolts", "dials", "wheels", "furniture", "list", "physics"]
# The no-DSR comparison includes the dreamcoder domains without rewrite files
# (text/logo/towers).
NO_REWRITES_DOMAINS = REWRITES_DOMAINS + ["text", "logo", "towers"]

# Hyperparameter sweeps for the two ours-search modes. Each value gets its
# own runner / cache file, labelled ``enum-<num_steps>`` /
# ``smc-<num_particles>`` so the renderer can group sweep points back into
# a single line per base method, at the one canonical point below.
BFS_STEP_SWEEP: tuple[int, ...] = (200, 500, 1000, 2000, 5000, 10000, 20000, 50000)
SMC_PARTICLE_SWEEP: tuple[int, ...] = (20, 50, 100, 200, 500, 1000, 2000, 5000)

# The single sweep point each base method contributes to the table cells.
# Plots use the full sweep regardless.
TABLE_BFS_STEPS = 10000
TABLE_SMC_PARTICLES = 1000


def _sweep_runners(
    timeout: float | None = None,
    bfs_steps: tuple[int, ...] = BFS_STEP_SWEEP,
    smc_particles: tuple[int, ...] = SMC_PARTICLE_SWEEP,
    mem_limit: int | None = None,
    max_arity: int = MAX_ARITY,
    iter_limit: int | None = None,
    language: str | None = None,
    extra_args: tuple[str, ...] = (),
) -> tuple[tuple[str, object], ...]:
    """``(label, runner)`` pairs for every BFS-step and SMC-particle sweep value.

    ``timeout`` (seconds) caps each tool invocation's wall-clock and
    ``mem_limit`` (bytes) its address space; None means no cap. ``bfs_steps`` /
    ``smc_particles`` override the sweeps (the molecules table extends them, the circuits table truncates).
    ``max_arity`` raises the abstraction arity cap (the circuits table uses 4). ``iter_limit``
    caps e-saturation iterations (the circuits table uses 30; None keeps the binary default).
    ``language`` overrides the weighting-derived language (the circuits table uses
    ``op-children-db``). ``extra_args`` are appended verbatim to every swept
    runner's CLI (the standalone drawings-algebraic experiment passes its
    match-set caps here).
    """
    common = dict(max_arity=max_arity, iter_limit=iter_limit, language=language,
                  timeout=timeout, mem_limit=mem_limit, extra_args=extra_args)
    bfs = tuple((f"enum-{n}", OursBf(num_steps=n, **common)) for n in bfs_steps)
    smc = tuple((f"smc-{p}", OursSmc(num_particles=p, **common)) for p in smc_particles)
    return bfs + smc


# Best-first operating point for the single dsrs-only-at-start baseline row
# (the "BFS@start" column on the DSR tables and molecules). It collapses to a
# small rule-free e-graph, so we just let best-first run essentially to
# exhaustion — the step budget is set far above what any input needs.
BASELINE_BFS_STEPS = 10_000_000

# Runner rosters — the no-DSR comparisons share the with-Stitch roster; the DSR
# comparisons (no Stitch) add two best-first baselines: "dsrs-only-at-start" (BFS/MT), which
# canonicalises with the DSRs once instead of keeping them live, and
# "enum-baseline" (BFS/NR), which turns the DSRs off entirely.
BASE_RUNNERS: tuple[tuple[str, object], ...] = _sweep_runners() + (
    ("babble", Babble()),
)
RUNNERS_WITH_STITCH: tuple[tuple[str, object], ...] = BASE_RUNNERS + (
    ("stitch", Stitch()),
)
DSR_RUNNERS: tuple[tuple[str, object], ...] = BASE_RUNNERS + (
    ("enum-dsrs-at-start", OursBf(num_steps=BASELINE_BFS_STEPS, only_use_dsrs_at_start=True)),
    ("enum-baseline", OursBf(num_steps=BASELINE_BFS_STEPS, no_dsrs=True)),
)


def _run_method_for_table(
    label: str,
    runner: object,
    *,
    domains: Sequence[str],
    num_abstractions: int,
    use_dsrs: bool,
    cache_path: Path,
    bar: tqdm,
) -> dict[str, list[list[dict]]]:
    """Run one method across all domains × its repeat count for a single table.

    The repeat count is ``SMC_NUM_RUNS`` for SMC and ``NUM_RUNS`` otherwise (see
    ``_num_runs_for``). Returns ``{domain: [run0_per_file_dicts, ...]}``. Cached
    as a single JSON file per (table, method); delete the file to force a
    recompute.
    """
    from .runner import run_method  # local import: runner pulls heavy deps

    num_runs = _num_runs_for(label)
    if cache_path.exists():
        with open(cache_path) as fh:
            out = json.load(fh)
        bar.update(len(domains) * num_runs)
        return out

    out: dict[str, list[list[dict]]] = {}
    for domain in domains:
        runs: list[list[dict]] = []
        for i in range(num_runs):
            bar.set_description(f"{domain} {label} rep {i+1}/{num_runs}")
            per_file = run_method(
                runner,
                domain,
                rounds=num_abstractions,
                use_dsrs=use_dsrs,
            )
            run = [r.to_dict() for r in per_file]
            runs.append(run)
            bar.update()
            # A single timed-out/OOM'd replicate makes the method a DNF for this
            # domain (the renderer drops a method with any DNF), so the remaining
            # replicates can only repeat an expensive timeout — skip them.
            if any(r["compression_ratio"] is None for r in run):
                bar.update(num_runs - (i + 1))
                break
        out[domain] = runs

    cache_path.parent.mkdir(parents=True, exist_ok=True)
    with open(cache_path, "w") as fh:
        json.dump(out, fh, indent=2)
    return out


def _run_table(
    *,
    domains: Sequence[str],
    runners: Sequence[tuple[str, object]],
    num_abstractions: int,
    use_dsrs: bool,
    folder_prefix: str,
    output_name: str,
) -> Path:
    """Run each ``(label, runner)`` on every domain (SMC ``SMC_NUM_RUNS`` times,
    others ``NUM_RUNS``; see ``_num_runs_for``) and save JSON."""
    assert all(
        d in ALL_DOMAINS or d.startswith("molecules:") or d.startswith("epfl-circuits:") or d.startswith("drawings:")
        for d in domains
    ), "domain typo"
    set_folder(f"{folder_prefix}/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    results: dict = {
        "config": {"num_abstractions": num_abstractions},
        "domains": {domain: {"runs": {}} for domain in domains},
    }
    cache_root = SUMMARY_RESULTS_DIR / Path(output_name).stem

    total = len(domains) * sum(_num_runs_for(label) for label, _ in runners)
    with tqdm(total=total, unit="run", smoothing=0.05) as bar:
        for label, runner in runners:
            by_domain = _run_method_for_table(
                label,
                runner,
                domains=domains,
                num_abstractions=num_abstractions,
                use_dsrs=use_dsrs,
                cache_path=cache_root / f"{label}.json",
                bar=bar,
            )
            for domain, runs in by_domain.items():
                results["domains"][domain]["runs"][label] = runs

    out_path = summary_results_path(output_name)
    with open(out_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nwrote {out_path}", flush=True)
    return out_path


def babble_comparison_rewrites_single() -> Path:
    """Run Enum, SMC, babble, and the dsrs-only-at-start (BFS/MT) and no-rules
    (BFS/NR) baselines on :data:`REWRITES_DOMAINS` with DSRs."""
    return _run_table(
        domains=REWRITES_DOMAINS,
        runners=DSR_RUNNERS,
        num_abstractions=1,
        use_dsrs=True,
        folder_prefix="babble_comparison_rewrites_single",
        output_name="babble_comparison_rewrites_single.json",
    )


def babble_comparison_no_rewrites_single() -> Path:
    """Run Enum, SMC, babble, and Stitch on :data:`NO_REWRITES_DOMAINS` with no DSRs."""
    return _run_table(
        domains=NO_REWRITES_DOMAINS,
        runners=RUNNERS_WITH_STITCH,
        num_abstractions=1,
        use_dsrs=False,
        folder_prefix="babble_comparison_no_rewrites_single",
        output_name="babble_comparison_no_rewrites_single.json",
    )


def babble_comparison_rewrites() -> Path:
    """Run the :func:`babble_comparison_rewrites_single` setup with 20 stacked abstractions."""
    return _run_table(
        domains=REWRITES_DOMAINS,
        runners=DSR_RUNNERS,
        num_abstractions=20,
        use_dsrs=True,
        folder_prefix="babble_comparison_rewrites",
        output_name="babble_comparison_rewrites.json",
    )


def babble_comparison_no_rewrites() -> Path:
    """Run the :func:`babble_comparison_no_rewrites_single` setup with 20 stacked abstractions."""
    return _run_table(
        domains=NO_REWRITES_DOMAINS,
        runners=RUNNERS_WITH_STITCH,
        num_abstractions=20,
        use_dsrs=False,
        folder_prefix="babble_comparison_no_rewrites",
        output_name="babble_comparison_no_rewrites.json",
    )


# The molecule scramble subset, with DSRs. Same algorithm roster as
# babble_comparison_rewrites (Enum/SMC sweeps + babble) plus a "dsrs-only-at-start" baseline
# (best-first that canonicalises with the DSRs once instead of keeping them
# live). Every algorithm gets a hard wall-clock cap.
MOLECULES_DOMAINS = MOLECULES.domains
MOLECULES_TIMEOUT = 300.0  # seconds, per tool invocation
MOLECULES_NUM_ABSTRACTIONS = 4
# Live DSRs inflate the e-graph with every symmetry-equivalent orientation, so
# best-first needs far more pops to converge on molecules than the 10k cogsci
# point. The sweep is extended to 100k, which is the representative enum
# operating point for this domain.
MOLECULES_BFS_SWEEP = BFS_STEP_SWEEP + (100_000,)
MOLECULES_ENUM_POINT = 100_000
# Representative SMC operating point for this domain (matches the canonical
# ``TABLE_SMC_PARTICLES`` point used by the tables renderer).
MOLECULES_SMC_POINT = 1_000


def _molecules_runners() -> tuple[tuple[str, object], ...]:
    """The babble_comparison_rewrites roster (Enum/SMC sweeps + babble) plus the dsrs-only-at-start
    and no-rules Enum (BFS/NR) baselines and Stitch, every runner capped at
    :data:`MOLECULES_TIMEOUT` and :data:`MEM_LIMIT_BYTES`.

    Stitch can't take the DSRs, so it runs on the raw corpus — the same problem
    BFS/NR solves, and the check that BFS/NR isn't a handicapped baseline."""
    capped = dict(timeout=MOLECULES_TIMEOUT, mem_limit=MEM_LIMIT_BYTES)
    return (
        _sweep_runners(timeout=MOLECULES_TIMEOUT, bfs_steps=MOLECULES_BFS_SWEEP, mem_limit=MEM_LIMIT_BYTES)
        + (("babble", Babble(**capped)),)
        + (("enum-dsrs-at-start", OursBf(
            num_steps=BASELINE_BFS_STEPS, only_use_dsrs_at_start=True, **capped)),)
        + (("enum-baseline", OursBf(
            num_steps=BASELINE_BFS_STEPS, no_dsrs=True, **capped)),)
        + (("stitch", Stitch(ignore_dsrs=True, **capped)),)
    )


def _require_free_memory(name: str) -> None:
    """Refuse to start table run `name` unless the machine has MEM_LIMIT_BYTES
    free: the per-tool memory cap is only a consistent control if that much is
    actually available."""
    free = available_memory_bytes()
    if free < MEM_LIMIT_BYTES:
        raise SystemExit(
            f"{name}: need >= {MEM_LIMIT_BYTES / 2**30:.0f} GiB free to apply a "
            f"consistent per-tool memory cap, but only {free / 2**30:.1f} GiB is "
            f"available. Free up memory or lower MEM_LIMIT_BYTES."
        )


def molecules() -> Path:
    """Run the molecule scramble subset with DSRs, the babble_comparison_rewrites roster + the
    dsrs-only-at-start baseline, each algorithm capped at 300s and 20 GiB."""
    _require_free_memory("molecules")
    return _run_table(
        domains=MOLECULES_DOMAINS,
        runners=_molecules_runners(),
        num_abstractions=MOLECULES_NUM_ABSTRACTIONS,
        use_dsrs=True,
        folder_prefix="molecules",
        output_name="molecules.json",
    )


# The EPFL circuits with the factoring DSRs. The molecules table's full roster
# (Enum/SMC sweeps + dsrs-only-at-start + babble + no-rules Enum baseline +
# Stitch), at max-arity 4. babble runs via its ``circuits`` binary (boolean
# and/or/not over ``$N`` inputs).
CIRCUITS_DOMAINS = EPFL_CIRCUITS.domains
CIRCUITS_TIMEOUT = 300.0  # seconds, per tool invocation
CIRCUITS_NUM_ABSTRACTIONS = 4
CIRCUITS_MAX_ARITY = 4
# Cone A's ``$3`` and cone B's ``$3`` are different nets, so an abstraction must
# not hardcode one. ``op-children-db`` parses ``$n`` as a free De Bruijn
# variable, which keeps it out of abstraction bodies; it is also the language
# ``scripts/epfl-circuits/build_benchmarks.py`` selects the corpus under.
CIRCUITS_LANGUAGE = "op-children-db"
# stitch's utility calculation disagrees with its own rewriter on these cones
# (it overcounts a self-overlapping `and`/`not` abstraction), which aborts the
# run; `--no-mismatch-check` lets it finish. The compression we report is
# recomputed from the rewritten corpus by `ast_size`, so the number stays
# honest -- but stitch selects its abstractions on the bad utility, so it is
# not the optimal-per-round search it is on every other corpus.
CIRCUITS_STITCH_NO_MISMATCH_CHECK = True
# Cap e-saturation at 30 iterations (vs the binary default 100). The factoring
# DSRs blow the e-graph up on these cones, and 100 iterations runs ~4-5x slower
# for no better result -- past the timeout at the high sweep points.
CIRCUITS_ITER_LIMIT = 30
# Enum needs ~2300 expansions of leaf-enumeration warmup before it forms any
# abstraction on these wide corpora, so below that it finishes with an empty
# library (a misleading 1.0); above it, the rule-saturated e-graph never
# converges and it times out. Sweep up to 10k so the representative point is
# past the warmup and lands on a real result (here: DNF). SMC caps at 2000.
# Both representative table/marker points are the renderer's canonical ones
# (TABLE_BFS_STEPS=10000, TABLE_SMC_PARTICLES=1000) -- the circuits table needs no custom
# point, unlike the molecules table's extended enum sweep (MOLECULES_ENUM_POINT=100k).
CIRCUITS_BFS_SWEEP = tuple(n for n in BFS_STEP_SWEEP if n <= 10000)
CIRCUITS_SMC_SWEEP = tuple(p for p in SMC_PARTICLE_SWEEP if p <= 2000)


def _circuits_runners() -> tuple[tuple[str, object], ...]:
    """Enum/SMC sweeps (live DSRs), the dsrs-only-at-start and no-rules Enum
    baselines, babble, and Stitch, every runner at max-arity 4 and capped at
    :data:`CIRCUITS_TIMEOUT` / :data:`MEM_LIMIT_BYTES`.

    ``iter_limit`` and ``language`` are ours-only knobs (egg-stitch's
    e-saturation cap and :data:`CIRCUITS_LANGUAGE`); babble runs its DSR
    saturation for a fixed 3 iterations internally, so it only takes the arity
    / resource caps."""
    common = dict(max_arity=CIRCUITS_MAX_ARITY, iter_limit=CIRCUITS_ITER_LIMIT,
                  language=CIRCUITS_LANGUAGE, timeout=CIRCUITS_TIMEOUT, mem_limit=MEM_LIMIT_BYTES)
    return (
        _sweep_runners(bfs_steps=CIRCUITS_BFS_SWEEP, smc_particles=CIRCUITS_SMC_SWEEP, **common)
        + (("enum-dsrs-at-start", OursBf(num_steps=BASELINE_BFS_STEPS, only_use_dsrs_at_start=True, **common)),)
        + (("babble", Babble(max_arity=CIRCUITS_MAX_ARITY, timeout=CIRCUITS_TIMEOUT, mem_limit=MEM_LIMIT_BYTES)),)
        + (("enum-baseline", OursBf(num_steps=BASELINE_BFS_STEPS, no_dsrs=True, **common)),)
        + (("stitch", Stitch(
            max_arity=CIRCUITS_MAX_ARITY, timeout=CIRCUITS_TIMEOUT, mem_limit=MEM_LIMIT_BYTES,
            ignore_dsrs=True, no_mismatch_check=CIRCUITS_STITCH_NO_MISMATCH_CHECK)),)
    )


def circuits() -> Path:
    """Run the EPFL circuits with the factoring DSRs: the molecules table's full roster
    (Enum/SMC + babble + the dsrs-only-at-start and no-rules Enum baselines),
    max-arity 4, 4 abstractions, each capped at 300s and 20 GiB."""
    _require_free_memory("circuits")
    return _run_table(
        domains=CIRCUITS_DOMAINS,
        runners=_circuits_runners(),
        num_abstractions=CIRCUITS_NUM_ABSTRACTIONS,
        use_dsrs=True,
        folder_prefix="circuits",
        output_name="circuits.json",
    )


# The circuits table's configuration over the *whole* EPFL suite rather than the
# five circuits build_benchmarks.py selects. Selection scores each circuit on
# structural diversity and on no-DSR compression, so the reported set is by
# construction the half where repeated structure exists for the DSRs to exploit
# — this table is what quantifies that. ~4x the circuits table's domains at the same roster,
# so budget hours; every (method, domain) cell is cached under results/circuits_all/
# and a re-run resumes.
CIRCUITS_ALL_DOMAINS = [f"{EPFL_CIRCUITS.name}:{m}" for m in EPFL_ALL_MEMBERS]


def circuits_all() -> Path:
    """Run the circuits table's roster and knobs across all 20 EPFL circuits."""
    _require_free_memory("circuits_all")
    return _run_table(
        domains=CIRCUITS_ALL_DOMAINS,
        runners=_circuits_runners(),
        num_abstractions=CIRCUITS_NUM_ABSTRACTIONS,
        use_dsrs=True,
        folder_prefix="circuits_all",
        output_name="circuits_all.json",
    )

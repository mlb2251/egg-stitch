"""Table 1-4 experiments: Ours (Enum + SMC) vs babble (vs Stitch) on the
cogsci drawing domains plus the dreamcoder benchmarks.

Each table is a single configuration of: which domains to run, which
runners participate, whether DSRs are enabled, and ``num_abstractions``.

    Table 1: with DSRs, 1 abstraction,  Enum/SMC/babble.
    Table 2: no DSRs,   1 abstraction,  Enum/SMC/babble/Stitch.
    Table 3: with DSRs, 20 abstractions (same shape as Table 1).
    Table 4: no DSRs,   20 abstractions (same shape as Table 2).
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
from .runner import EPFL_CIRCUITS, MOLECULES

NUM_RUNS = 10

# Table 1 / 3: babble has no equational theory for text/logo/towers, so the
# "with DSRs" comparison excludes them.
TABLE1_DOMAINS = ["nuts-bolts", "dials", "wheels", "furniture", "list", "physics"]
# Table 2 / 4: no-DSR comparison, includes the dreamcoder domains without
# rewrite files (text/logo/towers).
TABLE2_DOMAINS = TABLE1_DOMAINS + ["text", "logo", "towers"]

# Hyperparameter sweeps for the two ours-search modes. Each value gets its
# own runner / cache file, labelled ``enum-<num_steps>`` /
# ``smc-<num_particles>`` so the renderer can group sweep points back into
# a single line per base method. The table cells still show one canonical
# point per method (see ``TABLE_BFS_STEPS`` / ``TABLE_SMC_PARTICLES`` in
# ``scripts/render_tables.py``).
BFS_STEP_SWEEP: tuple[int, ...] = (200, 500, 1000, 2000, 5000, 10000, 20000, 50000)
SMC_PARTICLE_SWEEP: tuple[int, ...] = (20, 50, 100, 200, 500, 1000, 2000, 5000)


def _sweep_runners(
    timeout: float | None = None,
    bfs_steps: tuple[int, ...] = BFS_STEP_SWEEP,
    smc_particles: tuple[int, ...] = SMC_PARTICLE_SWEEP,
    mem_limit: int | None = None,
    max_arity: int = MAX_ARITY,
    iter_limit: int | None = None,
    max_match_set: int | None = None,
    decompose_min_rows: int | None = None,
) -> tuple[tuple[str, object], ...]:
    """``(label, runner)`` pairs for every BFS-step and SMC-particle sweep value.

    ``timeout`` (seconds) caps each tool invocation's wall-clock and
    ``mem_limit`` (bytes) its address space; None means no cap. ``bfs_steps`` /
    ``smc_particles`` override the sweeps (table5 extends them, table7 truncates).
    ``max_arity`` raises the abstraction arity cap (table7 uses 4). ``iter_limit``
    caps e-saturation iterations (table7 uses 30; None keeps the binary default).
    ``max_match_set`` / ``decompose_min_rows`` cap the per-factor match-set on
    every swept runner (table6 sets them for the non-confluent algebra DSRs;
    None elsewhere).
    """
    common = dict(max_arity=max_arity, iter_limit=iter_limit, timeout=timeout, mem_limit=mem_limit, max_match_set=max_match_set, decompose_min_rows=decompose_min_rows)
    bfs = tuple((f"enum-{n}", OursBf(num_steps=n, **common)) for n in bfs_steps)
    smc = tuple((f"smc-{p}", OursSmc(num_particles=p, **common)) for p in smc_particles)
    return bfs + smc


# Best-first operating point for the single dsrs-only-at-start baseline row
# (the "BFS@start" column on the DSR tables and table5). It collapses to a
# small rule-free e-graph, so we just let best-first run essentially to
# exhaustion — the step budget is set far above what any input needs.
BASELINE_BFS_STEPS = 10_000_000

# Runner rosters — Table 2/4 share the with-Stitch roster; Table 1/3 (DSRs,
# no Stitch) add a "dsrs-only-at-start" baseline: best-first that canonicalises
# with the DSRs once instead of keeping them live (the BFS@start column).
BASE_RUNNERS: tuple[tuple[str, object], ...] = _sweep_runners() + (
    ("babble", Babble()),
)
RUNNERS_WITH_STITCH: tuple[tuple[str, object], ...] = BASE_RUNNERS + (
    ("stitch", Stitch()),
)
DSR_RUNNERS: tuple[tuple[str, object], ...] = BASE_RUNNERS + (
    ("enum-dsrs-at-start", OursBf(num_steps=BASELINE_BFS_STEPS, only_use_dsrs_at_start=True)),
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
    """Run one method across all domains × ``NUM_RUNS`` for a single table.

    Returns ``{domain: [run0_per_file_dicts, run1_per_file_dicts, ...]}``.
    Cached as a single JSON file per (table, method); delete the file to
    force a recompute.
    """
    from .runner import run_method  # local import: runner pulls heavy deps

    if cache_path.exists():
        with open(cache_path) as fh:
            out = json.load(fh)
        bar.update(len(domains) * NUM_RUNS)
        return out

    out: dict[str, list[list[dict]]] = {}
    for domain in domains:
        runs: list[list[dict]] = []
        for i in range(NUM_RUNS):
            bar.set_description(f"{domain} {label} rep {i+1}/{NUM_RUNS}")
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
                bar.update(NUM_RUNS - (i + 1))
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
    """Run each ``(label, runner)`` on every domain ``NUM_RUNS`` times and save JSON."""
    assert all(
        d in ALL_DOMAINS or d.startswith(("molecules:", "epfl-circuits:", "drawings:"))
        for d in domains
    ), "domain typo"
    set_folder(f"{folder_prefix}/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    results: dict = {
        "config": {"num_abstractions": num_abstractions},
        "domains": {domain: {"runs": {}} for domain in domains},
    }
    cache_root = SUMMARY_RESULTS_DIR / Path(output_name).stem

    total = len(domains) * NUM_RUNS * len(runners)
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


def table1() -> Path:
    """Run Enum, SMC, babble, and the dsrs-only-at-start baseline on the
    Table 1 domains with DSRs."""
    return _run_table(
        domains=TABLE1_DOMAINS,
        runners=DSR_RUNNERS,
        num_abstractions=1,
        use_dsrs=True,
        folder_prefix="table1",
        output_name="table1.json",
    )


def table2() -> Path:
    """Run Enum, SMC, babble, and Stitch on the Table 2 domains with no DSRs."""
    return _run_table(
        domains=TABLE2_DOMAINS,
        runners=RUNNERS_WITH_STITCH,
        num_abstractions=1,
        use_dsrs=False,
        folder_prefix="table2",
        output_name="table2.json",
    )


def table3() -> Path:
    """Run the Table 1 setup with 20 stacked abstractions."""
    return _run_table(
        domains=TABLE1_DOMAINS,
        runners=DSR_RUNNERS,
        num_abstractions=20,
        use_dsrs=True,
        folder_prefix="table3",
        output_name="table3.json",
    )


def table4() -> Path:
    """Run the Table 2 setup with 20 stacked abstractions."""
    return _run_table(
        domains=TABLE2_DOMAINS,
        runners=RUNNERS_WITH_STITCH,
        num_abstractions=20,
        use_dsrs=False,
        folder_prefix="table4",
        output_name="table4.json",
    )


# Table 5: the molecule scramble subset, with DSRs. Same algorithm roster as
# Table 3 (Enum/SMC sweeps + babble) plus a "dsrs-only-at-start" baseline
# (best-first that canonicalises with the DSRs once instead of keeping them
# live). Every algorithm gets a hard wall-clock cap.
TABLE5_DOMAINS = MOLECULES.domains
TABLE5_TIMEOUT = 300.0  # seconds, per tool invocation
TABLE5_NUM_ABSTRACTIONS = 4
# Live DSRs inflate the e-graph with every symmetry-equivalent orientation, so
# best-first needs far more pops to converge on molecules than the 10k cogsci
# point. The sweep is extended to 100k, which is the representative enum
# operating point for this domain.
TABLE5_BFS_SWEEP = BFS_STEP_SWEEP + (100_000,)
TABLE5_ENUM_POINT = 100_000
# Representative SMC operating point for this domain (matches the canonical
# ``TABLE_SMC_PARTICLES`` point used by the tables renderer).
TABLE5_SMC_POINT = 1_000


def _table5_runners() -> tuple[tuple[str, object], ...]:
    """Table 3's roster (Enum/SMC sweeps + babble) plus the dsrs-only-at-start
    and no-rules Enum (BFS/NR) baselines, every runner capped at
    :data:`TABLE5_TIMEOUT` and :data:`MEM_LIMIT_BYTES`."""
    capped = dict(timeout=TABLE5_TIMEOUT, mem_limit=MEM_LIMIT_BYTES)
    return (
        _sweep_runners(timeout=TABLE5_TIMEOUT, bfs_steps=TABLE5_BFS_SWEEP, mem_limit=MEM_LIMIT_BYTES)
        + (("babble", Babble(**capped)),)
        + (("enum-dsrs-at-start", OursBf(
            num_steps=BASELINE_BFS_STEPS, only_use_dsrs_at_start=True, **capped)),)
        + (("enum-baseline", OursBf(
            num_steps=BASELINE_BFS_STEPS, no_dsrs=True, **capped)),)
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


def table5() -> Path:
    """Run the molecule scramble subset with DSRs, Table 3 roster + the
    dsrs-only-at-start baseline, each algorithm capped at 300s and 20 GiB."""
    _require_free_memory("table5")
    return _run_table(
        domains=TABLE5_DOMAINS,
        runners=_table5_runners(),
        num_abstractions=TABLE5_NUM_ABSTRACTIONS,
        use_dsrs=True,
        folder_prefix="table5",
        output_name="table5.json",
    )


# Table 6: the cogsci drawing domains with our algebraic drawing DSRs
# (data/domains/cogsci/drawings.rewrites), comparing the DSRs kept LIVE during
# search against applied only AT START. The rules are deliberately *non-confluent*
# (transform factoring, repeat<->unroll, overlay assoc/comm, scale/translate
# interchange) — they expose multiple equivalent normal forms whose best choice
# depends on the library being built. Live keeps all forms so each abstraction can
# align to the matching one; at-start commits to a single greedy min-term up front,
# so live wins (and the gap widens with expressiveness). Same roster shape as
# table5/7: the Enum/SMC sweeps (live DSRs) plus the dsrs-only-at-start baseline,
# no babble (it can't parse the constant_folding/matmul directives). The
# commutative rules would explode the abstraction match set if left unchecked, so
# every run carries the per-factor ``--max-match-set`` cap and an ``--iter-limit``.
TABLE6_DOMAINS = [f"drawings:{d}" for d in ("nuts-bolts", "dials", "wheels", "furniture")]
TABLE6_NUM_ABSTRACTIONS = 4
TABLE6_ITER_LIMIT = 6
# Per-factor row cap (the `--max-match-set` metric is a factor's row count): the
# commutativity blowup is one entangled factor whose equivalent parse trees pile
# up as rows. 24 bounds dials/furniture memory while sparing the high-usage
# patterns; above ~64 furniture's blowup escapes the cap.
TABLE6_MATCH_SET_CAP = 24
# Decompose factors before the row cap can prune them: must be <= the cap (the
# binary asserts it) so a benign independent product just under the cap isn't
# mistaken for an entangled blowup. Pinned equal to the cap.
TABLE6_DECOMPOSE_MIN_ROWS = 24
# Arity 4 (vs the other tables' 2): the drawing domains have deeper repeated
# part-hierarchies, and higher arity both raises absolute compression and widens
# the live-vs-at-start gap (more holes => more normal-form alignment that live can
# exploit but at-start commits away). May revert to 2 for cross-table parity.
TABLE6_MAX_ARITY = 4
TABLE6_TIMEOUT = 300.0  # seconds, per tool invocation


def _table6_runners() -> tuple[tuple[str, object], ...]:
    """Enum/SMC sweeps (live DSRs) plus the dsrs-only-at-start and no-rules Enum
    baselines, every runner at arity 4 with the per-factor match-set cap +
    iter-limit the non-confluent algebra needs (no babble — it can't parse the
    rules)."""
    common = dict(
        max_arity=TABLE6_MAX_ARITY,
        iter_limit=TABLE6_ITER_LIMIT,
        max_match_set=TABLE6_MATCH_SET_CAP,
        decompose_min_rows=TABLE6_DECOMPOSE_MIN_ROWS,
        timeout=TABLE6_TIMEOUT,
        mem_limit=MEM_LIMIT_BYTES,
    )
    return (
        _sweep_runners(**common)
        + (("enum-dsrs-at-start", OursBf(num_steps=BASELINE_BFS_STEPS, only_use_dsrs_at_start=True, **common)),)
        + (("enum-baseline", OursBf(num_steps=BASELINE_BFS_STEPS, no_dsrs=True, **common)),)
    )


def table6() -> Path:
    """Run the cogsci drawing domains with the non-confluent affine-algebra DSRs:
    the table5/7 roster (Enum/SMC sweeps + dsrs-only-at-start and no-rules Enum
    baselines, no babble) at arity 4 with the per-factor match-set cap.
    Demonstrates live > at-start on a non-confluent rule set."""
    _require_free_memory("table6")
    return _run_table(
        domains=TABLE6_DOMAINS,
        runners=_table6_runners(),
        num_abstractions=TABLE6_NUM_ABSTRACTIONS,
        use_dsrs=True,
        folder_prefix="table6",
        output_name="table6.json",
    )


# Table 7: the EPFL circuits with the factoring DSRs. Table 5's roster (Enum/SMC
# sweeps + dsrs-only-at-start) at max-arity 4, plus a no-rules Enum baseline so the
# three-way baseline/live/at-start shows. babble/Stitch have no theory for these
# boolean circuits, so they're dropped.
TABLE7_DOMAINS = EPFL_CIRCUITS.domains
TABLE7_TIMEOUT = 300.0  # seconds, per tool invocation
TABLE7_NUM_ABSTRACTIONS = 4
TABLE7_MAX_ARITY = 4
# Cap e-saturation at 30 iterations (vs the binary default 100). The factoring
# DSRs blow the e-graph up on these cones, and 100 iterations runs ~4-5x slower
# for no better result -- past the timeout at the high sweep points.
TABLE7_ITER_LIMIT = 30
# Enum needs ~2300 expansions of leaf-enumeration warmup before it forms any
# abstraction on these wide corpora, so below that it finishes with an empty
# library (a misleading 1.0); above it, the rule-saturated e-graph never
# converges and it times out. Sweep up to 10k so the representative point is
# past the warmup and lands on a real result (here: DNF). SMC caps at 2000.
# Both representative table/marker points are the renderer's canonical ones
# (TABLE_BFS_STEPS=10000, TABLE_SMC_PARTICLES=1000) -- table7 needs no custom
# point, unlike table5's extended enum sweep (TABLE5_ENUM_POINT=100k).
TABLE7_BFS_SWEEP = tuple(n for n in BFS_STEP_SWEEP if n <= 10000)
TABLE7_SMC_SWEEP = tuple(p for p in SMC_PARTICLE_SWEEP if p <= 2000)


def _table7_runners() -> tuple[tuple[str, object], ...]:
    """Enum/SMC sweeps (live DSRs) plus the dsrs-only-at-start and no-rules Enum
    baselines, every runner at max-arity 4 and capped at :data:`TABLE7_TIMEOUT` /
    :data:`MEM_LIMIT_BYTES`."""
    common = dict(max_arity=TABLE7_MAX_ARITY, iter_limit=TABLE7_ITER_LIMIT, timeout=TABLE7_TIMEOUT, mem_limit=MEM_LIMIT_BYTES)
    return (
        _sweep_runners(bfs_steps=TABLE7_BFS_SWEEP, smc_particles=TABLE7_SMC_SWEEP, **common)
        + (("enum-dsrs-at-start", OursBf(num_steps=BASELINE_BFS_STEPS, only_use_dsrs_at_start=True, **common)),)
        + (("enum-baseline", OursBf(num_steps=BASELINE_BFS_STEPS, no_dsrs=True, **common)),)
    )


def table7() -> Path:
    """Run the EPFL circuits with the factoring DSRs: table5 roster + a no-rules
    Enum baseline, max-arity 4, 4 abstractions, each capped at 300s and 20 GiB."""
    _require_free_memory("table7")
    return _run_table(
        domains=TABLE7_DOMAINS,
        runners=_table7_runners(),
        num_abstractions=TABLE7_NUM_ABSTRACTIONS,
        use_dsrs=True,
        folder_prefix="table7",
        output_name="table7.json",
    )

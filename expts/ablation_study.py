"""
Ablation experiment: how each search optimisation pays off on the single
hardest experiment of tables 3, 5, and 7.

Runs *after* ``results/table{3,5,7}.json`` exist. For each of those tables it:

1. Picks the hardest single-file domain from the latex tables, hard=BFS took longest.
2. Establishes a target compression to reach based on the BFS's configuration but run
    for 1 abstraction, multiplied by 0.99.
3a. Runs BFS ablations with a large step budget and a ``--compression-limit`` stop.
3b. Does a binary search over SMC particle counts to find the smallest that reaches the target.

Everything runs with a single abstraction.

Results are cached per measurement under results/ablation/ and summarized into results/ablation.json.
"""

from __future__ import annotations

import json
import math
import statistics
import time
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Callable

from .bench import MAX_ARITY, MEM_LIMIT_BYTES
from .folders import SUMMARY_RESULTS_DIR, set_folder, summary_results_path
from .render_common import aggregate_time, has_dnf, repeat_cr, reported_sweep_point
from .run_models import OursBf, OursSmc
from .runner import input_files, run_method
from .tables import (
    BFS_STEP_SWEEP, TABLE5_BFS_SWEEP, TABLE5_ENUM_POINT, TABLE5_TIMEOUT,
    TABLE7_BFS_SWEEP, TABLE7_ITER_LIMIT, TABLE7_MAX_ARITY, TABLE7_TIMEOUT, TABLE_BFS_STEPS,
)

BFS_NUM_REPS = 3
SMC_NUM_REPS = 9
# Run for effectively unlimited steps, but stop at 5 minutes or 8GB
BFS_ABLATION_STEPS = 10_000_000
BFS_ABLATION_TIMEOUT = 300.0
BFS_ABLATION_MEM = MEM_LIMIT_BYTES
# Particle-sweep bounds for 3b. The search grows from LO by doubling until a
# point reaches the target (or MAX is exceeded → "never reached"), then bisects.
SMC_START_PARTICLES = 20
SMC_MAX_PARTICLES = 20_000

BFS_ABLATIONS: dict[str, tuple[str, ...]] = {
    "baseline": (),
    "no-lower-bound": ("--lower-bound", "off"),
    "no-dominance": ("--no-opt-dominance-reuse", "--no-opt-useless-inline"),
    "no-equivalence": ("--no-opt-dedup-by-match",),
    "no-var-ordering": ("--freeze-rule", "off"),
    "var-ordering-l2r": ("--var-order", "left-to-right"),
}

SMC_ABLATIONS: dict[str, tuple[str, ...]] = {
    "baseline": (),
    "add-lower-bound": ("--lower-bound", "on"),
    "no-dominance": ("--no-opt-dominance-reuse", "--no-opt-useless-inline"),
    "add-var-ordering": ("--freeze-rule", "on"),
}


@dataclass(frozen=True)
class TableSpec:
    table: int
    max_arity: int
    iter_limit: int | None
    timeout: float | None
    mem_limit: int | None
    # Configured BFS operating point and its sweep, resolved to the reported cell
    # via :func:`reported_sweep_point` (see :func:`_reported_enum_point`).
    enum_point: int
    bfs_sweep: tuple[int, ...]

    @property
    def enum_key(self) -> str:
        """The ``runs`` key of this table's canonical BFS cell in tableN.json."""
        return f"enum-{self.enum_point}"


TABLE_SPECS: dict[int, TableSpec] = {
    3: TableSpec(table=3, max_arity=MAX_ARITY, iter_limit=None, timeout=None,
                 mem_limit=None, enum_point=TABLE_BFS_STEPS, bfs_sweep=BFS_STEP_SWEEP),
    5: TableSpec(table=5, max_arity=MAX_ARITY, iter_limit=None, timeout=TABLE5_TIMEOUT,
                 mem_limit=MEM_LIMIT_BYTES, enum_point=TABLE5_ENUM_POINT, bfs_sweep=TABLE5_BFS_SWEEP),
    7: TableSpec(table=7, max_arity=TABLE7_MAX_ARITY, iter_limit=TABLE7_ITER_LIMIT,
                 timeout=TABLE7_TIMEOUT, mem_limit=MEM_LIMIT_BYTES,
                 enum_point=TABLE_BFS_STEPS, bfs_sweep=TABLE7_BFS_SWEEP),
}


def _load_table(spec: TableSpec) -> dict:
    """Load ``results/table{N}.json`` (the ablation runs after that table)."""
    path = SUMMARY_RESULTS_DIR / f"table{spec.table}.json"
    if not path.exists():
        raise SystemExit(f"ablation: missing {path}; run table{spec.table} first")
    with open(path) as fh:
        return json.load(fh)


def _reported_enum_point(spec: TableSpec, saved: dict) -> int:
    """The BFS point the LaTeX table reports"""
    domain_runs = [d.get("runs", {}) for d in saved["domains"].values()]
    return reported_sweep_point(domain_runs, "enum", spec.bfs_sweep, spec.enum_point)


def hardest_domain(spec: TableSpec, saved: dict) -> str:
    """The table's hardest single-file domain (longest BFS time, skipping multi-file).
    """
    key = spec.enum_key
    best: tuple[str, float] | None = None  # (domain, time)
    for domain, payload in saved["domains"].items():
        if len(input_files(domain)) != 1:  # skip multi-file (dreamcoder) domains
            continue
        reps = payload.get("runs", {}).get(key)
        if not reps or has_dnf(reps):
            raise ValueError(f"ablation: table{spec.table} domain {domain} has no non-DNF single-file BFS ({key}) result")
        t = aggregate_time(reps)
        if t is not None and (best is None or t > best[1]):
            best = (domain, t)
    if best is None:
        raise SystemExit(
            f"ablation: table{spec.table} has no non-DNF single-file BFS ({key}) result"
        )
    return best[0]


# ─── measurement + caching ─────────────────────────────────────────────────


def _cache_path(spec: TableSpec, key: str) -> Path:
    """Per-measurement cache file (delete to force a recompute)."""
    return SUMMARY_RESULTS_DIR / "ablation" / f"table{spec.table}" / f"{key}.json"


def _geomean(vals: list[float]) -> float | None:
    """Geometric mean of ``vals``, or ``None`` when empty."""
    return math.exp(sum(map(math.log, vals)) / len(vals)) if vals else None


def _measure(runner, domain: str, spec: TableSpec, cache_key: str,
             reps: int = BFS_NUM_REPS, cr_median: bool = False) -> dict:
    """Run ``runner`` on ``domain`` for ``reps`` replicates and return
    ``{cr, egg_cr, time, steps, dnf}`` (or ``None`` when any replicate DNF'd).
    ``cr`` is the harness ic/fc metric (the table's standard, for display and the
    SMC bar): the geomean of per-rep CRs, or their **median** when ``cr_median``
    (SMC uses this — robust to an unlucky run missing the razor-thin target).
    ``egg_cr`` is egg-stitch's own reported ``compression_ratio`` — the exact
    metric ``--compression-limit`` checks, fed straight back so the BFS stop is
    reachable. Cached by ``cache_key`` under ``results/ablation/``."""
    cache = _cache_path(spec, cache_key)
    if cache.exists():
        with open(cache) as fh:
            return json.load(fh)
    runs: list[list[dict]] = []
    for _ in range(reps):
        per_file = run_method(runner, domain, rounds=1, use_dsrs=True)
        runs.append([r.to_dict() for r in per_file])
    dnf = has_dnf(runs)
    # Geomean search-work (best-first pops cut short by --compression-limit, or
    # SMC steps) over every (rep, file); deterministic for BFS, averaged for SMC.
    step_vals = [r["num_steps_run"] for run in runs for r in run if r.get("num_steps_run")]
    steps = round(_geomean(step_vals)) if step_vals and not dnf else None
    # egg-stitch's own reported ratio — the exact --compression-limit metric.
    egg_vals = [r["egg_compression_ratio"] for run in runs for r in run if r.get("egg_compression_ratio")]
    # Per-rep CRs aggregated by median (SMC) or geomean (BFS/default).
    rep_crs = [c for c in (repeat_cr(r) for r in runs) if c is not None]
    cr = statistics.median(rep_crs) if cr_median else _geomean(rep_crs)
    out = {
        "cr": None if dnf or not rep_crs else cr,
        "egg_cr": None if dnf else _geomean(egg_vals),
        "time": None if dnf else aggregate_time(runs),
        "steps": steps, "dnf": dnf,
    }
    cache.parent.mkdir(parents=True, exist_ok=True)
    with open(cache, "w") as fh:
        json.dump(out, fh, indent=2)
    return out


def _bfs_runner(spec: TableSpec, flags: tuple[str, ...], limit_cr: float | None) -> OursBf:
    """Best-first runner, in one of two modes keyed on ``limit_cr``:

    * ``limit_cr is None`` — the target-setting run: the table's canonical BFS
      operating point (``spec.enum_point`` steps, ``spec.timeout``), no limit.
      Its output *defines* the targets every ablation is then timed to reach.
    * ``limit_cr`` set — an ablation run: a ``--compression-limit`` stop at that
      ratio, run with the large :data:`BFS_ABLATION_STEPS` budget and bounded by
      :data:`BFS_ABLATION_TIMEOUT` / :data:`BFS_ABLATION_MEM` (see those
      constants for why).

    ``limit_cr`` must be egg's own reported ratio (``egg_cr``), the quantity
    ``--compression-limit`` checks — not the harness ic/fc (see
    :attr:`expts.bench.BenchResult.egg_compression_ratio`). It is rounded *down*
    to 6 decimals (never up), so it can't land a hair above the baseline's own
    achieved ratio — otherwise the baseline (and any tie) would sail past its
    ``>=`` early stop and overcount steps/time.
    """
    if limit_cr is None:
        num_steps, timeout, mem, limit = spec.enum_point, spec.timeout, spec.mem_limit, ()
    else:
        num_steps, timeout, mem = BFS_ABLATION_STEPS, BFS_ABLATION_TIMEOUT, BFS_ABLATION_MEM
        limit = ("--compression-limit", f"{math.floor(limit_cr * 1e6) / 1e6:.6f}")
    return OursBf(
        num_steps=num_steps, max_arity=spec.max_arity, iter_limit=spec.iter_limit,
        timeout=timeout, mem_limit=mem, extra_args=flags + limit,
    )


def _smc_runner(spec: TableSpec, flags: tuple[str, ...], num_particles: int) -> OursSmc:
    """SMC runner for one ablation at ``num_particles`` (no compression limit —
    3b sweeps particles to *reach* the target, then reports the run's time)."""
    # num_steps/temperature left at OursSmc's defaults, matching the table runs.
    return OursSmc(
        num_particles=num_particles, max_arity=spec.max_arity, iter_limit=spec.iter_limit,
        timeout=spec.timeout, mem_limit=spec.mem_limit, extra_args=flags,
    )


def max_compression(spec: TableSpec, domain: str) -> tuple[float, float]:
    """The *max* compression the baseline single-abstraction best-first run
    reaches on ``domain``, as ``(harness_cr, egg_cr)`` — the SMC quality bar and
    the BFS ``--compression-limit`` metric respectively (both anchored to the same
    baseline final_cost). One (deterministic) replicate; cached. Each ablation is
    then timed to reach :data:`TARGET_COMPRESSION_FRACTION` of this (see
    :func:`ablation`)."""
    m = _measure(_bfs_runner(spec, (), None), domain, spec, "bfs_target", reps=1)
    if m["dnf"] or m["cr"] is None or m.get("egg_cr") is None:
        raise SystemExit(f"ablation: baseline BFS DNF'd on {domain}; can't set a target")
    return m["cr"], m["egg_cr"]


def _reached(m: dict, target_cr: float) -> bool:
    """Whether a measurement met the target compression (not DNF, CR ≥ target)."""
    return not m["dnf"] and m["cr"] is not None and m["cr"] >= target_cr


def _find_min_particles(eval_at: Callable[[int], dict], target_cr: float) -> tuple[int | None, dict | None]:
    """Smallest particle count whose measurement reaches ``target_cr``.

    Doubles from :data:`SMC_START_PARTICLES` until a point reaches the target
    (bracketing it), probing :data:`SMC_MAX_PARTICLES` itself before giving up
    (the doubling is capped there so the ceiling is honoured exactly, not
    overshot); if even that doesn't reach, returns ``(None, None)`` — never
    reached. Then bisects the integer particle count. ``eval_at`` is a cached
    measurement of one particle count. Returns ``(particles, measurement)``.
    """
    lo, p, hi, hi_m = 0, SMC_START_PARTICLES, None, None
    while True:
        m = eval_at(p)
        if _reached(m, target_cr):
            hi, hi_m = p, m
            break
        if p >= SMC_MAX_PARTICLES:  # ceiling probed and still short → give up
            break
        lo, p = p, min(p * 2, SMC_MAX_PARTICLES)
    if hi is None:
        return None, None
    while hi - lo > 1:  # bisect the integer particle count in (lo, hi]
        mid = (lo + hi) // 2
        m = eval_at(mid)
        if _reached(m, target_cr):
            hi, hi_m = mid, m
        else:
            lo = mid
    return hi, hi_m


def _run_bfs_ablations(spec: TableSpec, domain: str, limit_cr: float) -> dict[str, dict]:
    """3a: geomean time (over :data:`NUM_REPS`) for every BFS ablation, each
    stopped at ``limit_cr`` (99% of the baseline's egg-reported max) via
    ``--compression-limit``."""
    out: dict[str, dict] = {}
    for name, flags in BFS_ABLATIONS.items():
        m = _measure(_bfs_runner(spec, flags, limit_cr), domain, spec, f"bfs_{name}")
        out[name] = {"time": m["time"], "steps": m["steps"], "cr": m["cr"], "dnf": m["dnf"]}
        print(f"  [table{spec.table}] BFS {name}: time={m['time']}, steps={m['steps']}, cr={m['cr']}", flush=True)
    return out


def _run_smc_ablations(spec: TableSpec, domain: str, target_cr: float) -> dict[str, dict]:
    """3b: for every SMC ablation, the smallest particle count reaching
    ``target_cr`` and that point's geomean time."""
    out: dict[str, dict] = {}
    for name, flags in SMC_ABLATIONS.items():
        def eval_at(p: int, _flags=flags) -> dict:
            return _measure(_smc_runner(spec, _flags, p), domain, spec, f"smc_{name}_p{p}",
                            reps=SMC_NUM_REPS, cr_median=True)

        particles, m = _find_min_particles(eval_at, target_cr)
        if particles is None:
            out[name] = {"particles": None, "time": None, "steps": None, "cr": None, "reached": False}
        else:
            out[name] = {"particles": particles, "time": m["time"], "steps": m["steps"], "cr": m["cr"], "reached": True}
        print(f"  [table{spec.table}] SMC {name}: particles={out[name]['particles']}, time={out[name]['time']}", flush=True)
    return out


def ablation() -> Path:
    """Run the full ablation over tables 3, 5, and 7 and write
    ``results/ablation.json``. Cheap to re-run: every measurement is cached."""
    set_folder(f"ablation/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    results: dict = {"tables": {}}
    for table, spec in TABLE_SPECS.items():
        saved = _load_table(spec)
        # Pin enum_point to the cell the table actually reports (kicked down when
        # the configured point DNFs), so the hardest pick and target run match it.
        spec = replace(spec, enum_point=_reported_enum_point(spec, saved))
        domain = hardest_domain(spec, saved)
        max_cr, max_egg_cr = max_compression(spec, domain)
        # Target = 99% of max, so an ablation that learns the same-quality
        # abstraction counts as reaching it even if it misses the exact
        # minimal-body form by a node.
        target_cr = max_cr * TARGET_COMPRESSION_FRACTION
        egg_cr = max_egg_cr * TARGET_COMPRESSION_FRACTION
        print(f"=== table{table}: hardest={domain}, max CR={max_cr:.4f}, "
              f"target={target_cr:.4f} (egg {egg_cr:.4f}) ===", flush=True)
        results["tables"][str(table)] = {
            "domain": domain,
            "max_cr": max_cr,
            "target_cr": target_cr,
            "egg_cr": egg_cr,
            "target_fraction": TARGET_COMPRESSION_FRACTION,
            "enum_point": spec.enum_point,
            "smc_num_steps": OursSmc.num_steps,
            # BFS stops via --compression-limit at egg's own reported ratio
            # (egg_cr); SMC's _reached check compares the harness ic/fc against
            # target_cr. Both anchor to the same baseline final_cost.
            "bfs": _run_bfs_ablations(spec, domain, egg_cr),
            "smc": _run_smc_ablations(spec, domain, target_cr),
        }
    out_path = summary_results_path("ablation.json")
    with open(out_path, "w") as fh:
        json.dump(results, fh, indent=2)
    print(f"\nwrote {out_path}", flush=True)
    return out_path

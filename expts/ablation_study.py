"""Ablation experiment: how each search optimisation pays off on the single
hardest experiment of tables 3, 5, and 7.

Runs *after* ``results/table{3,5,7}.json`` exist. For each of those tables it:

1. Picks the **hardest experiment** — the *single-file* domain whose canonical
   BFS point (the ``enum-<steps>`` cell reported in the LaTeX table) took the
   longest wall-clock, among domains where BFS actually produced a result (a DNF
   cell has no reported time, so it can't be "hardest"). Multi-file (dreamcoder)
   domains are skipped so the compression target is one per-file ratio.
2. Establishes a **target compression** = the ratio the baseline (all
   optimisations on) best-first run reaches with a single abstraction.
3a. **BFS (``--compression-limit``).** Re-runs best-first with every BFS
    ablation, each stopped the instant it reaches the target compression, and
    reports the geomean wall-clock over :data:`NUM_REPS` replicates. Every
    ablation stops at the same quality, so the times compare like-for-like.
3b. **SMC (particle sweep).** For every SMC ablation, binary-searches the
    smallest particle count whose compression reaches the target, then reports
    that point's geomean wall-clock.

Everything runs with a **single abstraction** (``--num-abstractions 1``), which
is the only mode ``--compression-limit`` supports (there's no one ratio to stop
at once abstractions stack).

The ablations (see :data:`BFS_ABLATIONS` / :data:`SMC_ABLATIONS`):

* no lower-bound pruning (both)      — ``--no-opt-lower-bound``
* no dominance (both)                — ``--no-opt-dominance-reuse`` +
  ``--no-opt-useless-inline`` (the reuse and inlining dominating-successor
  short-circuits both go off together)
* no equivalence pruning (BFS only)  — ``--no-opt-dedup-by-match``
* no variable ordering (BFS only)    — ``--freeze-rule off``
* variable ordering left-to-right    — ``--var-order left-to-right``
* no forced expansion (BFS only)     — ``--priority cost``
* add variable ordering (SMC only)   — ``--freeze-rule on``

Results are cached per measurement under ``results/ablation/`` (delete a file to
recompute it) and summarised into ``results/ablation.json``.
"""

from __future__ import annotations

import json
import math
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

from .bench import MAX_ARITY, MEM_LIMIT_BYTES
from .folders import SUMMARY_RESULTS_DIR, set_folder, summary_results_path
from .render_common import aggregate_cr, aggregate_time, has_dnf
from .run_models import OursBf, OursSmc
from .runner import input_files, run_method
from .tables import TABLE5_ENUM_POINT, TABLE5_TIMEOUT, TABLE7_ITER_LIMIT, TABLE7_MAX_ARITY, TABLE7_TIMEOUT

# Every ablation run learns a single abstraction — the only mode
# `--compression-limit` supports.
NUM_ABSTRACTIONS = 1
# Replicates per measurement. BFS is deterministic, so these only average out
# wall-clock noise; SMC is stochastic, so they also stabilise its geomean CR.
NUM_REPS = 3
# SMC search defaults (matching the table runs: OursSmc's own defaults).
SMC_NUM_STEPS = 100
SMC_TEMPERATURE = 100.0
# Particle-sweep bounds for 3b. The search grows from LO by doubling until a
# point reaches the target (or MAX is exceeded → "never reached"), then bisects.
SMC_START_PARTICLES = 20
SMC_MAX_PARTICLES = 20_000

# The canonical BFS step budget behind each table's ``enum`` cell — table5's
# molecule sweep uses an extended 100k point, the cogsci/circuit tables 10k
# (mirrors ``TABLE_BFS_STEPS`` / ``TABLE5_ENUM_POINT`` in render_tables.py).
TABLE3_ENUM_POINT = 10_000
TABLE7_ENUM_POINT = 10_000

# BFS ablations: the baseline (all optimisations on) plus one knob removed each.
# "no variable ordering" turns the freeze rule off entirely (`--freeze-rule off`);
# "left-to-right" keeps the rule on but swaps the ordering (`--var-order`).
# "no forced expansion" drops the default `forced-then-cost` heap ordering back
# to plain cost (`--priority cost`), so patterns are no longer explored in
# forced-expansion order.
BFS_ABLATIONS: dict[str, tuple[str, ...]] = {
    "baseline": (),
    "no-lower-bound": ("--no-opt-lower-bound",),
    "no-dominance": ("--no-opt-dominance-reuse", "--no-opt-useless-inline"),
    "no-equivalence": ("--no-opt-dedup-by-match",),
    "no-var-ordering": ("--freeze-rule", "off"),
    "var-ordering-l2r": ("--var-order", "left-to-right"),
    "no-forced-expansion": ("--priority", "cost"),
}
# SMC ablations: baseline plus the two shared prunes removed, and the (normally
# off) variable ordering added by turning the freeze rule on (`--freeze-rule on`).
SMC_ABLATIONS: dict[str, tuple[str, ...]] = {
    "baseline": (),
    "no-lower-bound": ("--no-opt-lower-bound",),
    "no-dominance": ("--no-opt-dominance-reuse", "--no-opt-useless-inline"),
    "add-var-ordering": ("--freeze-rule", "on"),
}


@dataclass(frozen=True)
class TableSpec:
    """The table-run configuration the ablation reproduces for one table.

    Mirrors the roster config in :mod:`expts.tables` so the ablation runs the
    hardest domain under the settings that produced its LaTeX cell (arity,
    resource caps, BFS step budget) — except that it always learns a single
    abstraction (:data:`NUM_ABSTRACTIONS`).
    """

    table: int
    max_arity: int
    iter_limit: int | None
    timeout: float | None
    mem_limit: int | None
    enum_point: int

    @property
    def enum_key(self) -> str:
        """The ``runs`` key of this table's canonical BFS cell in tableN.json."""
        return f"enum-{self.enum_point}"


TABLE_SPECS: dict[int, TableSpec] = {
    3: TableSpec(table=3, max_arity=MAX_ARITY, iter_limit=None, timeout=None,
                 mem_limit=None, enum_point=TABLE3_ENUM_POINT),
    5: TableSpec(table=5, max_arity=MAX_ARITY, iter_limit=None, timeout=TABLE5_TIMEOUT,
                 mem_limit=MEM_LIMIT_BYTES, enum_point=TABLE5_ENUM_POINT),
    7: TableSpec(table=7, max_arity=TABLE7_MAX_ARITY, iter_limit=TABLE7_ITER_LIMIT,
                 timeout=TABLE7_TIMEOUT, mem_limit=MEM_LIMIT_BYTES, enum_point=TABLE7_ENUM_POINT),
}


def hardest_domain(spec: TableSpec) -> str:
    """The table's hardest **single-file** BFS experiment.

    Reads ``results/table{N}.json`` and, over the single-file domains where the
    canonical BFS cell finished (non-DNF time), returns the one with the longest
    BFS wall-clock. Multi-file (dreamcoder) domains are excluded so the target is
    a single per-file ratio rather than a cross-file geomean.
    """
    path = SUMMARY_RESULTS_DIR / f"table{spec.table}.json"
    if not path.exists():
        raise SystemExit(f"ablation: missing {path}; run table{spec.table} first")
    with open(path) as fh:
        saved = json.load(fh)
    key = spec.enum_key
    best: tuple[str, float] | None = None  # (domain, time)
    for domain, payload in saved["domains"].items():
        if len(input_files(domain)) != 1:  # skip multi-file (dreamcoder) domains
            continue
        reps = payload.get("runs", {}).get(key)
        if not reps or has_dnf(reps):
            continue
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


def _measure(runner, domain: str, spec: TableSpec, cache_key: str, reps: int = NUM_REPS) -> dict:
    """Run ``runner`` on ``domain`` for ``reps`` replicates and return
    ``{cr, time, dnf}`` (geomean CR / geomean total wall-clock, or ``None`` when
    any replicate DNF'd). Cached by ``cache_key`` under ``results/ablation/``."""
    cache = _cache_path(spec, cache_key)
    if cache.exists():
        with open(cache) as fh:
            return json.load(fh)
    runs: list[list[dict]] = []
    for _ in range(reps):
        per_file = run_method(runner, domain, rounds=NUM_ABSTRACTIONS, use_dsrs=True)
        runs.append([r.to_dict() for r in per_file])
    dnf = has_dnf(runs)
    # Geomean search-work (best-first pops cut short by --compression-limit, or
    # SMC steps) over every (rep, file); deterministic for BFS, averaged for SMC.
    step_vals = [r["num_steps_run"] for run in runs for r in run if r.get("num_steps_run")]
    steps = round(math.exp(sum(map(math.log, step_vals)) / len(step_vals))) if step_vals and not dnf else None
    out = {"cr": None if dnf else aggregate_cr(runs), "time": None if dnf else aggregate_time(runs), "steps": steps, "dnf": dnf}
    cache.parent.mkdir(parents=True, exist_ok=True)
    with open(cache, "w") as fh:
        json.dump(out, fh, indent=2)
    return out


def _bfs_runner(spec: TableSpec, flags: tuple[str, ...], target_cr: float | None) -> OursBf:
    """Best-first runner: the table's BFS config plus the ablation flags and,
    when ``target_cr`` is set, a ``--compression-limit`` stop at that ratio.

    The limit is rounded *down* to 6 decimals (never up), so it can't land a
    hair above the baseline's own achieved CR — otherwise the baseline (and any
    tie) would sail past its `>=` early stop and overcount steps/time."""
    limit = () if target_cr is None else (
        "--compression-limit", f"{math.floor(target_cr * 1e6) / 1e6:.6f}")
    return OursBf(
        num_steps=spec.enum_point, max_arity=spec.max_arity, iter_limit=spec.iter_limit,
        timeout=spec.timeout, mem_limit=spec.mem_limit, extra_args=flags + limit,
    )


def _smc_runner(spec: TableSpec, flags: tuple[str, ...], num_particles: int) -> OursSmc:
    """SMC runner for one ablation at ``num_particles`` (no compression limit —
    3b sweeps particles to *reach* the target, then reports the run's time)."""
    return OursSmc(
        num_particles=num_particles, num_steps=SMC_NUM_STEPS, temperature=SMC_TEMPERATURE,
        max_arity=spec.max_arity, iter_limit=spec.iter_limit,
        timeout=spec.timeout, mem_limit=spec.mem_limit, extra_args=flags,
    )


def target_compression(spec: TableSpec, domain: str) -> float:
    """The single-abstraction compression the baseline best-first run reaches on
    ``domain`` — the quality every ablation is then timed to reach. One
    (deterministic) replicate; cached."""
    m = _measure(_bfs_runner(spec, (), None), domain, spec, "bfs_target", reps=1)
    if m["dnf"] or m["cr"] is None:
        raise SystemExit(f"ablation: baseline BFS DNF'd on {domain}; can't set a target")
    return m["cr"]


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


def _run_bfs_ablations(spec: TableSpec, domain: str, target_cr: float) -> dict[str, dict]:
    """3a: geomean time (over :data:`NUM_REPS`) for every BFS ablation, each
    stopped at ``target_cr`` via ``--compression-limit``."""
    out: dict[str, dict] = {}
    for name, flags in BFS_ABLATIONS.items():
        m = _measure(_bfs_runner(spec, flags, target_cr), domain, spec, f"bfs_{name}")
        out[name] = {"time": m["time"], "steps": m["steps"], "cr": m["cr"], "dnf": m["dnf"]}
        print(f"  [table{spec.table}] BFS {name}: time={m['time']}, steps={m['steps']}, cr={m['cr']}", flush=True)
    return out


def _run_smc_ablations(spec: TableSpec, domain: str, target_cr: float) -> dict[str, dict]:
    """3b: for every SMC ablation, the smallest particle count reaching
    ``target_cr`` and that point's geomean time."""
    out: dict[str, dict] = {}
    for name, flags in SMC_ABLATIONS.items():
        def eval_at(p: int, _flags=flags) -> dict:
            return _measure(_smc_runner(spec, _flags, p), domain, spec, f"smc_{name}_p{p}")

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
        domain = hardest_domain(spec)
        target_cr = target_compression(spec, domain)
        print(f"=== table{table}: hardest={domain}, target CR={target_cr:.4f} ===", flush=True)
        results["tables"][str(table)] = {
            "domain": domain,
            "target_cr": target_cr,
            "enum_point": spec.enum_point,
            "smc_num_steps": SMC_NUM_STEPS,
            "bfs": _run_bfs_ablations(spec, domain, target_cr),
            "smc": _run_smc_ablations(spec, domain, target_cr),
        }
    out_path = summary_results_path("ablation.json")
    with open(out_path, "w") as fh:
        json.dump(results, fh, indent=2)
    print(f"\nwrote {out_path}", flush=True)
    return out_path

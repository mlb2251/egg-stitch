"""Arity-scaling experiment: how BFS and Stitch search time grows with the
abstraction-arity cap.

Unlike tables 1-7 this varies ``max_arity`` (not steps/particles), learns a
single abstraction, and runs each tool to convergence. It runs on the two
cogsci domains whose optimal single abstraction has arity > 2 (wheels, dials),
so raising the cap keeps unlocking better abstractions.

No DSRs: Stitch can't take them, and dropping them isolates the pure
search-cost-vs-arity effect (and matches the no-DSR tables 2/4 where Stitch
participates). Each ``(method, domain)`` sweeps arities upward until a run
exceeds the per-run wall-clock timeout, at which point that curve stops --
Stitch blows up combinatorially long before BFS, which is the point of the
plot. BFS is left free to keep climbing to :data:`ARITY_MAX`.

The sweep is every integer arity 1..20 (where the compression jumps happen),
then a single effectively-unbounded ``1_000_000`` point that lifts the cap
entirely to expose each tool's unbounded-arity search cost. Runs are
deterministic; the repeats only average out wall-clock noise.
"""

from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Callable, Sequence

from tqdm import tqdm

from .bench import MEM_LIMIT_BYTES
from .folders import SUMMARY_RESULTS_DIR, set_folder, summary_results_path
from .run_models import OursBf, Stitch
from .tables import BASELINE_BFS_STEPS

# Domains whose optimal single abstraction needs arity > 2 (so the arity cap is
# the binding constraint). Restricting to these keeps the comparison clean.
ARITY_DOMAINS = ["wheels", "furniture"]
ARITY_TIMEOUT = 500.0  # seconds, per run; a method stops climbing once it blows this
ARITY_NUM_RUNS = 10    # deterministic; repeats only smooth timing noise
ARITY_NUM_ABSTRACTIONS = 1

# Every integer 1..20 (the regime where the compression jumps live) plus one
# effectively-unbounded point that removes the cap entirely.
ARITIES = list(range(1, 21)) + [1_000_000]


# ``(method, make_runner)`` pairs. ``make_runner(arity)`` builds a runner capped
# at that arity. BFS uses a huge step budget so its heap drains (runs to
# convergence) rather than being clipped; Stitch is exhaustive by construction.
def _arity_runners(
    *, timeout: float, mem_limit: int | None
) -> list[tuple[str, Callable[[int], object]]]:
    """The BFS and Stitch runner factories for the arity sweep, both capped at
    ``timeout``/``mem_limit`` and run without DSRs."""
    return [
        ("enum", lambda a: OursBf(
            num_steps=BASELINE_BFS_STEPS, no_dsrs=True,
            max_arity=a, timeout=timeout, mem_limit=mem_limit)),
        ("stitch", lambda a: Stitch(
            max_arity=a, timeout=timeout, mem_limit=mem_limit)),
    ]


def _reps_have_dnf(reps: list[list[dict]]) -> bool:
    """True if any per-file record in ``reps`` timed out / OOM'd."""
    return any(r["compression_ratio"] is None for rep in reps for r in rep)


def _run_arity_sweep(
    method: str,
    make_runner: Callable[[int], object],
    domain: str,
    *,
    arities: Sequence[int],
    num_runs: int,
    cache_path: Path,
    bar: tqdm,
) -> dict[str, list[list[dict]]]:
    """Sweep ``arities`` (ascending) for one ``(method, domain)``, stopping once
    a run DNFs. Returns ``{str(arity): [rep0_per_file, ...]}``.

    Cached incrementally to ``cache_path`` (one JSON per method/domain): each
    completed arity is written immediately, so a killed run resumes where it
    left off. A cached DNF arity means this curve already stopped there.
    """
    from .runner import run_method  # local import: runner pulls heavy deps

    done: dict[str, list[list[dict]]] = {}
    if cache_path.exists():
        with open(cache_path) as fh:
            done = json.load(fh)

    # Iterate ascending so a timeout only stops *larger* arities. A cached DNF
    # sets ``stopped`` when we reach it (not up front), so new arities below a
    # stale high-arity DNF -- e.g. from an earlier, coarser sweep -- still get
    # computed instead of being skipped.
    stopped = False
    for a in sorted(arities):
        key = str(a)
        if key in done:
            if _reps_have_dnf(done[key]):
                stopped = True
            bar.update(num_runs)
            continue
        if stopped:
            bar.update(num_runs)
            continue
        runner = make_runner(a)
        reps: list[list[dict]] = []
        for i in range(num_runs):
            bar.set_description(f"{domain} {method} a={a} rep {i+1}/{num_runs}")
            per_file = run_method(runner, domain, rounds=ARITY_NUM_ABSTRACTIONS, use_dsrs=False)
            rep = [r.to_dict() for r in per_file]
            reps.append(rep)
            bar.update()
            # A DNF at this arity ends the curve; the deterministic remaining
            # repeats would only re-pay an expensive timeout, so skip them.
            if any(r["compression_ratio"] is None for r in rep):
                stopped = True
                bar.update(num_runs - (i + 1))
                break
        done[key] = reps
        cache_path.parent.mkdir(parents=True, exist_ok=True)
        with open(cache_path, "w") as fh:
            json.dump(done, fh, indent=2)
    # Return only this sweep's arities so stale cache keys never reach the plot.
    return {str(a): done[str(a)] for a in arities if str(a) in done}


def arity_experiment(
    *,
    domains: Sequence[str] = ARITY_DOMAINS,
    timeout: float = ARITY_TIMEOUT,
    num_runs: int = ARITY_NUM_RUNS,
    arities: Sequence[int] = ARITIES,
    mem_limit: int | None = MEM_LIMIT_BYTES,
) -> Path:
    """Run the arity-vs-time sweep for BFS and Stitch on ``domains`` and save
    ``results/arity.json`` (rendered by ``scripts/render_arity.py``)."""
    arities = list(arities)
    methods = _arity_runners(timeout=timeout, mem_limit=mem_limit)
    set_folder(f"arity/{time.strftime('%Y-%m-%d_%H-%M-%S')}")
    cache_root = SUMMARY_RESULTS_DIR / "arity"

    results: dict = {
        "config": {
            "num_abstractions": ARITY_NUM_ABSTRACTIONS,
            "timeout": timeout,
            "num_runs": num_runs,
            "use_dsrs": False,
            "arities": arities,
        },
        "domains": {domain: {"methods": {}} for domain in domains},
    }

    total = len(methods) * len(domains) * len(arities) * num_runs
    with tqdm(total=total, unit="run", smoothing=0.05) as bar:
        for method, make_runner in methods:
            for domain in domains:
                done = _run_arity_sweep(
                    method, make_runner, domain,
                    arities=arities, num_runs=num_runs,
                    cache_path=cache_root / f"{method}_{domain}.json", bar=bar,
                )
                results["domains"][domain]["methods"][method] = done

    out_path = summary_results_path("arity.json")
    with open(out_path, "w") as f:
        json.dump(results, f, indent=2)
    print(f"\nwrote {out_path}", flush=True)
    return out_path

#!/usr/bin/env python3
"""Benchmark a PR by running our SMC and best-first searches on two branches
and comparing.

For each (branch, rep) pair we check out, build release, and run
``run_method`` for each (domain, method) once — once with the babble DSRs
("with DSRs") and once without ("without DSRs"). Babble and Stitch are not
invoked; only our two methods are timed. Reps are interleaved between
base and PR (one warmup rep on each branch first, results discarded) so
system-load drift doesn't bias one side. Prints a side-by-side mean
elapsed time and mean compression ratio per (domain, method).

Usage:
    python scripts/bench_pr.py [BASE=main] [PR=<current-branch>]

Env overrides (defaults match the paper-table runner):
    SMC_STEPS=100
    SMC_PARTICLES=1000
    SMC_TEMP=1000.0
    ENUM_STEPS=500
"""

import json
import os
import subprocess
import sys
import time
import numpy as np
from pathlib import Path
from statistics import mean

ROOT = Path(__file__).resolve().parent.parent

DOMAINS = ["nuts-bolts", "dials", "list", "physics"]
# DOMAINS = ["nuts-bolts", "dials"]

NUM_RUNS = 3


def sh(cmd, **kw):
    """Run a subprocess in the repo root, echoing the command first."""
    print("+", " ".join(cmd), flush=True)
    return subprocess.run(cmd, check=True, cwd=ROOT, **kw)


def check_clean_worktree() -> None:
    """Abort if the working tree has any uncommitted or untracked changes.

    bench_pr.py does ``git checkout`` between branches; running it with a
    dirty tree risks an aborted checkout mid-script or, worse, silently
    carrying staged/unstaged edits across branches and contaminating timings.
    """
    dirty = subprocess.check_output(
        ["git", "status", "--porcelain"], cwd=ROOT, text=True
    ).strip()
    if dirty:
        raise SystemExit(
            "bench_pr: working tree is not clean — commit or stash before running.\n"
            + dirty
        )


def run_branch_rep(branch: str, rep_tag: str, smc_steps: int, smc_parts: int, smc_temp: float, enum_steps: int, session: str) -> tuple[Path, Path]:
    """Check out ``branch``, build, run ours-bf / ours-smc once on each DSR
    condition, and return paths to the two per-rep JSON dumps.

    ``rep_tag`` is used as the leaf directory under the per-branch output
    root, so callers can interleave reps across branches without clobbering
    cache files. Pass e.g. ``"warmup"`` for runs that should be discarded.
    """
    sh(["git", "checkout", branch])
    sh(["cargo", "build", "--release", "--quiet"])
    safe = branch.replace("/", "_")
    out_root = ROOT / "viz" / "results" / "bench_pr" / session / safe / rep_tag
    with_path = out_root / "with_dsrs.json"
    without_path = out_root / "without_dsrs.json"
    # Hyperparameters are now fields on OursBf/OursSmc rather than module
    # globals or run_method kwargs, so we instantiate fresh runners with
    # the script's CLI/env settings inside the subprocess.
    py = f"""
import json
import tqdm
from pathlib import Path
from expts.run_models import OursBf, OursSmc
from expts.runner import run_method

def go(name, use_dsrs, out_path):
    runners = {{
        'enum': OursBf(num_steps={enum_steps}),
        'smc': OursSmc(num_steps={smc_steps}, num_particles={smc_parts}, temperature={smc_temp}),
    }}
    out = {{'branch': {branch!r}, 'rep': {rep_tag!r}, 'domains': {{}}}}
    cache_root = Path(out_path).parent / name
    for d in tqdm.tqdm({DOMAINS!r}, desc=f"{{name}} {rep_tag} ({branch!r})"):
        runs = {{}}
        for label, runner in runners.items():
            per_file = run_method(
                runner, d,
                rounds=1, use_dsrs=use_dsrs,
                cache_path=cache_root / label / d / 'res.json',
            )
            runs[label] = [r.to_dict() for r in per_file]
        out['domains'][d] = runs
    p = Path(out_path)
    p.parent.mkdir(parents=True, exist_ok=True)
    with open(p, 'w') as f:
        json.dump(out, f, indent=2)

go('with_dsrs', use_dsrs=True, out_path={str(with_path)!r})
go('without_dsrs', use_dsrs=False, out_path={str(without_path)!r})
"""
    res = subprocess.run([sys.executable, "-c", py], cwd=ROOT)
    if res.returncode != 0:
        raise SystemExit(f"benchmark subprocess failed for {branch} rep={rep_tag} (exit {res.returncode})")
    return with_path, without_path


def summarize(paths: list[Path]) -> dict:
    """Aggregate per-rep, per-file results into ``{domain: {method: {time, compression}}}``.

    ``time`` sums elapsed_secs across files in a rep (total wall time for the
    domain), then averages those totals across reps. ``compression`` averages
    each file's ``compression_ratio`` across all reps and files.
    """
    runs_by_dom: dict[str, dict[str, list[list[dict]]]] = {}
    for path in paths:
        with open(path) as f:
            data = json.load(f)
        for dom, methods in data["domains"].items():
            runs_by_dom.setdefault(dom, {})
            for method, files in methods.items():
                runs_by_dom[dom].setdefault(method, []).append(files)
    out: dict = {}
    for dom, methods in runs_by_dom.items():
        out[dom] = {}
        for method, rs in methods.items():
            if not rs:
                continue
            per_run_time = [sum(r["elapsed_secs"] for r in run) for run in rs]
            all_ratios = [r["compression_ratio"] for run in rs for r in run]
            out[dom][method] = {
                "time": mean(per_run_time),
                "compression": mean(all_ratios),
            }
    return out


def fmt_table(base_label: str, pr_label: str, base: dict, pr: dict, title: str) -> None:
    """Print a side-by-side comparison table for one (DSR-on / DSR-off) condition."""
    print(f"\n=== {title} — {pr_label} vs {base_label} ===")
    header = f"{'domain':<14} {'method':<6}  {'time base[s]':>13} {'time pr[s]':>11} {'speedup':>8}  {'comp base':>10} {'comp pr':>8}"
    print(header)
    print("-" * len(header))
    for m in ("enum", "smc"):
        elements = []
        for dom in DOMAINS:
            b = base[dom][m]
            p = pr[dom][m]
            speedup = b["time"] / p["time"]
            elements.append((b["time"], p["time"], speedup, b["compression"], p["compression"]))
        elements.append(np.prod(elements, axis=0) ** (1 / len(elements)))
        for dom, (t_base, t_pr, speedup, c_base, c_pr) in zip(DOMAINS + ["geomean"], elements):
            print(f"{dom:<14} {m:<6}  {t_base:>13.3f} {t_pr:>11.3f} {speedup:>7.2f}x  {c_base:>10.3f} {c_pr:>8.3f}")


def main() -> None:
    """CLI entry point; see module docstring for the argument shape."""
    args = sys.argv[1:]
    base = args[0] if len(args) >= 1 else "main"
    pr = args[1] if len(args) >= 2 else subprocess.check_output(["git", "branch", "--show-current"], cwd=ROOT, text=True).strip()
    smc_steps = int(os.environ.get("SMC_STEPS", 100))
    smc_parts = int(os.environ.get("SMC_PARTICLES", 1000))
    smc_temp = float(os.environ.get("SMC_TEMP", 1000.0))
    enum_steps = int(os.environ.get("ENUM_STEPS", 500))
    session = time.strftime("%Y-%m-%d_%H-%M-%S")

    check_clean_worktree()

    print(f"base={base}  pr={pr}  NUM_RUNS={NUM_RUNS}  smc=({smc_steps} steps, {smc_parts} particles, T={smc_temp})  enum_steps={enum_steps}  session={session}")

    common = (smc_steps, smc_parts, smc_temp, enum_steps, session)

    # Warmup: one rep on each branch, results discarded. Soaks up cold-cache
    # / first-build costs so the timed reps measure steady-state behavior.
    print("\n=== warmup (results discarded) ===")
    run_branch_rep(base, "warmup", *common)
    run_branch_rep(pr, "warmup", *common)

    # Timed reps, interleaved between base and PR per rep so any drift in
    # system load is shared across both sides rather than biasing one.
    base_with, base_without = [], []
    pr_with, pr_without = [], []
    for i in range(NUM_RUNS):
        b_w, b_wo = run_branch_rep(base, f"rep{i}", *common)
        base_with.append(b_w); base_without.append(b_wo)
        p_w, p_wo = run_branch_rep(pr, f"rep{i}", *common)
        pr_with.append(p_w); pr_without.append(p_wo)

    fmt_table(base, pr, summarize(base_with), summarize(pr_with), "with DSRs")
    fmt_table(base, pr, summarize(base_without), summarize(pr_without), "without DSRs")


if __name__ == "__main__":
    main()

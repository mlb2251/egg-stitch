#!/usr/bin/env python3
"""Benchmark a PR by running our SMC and best-first searches on two branches
and comparing.

Builds the egg-stitch binary in two ephemeral git worktrees (one at ``BASE``,
one at ``PR``), then drives all measurements from a single Python process,
swapping ``egg_stitch_bin`` between the two binaries per measurement. No
``git checkout`` happens in the main repo. For each (rep, domain, method,
DSR condition) we run base then PR back-to-back. The first rep is treated
as warmup and dropped from the aggregate. Babble and Stitch are not
invoked; only our two methods are timed. Prints a side-by-side mean elapsed
time and mean compression ratio per (domain, method).

Usage:
    python scripts/bench_pr.py [BASE=main] [PR=<current-branch>]

Env overrides (defaults match the paper-table runner):
    SMC_STEPS=100
    SMC_PARTICLES=1000
    SMC_TEMP=1000.0
    ENUM_STEPS=500
"""

import json
import math
import os
import subprocess
import sys
import time
import numpy as np
import tqdm
from itertools import product
from pathlib import Path
from statistics import mean, stdev

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

from expts.result import PerFileResult  # noqa: E402
from expts.run_models import OursBf, OursSmc  # noqa: E402
from expts.run_models import ours as _ours_mod  # noqa: E402
from expts.runner import run_method  # noqa: E402

DOMAINS = ["nuts-bolts", "dials", "list", "physics"]
# DOMAINS = ["nuts-bolts", "dials"]

# Adaptive rep count: keep adding reps until every cell's relative SEM
# (stdev / sqrt(n) / mean) is below TARGET_REL_SEM on both branches, or
# until MAX_RUNS. We can't use a fixed seed across branches (RNG draws
# don't have coherent meaning when search semantics change), so the only
# honest way to make the comparison meaningful is to drive the per-cell
# uncertainty down by sampling.
MIN_RUNS = 5
MAX_RUNS = 60
TARGET_REL_SEM = 0.02


def sh(cmd, *, cwd=None, **kw):
    """Run a subprocess, echoing the command first. Defaults cwd to the repo root."""
    print("+", " ".join(str(c) for c in cmd), flush=True)
    return subprocess.run(cmd, check=True, cwd=cwd or ROOT, **kw)


def check_clean_worktree() -> None:
    """Abort if the main worktree has any uncommitted or untracked changes.

    We don't ``git checkout`` here anymore — worktrees handle that — but a
    dirty tree usually indicates the user is mid-edit, which is rarely what
    they want to benchmark.
    """
    dirty = subprocess.check_output(
        ["git", "status", "--porcelain"], cwd=ROOT, text=True
    ).strip()
    if dirty:
        raise SystemExit(
            "bench_pr: working tree is not clean — commit or stash before running.\n"
            + dirty
        )


def setup_worktree(branch: str, wt_dir: Path) -> Path:
    """Create a git worktree for ``branch`` at ``wt_dir``, build release, return binary path."""
    # --detach so we don't conflict with whichever branch the main worktree
    # currently has checked out (commonly the PR branch itself).
    sh(["git", "worktree", "add", "--detach", str(wt_dir), branch])
    sh(["cargo", "build", "--release", "--bin", "egg-stitch", "--quiet"], cwd=wt_dir)
    return wt_dir / "target" / "release" / "egg-stitch"


def teardown_worktree(wt_dir: Path) -> None:
    """Remove a git worktree (force-removing despite the untracked ``target/``)."""
    subprocess.run(
        ["git", "worktree", "remove", "--force", str(wt_dir)],
        cwd=ROOT, check=False,
    )


def time_cell(binary_path: Path, runner, domain: str, use_dsrs: bool, cache_path: Path):
    """Run one (binary, domain, method, condition) cell, going through the cache.

    Monkey-patches ``expts.run_models.ours.egg_stitch_bin`` so ``run_method``
    invokes the requested binary; the rest of the pipeline (cwd, env,
    command-line construction) is shared between branches.
    """
    if cache_path.exists():
        with open(cache_path) as f:
            return [PerFileResult(**d) for d in json.load(f)]
    _ours_mod.egg_stitch_bin = lambda: binary_path
    out = run_method(runner, domain, rounds=1, use_dsrs=use_dsrs)
    cache_path.parent.mkdir(parents=True, exist_ok=True)
    with open(cache_path, "w") as f:
        json.dump([r.to_dict() for r in out], f, indent=2)
    return out


def cache_path_for(session: str, branch_label: str, dsr_label: str, method: str, domain: str, rep_idx: int) -> Path:
    """Per-cell cache file path. Unique per (branch, condition, method, domain, rep)."""
    return (
        ROOT / "viz" / "results" / "bench_pr" / session
        / branch_label / dsr_label / method / domain / f"rep{rep_idx}.json"
    )


def cell_per_rep_times(session: str, branch_label: str, dsr_label: str, method: str, domain: str, num_reps: int) -> list[float]:
    """Per-rep summed elapsed_secs for one cell (reps 1..num_reps; rep 0 is warmup)."""
    out = []
    for rep in range(1, num_reps + 1):
        p = cache_path_for(session, branch_label, dsr_label, method, domain, rep)
        with open(p) as f:
            files = json.load(f)
        out.append(sum(r["elapsed_secs"] for r in files))
    return out


def cell_compressions(session: str, branch_label: str, dsr_label: str, method: str, domain: str, num_reps: int) -> list[float]:
    """Per-file compression ratios across reps 1..num_reps for one cell."""
    out = []
    for rep in range(1, num_reps + 1):
        p = cache_path_for(session, branch_label, dsr_label, method, domain, rep)
        with open(p) as f:
            files = json.load(f)
        out.extend(r["compression_ratio"] for r in files)
    return out


def rel_sem(xs: list[float]) -> float:
    """stdev/sqrt(n)/mean — the SEM as a fraction of the mean. inf if mean=0 or n<2."""
    if len(xs) < 2:
        return float("inf")
    m = mean(xs)
    if m == 0:
        return float("inf")
    return stdev(xs) / math.sqrt(len(xs)) / m


def summarize(session: str, branch_label: str, dsr_label: str, methods: list[str], reps_for: dict) -> dict:
    """Aggregate cached per-cell results (dropping rep 0 as warmup) into
    ``{domain: {method: {time, compression}}}``.

    Per-rep ``time`` sums elapsed_secs across files of a domain; the cell's
    reported ``time`` is the mean of those per-rep totals. ``compression``
    is the mean of every file's compression_ratio across the kept reps.
    ``reps_for`` maps ``(dsr_label, domain, method) -> num_reps`` since
    different cells may have run different rep counts under the adaptive
    sampler.
    """
    out: dict = {}
    for domain in DOMAINS:
        out[domain] = {}
        for method in methods:
            n = reps_for[(dsr_label, domain, method)]
            times = cell_per_rep_times(session, branch_label, dsr_label, method, domain, n)
            ratios = cell_compressions(session, branch_label, dsr_label, method, domain, n)
            out[domain][method] = {
                "time": mean(times),
                "compression": mean(ratios),
            }
    return out


def update_pr_timing(pr_branch: str, timing_section: str) -> None:
    """Replace (or append) the ``## Timing`` section in the PR description.

    Looks up the open PR for ``pr_branch`` via ``gh``; if none exists, prints
    a warning and returns. ``timing_section`` must start with ``## Timing``.
    The section is replaced from its heading up to (but not including) the
    next ``## `` heading or EOF; if no existing section is found it's
    appended (separated by a blank line).
    """
    try:
        body = subprocess.check_output(
            ["gh", "pr", "view", pr_branch, "--json", "body", "-q", ".body"],
            cwd=ROOT, text=True, stderr=subprocess.PIPE,
        )
    except subprocess.CalledProcessError as e:
        print(f"\nbench_pr: no PR found for branch {pr_branch!r}, skipping PR update.\n  {e.stderr.strip()}")
        return
    body = body.rstrip("\n")
    # Match "## Timing" up to (but not including) the next "## " or EOF.
    import re
    pattern = re.compile(r"(?m)^## Timing\b.*?(?=^## |\Z)", re.DOTALL)
    if pattern.search(body):
        new_body = pattern.sub(timing_section.rstrip() + "\n\n", body).rstrip() + "\n"
    else:
        sep = "\n\n" if body else ""
        new_body = body + sep + timing_section.rstrip() + "\n"
    res = subprocess.run(
        ["gh", "pr", "edit", pr_branch, "--body-file", "-"],
        cwd=ROOT, input=new_body, text=True, capture_output=True,
    )
    if res.returncode != 0:
        print(f"\nbench_pr: gh pr edit failed (exit {res.returncode}):\n{res.stderr}")
    else:
        print(f"\nbench_pr: updated Timing section on PR for {pr_branch}.")


def _speedup_emoji(speedup: float) -> str:
    """Green for >1.02, red for <0.98, gray for the in-between band."""
    if speedup > 1.02:
        return "🟢"
    if speedup < 0.98:
        return "🔴"
    return "⚪"


def fmt_table(base_label: str, pr_label: str, base: dict, pr: dict, title: str) -> str:
    """Return a GitHub-flavored markdown comparison table for one DSR condition."""
    lines = [
        f"### {title} — `{pr_label}` vs `{base_label}`",
        "",
        f"|   | domain | method | time `{base_label}` [s] | time `{pr_label}` [s] | speedup | comp `{base_label}` | comp `{pr_label}` |",
        "|---|---|---|---:|---:|---:|---:|---:|",
    ]
    for m in ("enum", "smc"):
        elements = []
        for dom in DOMAINS:
            b = base[dom][m]
            p = pr[dom][m]
            speedup = b["time"] / p["time"]
            elements.append((b["time"], p["time"], speedup, b["compression"], p["compression"]))
        elements.append(np.prod(elements, axis=0) ** (1 / len(elements)))
        for dom, (t_base, t_pr, speedup, c_base, c_pr) in zip(DOMAINS + ["geomean"], elements):
            comp_warn = " ‼️" if c_pr / c_base < 0.99 else ""
            lines.append(f"| {_speedup_emoji(speedup)}{comp_warn} | {dom} | {m} | {t_base:.3f} | {t_pr:.3f} | {speedup:.2f}x | {c_base:.3f} | {c_pr:.3f} |")
    return "\n".join(lines)


def main() -> None:
    """CLI entry point; see module docstring for the argument shape."""
    args = sys.argv[1:]
    base = args[0] if len(args) >= 1 else "main"
    pr = args[1] if len(args) >= 2 else subprocess.check_output(["git", "branch", "--show-current"], cwd=ROOT, text=True).strip()
    smc_steps = int(os.environ.get("SMC_STEPS", 100))
    smc_parts = int(os.environ.get("SMC_PARTICLES", 1000))
    smc_temp = float(os.environ.get("SMC_TEMP", 1000.0))
    enum_steps = int(os.environ.get("ENUM_STEPS", 5000))
    session = time.strftime("%Y-%m-%d_%H-%M-%S")

    check_clean_worktree()

    print(f"base={base}  pr={pr}  reps=adaptive(min={MIN_RUNS}, max={MAX_RUNS}, target rel-SEM<{TARGET_REL_SEM:.0%})+1warmup  smc=({smc_steps} steps, {smc_parts} particles, T={smc_temp})  enum_steps={enum_steps}  session={session}")

    wt_root = Path(f"/tmp/bench_pr_{session}")
    wt_base = wt_root / "base"
    wt_pr = wt_root / "pr"
    try:
        base_bin = setup_worktree(base, wt_base)
        pr_bin = setup_worktree(pr, wt_pr)

        runners = {
            "enum": OursBf(num_steps=enum_steps),
            "smc": OursSmc(num_steps=smc_steps, num_particles=smc_parts, temperature=smc_temp),
        }
        conditions = [("with_dsrs", True), ("without_dsrs", False)]
        # Each cell is keyed by (dsr_label, domain, method); runner + use_dsrs
        # are recovered from these lookup tables.
        runner_for = dict(runners.items())
        use_dsrs_for = dict(conditions)
        cell_keys = [(d, dom, m) for (d, _), dom, (m, _) in product(conditions, DOMAINS, runners.items())]

        def run_rep_for(cell: tuple[str, str, str], rep_idx: int) -> None:
            """Run one rep of one cell on base then PR back-to-back."""
            dsr_label, domain, method = cell
            runner = runner_for[method]
            use_dsrs = use_dsrs_for[dsr_label]
            time_cell(base_bin, runner, domain, use_dsrs,
                      cache_path_for(session, "base", dsr_label, method, domain, rep_idx))
            time_cell(pr_bin, runner, domain, use_dsrs,
                      cache_path_for(session, "pr", dsr_label, method, domain, rep_idx))

        def cell_rel_sem(cell: tuple[str, str, str], num_reps: int) -> float:
            """Max rel-SEM across base & PR for one cell at num_reps."""
            dsr_label, domain, method = cell
            return max(
                rel_sem(cell_per_rep_times(session, b, dsr_label, method, domain, num_reps))
                for b in ("base", "pr")
            )

        # Phase 1: warmup + MIN_RUNS reps for every cell. Phase 2: keep
        # adding reps only to cells whose paired rel-SEM (max of base/PR)
        # is still above TARGET_REL_SEM, capped at MAX_RUNS per cell.
        reps_done: dict[tuple[str, str, str], int] = {c: 0 for c in cell_keys}

        def cell_done(cell: tuple[str, str, str]) -> bool:
            n = reps_done[cell]
            if n < MIN_RUNS:
                return False
            if n >= MAX_RUNS:
                return True
            return cell_rel_sem(cell, n) < TARGET_REL_SEM

        # Warmup rep (rep 0) for every cell, in cell-key order.
        wpbar = tqdm.tqdm(cell_keys, desc="warmup", unit="cell", leave=False)
        for cell in wpbar:
            wpbar.set_postfix_str("/".join(cell))
            run_rep_for(cell, 0)

        # Adaptive sampling: each outer iteration runs one more rep for
        # every still-unconverged cell. Stop when no cell needs more reps.
        round_idx = 0
        while True:
            pending = [c for c in cell_keys if not cell_done(c)]
            if not pending:
                break
            round_idx += 1
            pbar = tqdm.tqdm(pending, desc=f"round {round_idx} ({len(pending)} cells)", unit="cell", leave=False)
            for cell in pbar:
                reps_done[cell] += 1
                pbar.set_postfix_str(f"{'/'.join(cell)} rep{reps_done[cell]}")
                run_rep_for(cell, reps_done[cell])
            for cell in pending:
                n = reps_done[cell]
                r = cell_rel_sem(cell, n) if n >= 2 else float("inf")
                print(f"  {'/'.join(cell)}: {n} reps, rel-SEM={r:.2%}{' ✓' if r < TARGET_REL_SEM else ''}", flush=True)

        for cell, n in reps_done.items():
            if n >= MAX_RUNS and cell_rel_sem(cell, n) >= TARGET_REL_SEM:
                print(f"  WARN: {'/'.join(cell)} hit MAX_RUNS={MAX_RUNS} without converging "
                      f"(rel-SEM {cell_rel_sem(cell, n):.2%})", flush=True)

        methods = list(runners.keys())
        with_reps = {(d, dom, m): reps_done[(d, dom, m)] for d in ("with_dsrs",) for dom in DOMAINS for m in methods}
        without_reps = {(d, dom, m): reps_done[(d, dom, m)] for d in ("without_dsrs",) for dom in DOMAINS for m in methods}
        with_md = fmt_table(base, pr,
                            summarize(session, "base", "with_dsrs", methods, with_reps),
                            summarize(session, "pr", "with_dsrs", methods, with_reps),
                            "with DSRs")
        without_md = fmt_table(base, pr,
                               summarize(session, "base", "without_dsrs", methods, without_reps),
                               summarize(session, "pr", "without_dsrs", methods, without_reps),
                               "without DSRs")
        timing_section = "## Timing\n\n" + with_md + "\n\n" + without_md + "\n"
        print()
        print(timing_section)
        update_pr_timing(pr, timing_section)
    finally:
        teardown_worktree(wt_base)
        teardown_worktree(wt_pr)


if __name__ == "__main__":
    main()

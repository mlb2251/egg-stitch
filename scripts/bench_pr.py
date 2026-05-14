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
import os
import subprocess
import sys
import time
import numpy as np
import tqdm
from itertools import product
from pathlib import Path
from statistics import mean

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))

from expts.run_models import OursBf, OursSmc  # noqa: E402
from expts.run_models import ours as _ours_mod  # noqa: E402
from expts.runner import run_method  # noqa: E402

DOMAINS = ["nuts-bolts", "dials", "list", "physics"]
# DOMAINS = ["nuts-bolts", "dials"]

NUM_RUNS = 15  # plus one warmup rep that gets discarded


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
    _ours_mod.egg_stitch_bin = lambda: binary_path
    return run_method(runner, domain, rounds=1, use_dsrs=use_dsrs, cache_path=cache_path)


def cache_path_for(session: str, branch_label: str, dsr_label: str, method: str, domain: str, rep_idx: int) -> Path:
    """Per-cell cache file path. Unique per (branch, condition, method, domain, rep)."""
    return (
        ROOT / "viz" / "results" / "bench_pr" / session
        / branch_label / dsr_label / method / domain / f"rep{rep_idx}.json"
    )


def summarize(session: str, branch_label: str, dsr_label: str, methods: list[str]) -> dict:
    """Aggregate cached per-cell results (dropping rep 0 as warmup) into
    ``{domain: {method: {time, compression}}}``.

    Per-rep ``time`` sums elapsed_secs across files of a domain; the cell's
    reported ``time`` is the mean of those per-rep totals. ``compression``
    is the mean of every file's compression_ratio across the kept reps.
    """
    out: dict = {}
    for domain in DOMAINS:
        out[domain] = {}
        for method in methods:
            per_run_time = []
            all_ratios = []
            for rep in range(1, NUM_RUNS + 1):  # rep 0 is warmup, dropped
                p = cache_path_for(session, branch_label, dsr_label, method, domain, rep)
                with open(p) as f:
                    files = json.load(f)
                per_run_time.append(sum(r["elapsed_secs"] for r in files))
                all_ratios.extend(r["compression_ratio"] for r in files)
            out[domain][method] = {
                "time": mean(per_run_time),
                "compression": mean(all_ratios),
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
    enum_steps = int(os.environ.get("ENUM_STEPS", 500))
    session = time.strftime("%Y-%m-%d_%H-%M-%S")

    check_clean_worktree()

    print(f"base={base}  pr={pr}  NUM_RUNS={NUM_RUNS}+1warmup  smc=({smc_steps} steps, {smc_parts} particles, T={smc_temp})  enum_steps={enum_steps}  session={session}")

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

        # rep 0 is warmup (dropped by summarize); reps 1..NUM_RUNS are timed.
        # Within each cell we always run base then PR back-to-back so the two
        # binaries see the most similar system state we can give them.
        cells = list(product(range(NUM_RUNS + 1), conditions, DOMAINS, runners.items()))
        pbar = tqdm.tqdm(cells, desc="bench_pr", unit="cell")
        for rep_idx, (dsr_label, use_dsrs), domain, (method, runner) in pbar:
            tag = "warmup" if rep_idx == 0 else f"rep{rep_idx}"
            pbar.set_postfix_str(f"{tag} {dsr_label} {domain} {method}")
            time_cell(base_bin, runner, domain, use_dsrs,
                      cache_path_for(session, "base", dsr_label, method, domain, rep_idx))
            time_cell(pr_bin, runner, domain, use_dsrs,
                      cache_path_for(session, "pr", dsr_label, method, domain, rep_idx))

        methods = list(runners.keys())
        with_md = fmt_table(base, pr,
                            summarize(session, "base", "with_dsrs", methods),
                            summarize(session, "pr", "with_dsrs", methods),
                            "with DSRs")
        without_md = fmt_table(base, pr,
                               summarize(session, "base", "without_dsrs", methods),
                               summarize(session, "pr", "without_dsrs", methods),
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

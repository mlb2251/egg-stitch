"""Wrappers around our egg-stitch compressor binary.

Exposes two table-targeted entry points (:func:`run_ours_smc`,
:func:`run_ours_bf`) sharing a common subprocess body, plus a low-level
:func:`egg_stitch` escape hatch for ad-hoc dev experiments (used by
``run.py``).
"""

import json
import os
import subprocess
from pathlib import Path

from .. import EGG_STITCH_BIN, EGG_STITCH_DIR
from ..bench import Abstraction, BenchResult, MAX_ARITY, Weighting
from ..folders import current_folder_path, unique_path


# ─── Hyperparameters ───────────────────────────────────────────────────────
# Patch these at module level for one-off overrides; otherwise treat as fixed.

# SMC search
SMC_NUM_STEPS = 100
SMC_NUM_PARTICLES = 1000
SMC_TEMPERATURE = 1000.0

# Best-first (enum) search
BF_NUM_STEPS = 500

# Pass ``--rebuild-egraph`` to egg-stitch. Required when stacking many
# abstractions in one run (Tables 3/4) so the e-graph stays consistent
# after each successive abstraction is applied; off for single-abstraction
# runs since rebuilding is wasted work then.
REBUILD_EGRAPH = False


def egg_stitch(input, output="out.json", rewrites=None, flamegraph=False, samply=False, **kwargs) -> Path:
    """Low-level escape hatch: run the egg-stitch binary with arbitrary CLI flags.

    Used by ``run.py`` for ad-hoc dev experiments where the table-runner API
    is too coarse. ``output`` is interpreted relative to the current results
    folder. ``flamegraph=True`` profiles via ``cargo flamegraph`` (macOS,
    needs sudo); ``samply=True`` profiles via ``samply record``. All other
    kwargs are forwarded as ``--key value`` (or ``--key`` for ``True``
    booleans).
    """
    output_path = unique_path(current_folder_path() / output)
    prog_args = ["-i", input, "--output", str(output_path)]
    if flamegraph:
        svg_path = str(output_path).replace(".json", "_flamegraph.svg")
        cmd = ["cargo", "flamegraph", "--root", "-o", svg_path, "--", *prog_args]
    elif samply:
        cmd = ["samply", "record", str(EGG_STITCH_BIN), *prog_args]
    else:
        cmd = [str(EGG_STITCH_BIN), *prog_args]
    if rewrites is not None:
        cmd += ["-r", rewrites]
    for k, v in kwargs.items():
        flag = "--" + k.replace("_", "-")
        if isinstance(v, bool):
            if v:
                cmd.append(flag)
            continue
        cmd += [flag, str(v)]
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, check=True, env=dict(os.environ, RUST_BACKTRACE="1"))
    return output_path


def _run(*, rounds: int, input_path: Path, rewrites_path: str | None,
         weighting: Weighting, search: str, extra_flags: dict[str, object]) -> BenchResult:
    """Shared body for the SMC/best-first wrappers; only the search-kind-
    specific flags differ between them."""
    output_path = unique_path(
        current_folder_path() / f"{input_path.stem}_{search.replace('-', '_')}.json"
    )
    language = "op-children" if weighting == "no-apps" else "lambda-calc"
    cmd: list[str] = [
        str(EGG_STITCH_BIN),
        "-i", str(input_path),
        "--output", str(output_path),
        "--search", search,
        "--language", language,
        "--max-arity", str(MAX_ARITY),
        "--num-abstractions", str(rounds),
        # cogsci/no-apps tables suppress 0-arity abstractions to match how
        # babble/stitch are invoked; lambda-calc runs use the same setting
        # since the table comparison is symmetric.
        "--no-zero-arity",
    ]
    if REBUILD_EGRAPH:
        cmd.append("--rebuild-egraph")
    if rewrites_path is not None:
        cmd += ["-r", rewrites_path]
    for k, v in extra_flags.items():
        flag = "--" + k.replace("_", "-")
        if isinstance(v, bool):
            if v:
                cmd.append(flag)
        else:
            cmd += [flag, str(v)]
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, check=True, cwd=EGG_STITCH_DIR, env=dict(os.environ, RUST_BACKTRACE="1"))
    with open(output_path) as f:
        data = json.load(f)
    abstractions = [
        Abstraction(name=f"fn_{i}", body=a["pattern"])
        for i, a in enumerate(data.get("library", []))
    ]
    return BenchResult(
        elapsed_secs=float(data["elapsed_secs"]),
        initial_corpus=list(data["original_programs"]),
        final_corpus=list(data["rewritten_programs"]),
        abstractions=abstractions,
        cost_after_rewrites=(
            int(data["cost_after_rewrites"]) if rewrites_path is not None else None
        ),
    )


def run_ours_smc(rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
    """Run egg-stitch in SMC mode on a single ``input_path``."""
    return _run(
        rounds=rounds, input_path=input_path, rewrites_path=rewrites_path,
        weighting=weighting, search="smc",
        extra_flags={
            "num_steps": SMC_NUM_STEPS,
            "num_particles": SMC_NUM_PARTICLES,
            "temperature": SMC_TEMPERATURE,
        },
    )


def run_ours_bf(rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
    """Run egg-stitch in best-first (enum) mode on a single ``input_path``."""
    return _run(
        rounds=rounds, input_path=input_path, rewrites_path=rewrites_path,
        weighting=weighting, search="best-first",
        extra_flags={"num_steps": BF_NUM_STEPS},
    )

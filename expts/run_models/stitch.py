"""Wrapper around the external stitch compressor.

stitch doesn't accept DSRs, so the wrapper asserts ``rewrites_path is None``.
The cost-flag selection is what keeps stitch's internal scoring lined up with
the runner's uniform :func:`expts.runner.ast_size`: at ``no-apps`` weighting
all non-app costs are huge so the fixed App=1 is negligible vs. the
node-count metric; at ``apps-equal`` they're all 1.
"""

import json
import subprocess
import time
from pathlib import Path

from .._build import cargo_build, check_clean_main
from ..bench import Abstraction, BenchResult, MAX_ARITY, Weighting
from ..folders import current_folder_path, unique_path


# Stitch lives as a sibling clone of this repo; verify it's on a clean,
# up-to-date main before building so reported numbers are reproducible.
STITCH_DIR: Path = (Path(__file__).resolve().parent.parent.parent.parent / "stitch").resolve()
check_clean_main(STITCH_DIR, "git@github.com:mlb2251/stitch.git")
STITCH_BIN: Path = cargo_build(STITCH_DIR, "compress")


def run_stitch(rounds: int, input_path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
    """Run stitch on a single ``input_path``."""
    assert rewrites_path is None, "stitch doesn't accept DSRs"
    cost = "1" if weighting == "apps-equal" else "10000"
    out_path = unique_path(current_folder_path() / f"{input_path.stem}_stitch.json")
    cmd = [
        str(STITCH_BIN),
        str(input_path),
        f"-i{rounds}",
        f"-a{MAX_ARITY}",
        "--out", str(out_path),
        "--no-curried-bodies",
        "--no-curried-metavars",
        "--silent",
        "--allow-single-task",
        "--cost-app", "1",
        "--cost-var", cost,
        "--cost-ivar", cost,
        "--cost-prim-default", cost,
        "--cost-lam", cost,
    ]
    print("+", " ".join(cmd), flush=True)
    start = time.time()
    subprocess.run(cmd, check=True)
    elapsed = time.time() - start
    with open(out_path) as f:
        data = json.load(f)
    abstractions = [
        Abstraction(name=a.get("name", f"fn_{i}"), body=a["body"])
        for i, a in enumerate(data.get("abstractions", []))
    ]
    return BenchResult(
        elapsed_secs=elapsed,
        initial_corpus=list(data["original"]),
        final_corpus=list(data["rewritten"]),
        abstractions=abstractions,
    )

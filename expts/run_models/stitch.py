"""Wrapper around the external stitch compressor.

stitch doesn't accept DSRs, so the runner asserts ``rewrites_path is None``.
The cost-flag selection keeps stitch's internal scoring lined up with the
runner's uniform :func:`expts.runner.ast_size`: at ``no-apps`` weighting all
non-app costs are huge so the fixed App=1 is negligible vs. the node-count
metric; at ``apps-equal`` they're all 1.
"""

import json
import time
from dataclasses import dataclass
from functools import cache
from pathlib import Path

from .._build import cargo_build, check_clean_main
from .._subproc import run as _subproc_run
from ..bench import Abstraction, BenchResult, MAX_ARITY, Weighting
from ..folders import current_folder_path, unique_path


# Stitch lives as a sibling clone of this repo.
STITCH_DIR: Path = (Path(__file__).resolve().parent.parent.parent.parent / "stitch").resolve()


@cache
def stitch_bin() -> Path:
    """Verify ``../stitch`` is clean+synced, build, and return the binary path.

    Lazy + cached so importing this module is cheap and doesn't fetch from
    origin / shell out to cargo until someone actually wants to invoke
    stitch.
    """
    check_clean_main(STITCH_DIR, "git@github.com:mlb2251/stitch.git")
    return cargo_build(STITCH_DIR, "compress")


@dataclass(frozen=True)
class Stitch:
    """Run stitch on a single input file."""

    max_arity: int = MAX_ARITY

    def __call__(self, rounds: int, input_path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
        assert rewrites_path is None, "stitch doesn't accept DSRs"
        cost = "1" if weighting == "apps-equal" else "10000"
        out_path = unique_path(current_folder_path() / f"{input_path.stem}_stitch.json")
        cmd = [
            str(stitch_bin()),
            str(input_path),
            f"-i{rounds}",
            f"-a{self.max_arity}",
            "--out", str(out_path),
            "--silent",
            "--allow-single-task",
            "--cost-app", "1",
            "--cost-var", cost,
            "--cost-ivar", cost,
            "--cost-prim-default", cost,
            "--cost-lam", cost,
        ]
        # Curried abstractions can't be expressed in op-children (no-apps),
        # so restrict stitch to match. lambda-calc (apps-equal) represents
        # them natively, so leave stitch unconstrained there.
        if weighting == "no-apps":
            cmd += ["--no-curried-bodies", "--no-curried-metavars"]
        start = time.time()
        _subproc_run(cmd)
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

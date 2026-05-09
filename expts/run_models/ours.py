"""Wrappers around our egg-stitch compressor binary.

Two callable dataclasses (:class:`OursBf`, :class:`OursSmc`) carry their own
hyperparameters as fields; the runner instantiates them with domain-scaled
budgets via :meth:`scaled_for_domain` rather than mutating module-level
state. :func:`egg_stitch` is a low-level escape hatch for ad-hoc dev runs.
"""

import json
import os
import subprocess
from dataclasses import dataclass, replace
from functools import cache
from pathlib import Path
from typing import ClassVar

from .._build import cargo_build
from ..bench import Abstraction, BenchResult, MAX_ARITY, Weighting
from ..folders import current_folder_path, unique_path


# Project root for the egg-stitch (this repo) compressor. We're already on
# this tree so there's no clean-main check — that's the user's working copy
# by definition.
EGG_STITCH_DIR: Path = Path(__file__).resolve().parent.parent.parent


@cache
def egg_stitch_bin() -> Path:
    """Build (if needed) and return the path to the egg-stitch binary.

    Lazy + cached so importing this module is cheap; cargo only runs the
    first time someone actually wants to invoke the tool.
    """
    return cargo_build(EGG_STITCH_DIR, "egg-stitch")


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
        cmd = ["samply", "record", str(egg_stitch_bin()), *prog_args]
    else:
        cmd = [str(egg_stitch_bin()), *prog_args]
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
         weighting: Weighting, search: str, max_arity: int, rebuild_egraph: bool,
         search_flags: dict[str, object]) -> BenchResult:
    """Shared subprocess body for the SMC/best-first runners.

    ``search_flags`` carries only the runner-specific dials (num_steps,
    particles, temperature, …); the rest is identical between the two
    search modes.
    """
    output_path = unique_path(
        current_folder_path() / f"{input_path.stem}_{search.replace('-', '_')}.json"
    )
    language = "op-children" if weighting == "no-apps" else "lambda-calc"
    cmd: list[str] = [
        str(egg_stitch_bin()),
        "-i", str(input_path),
        "--output", str(output_path),
        "--search", search,
        "--language", language,
        "--max-arity", str(max_arity),
        "--num-abstractions", str(rounds),
        # cogsci/no-apps tables suppress 0-arity abstractions to match how
        # babble/stitch are invoked; lambda-calc runs use the same setting
        # since the table comparison is symmetric.
        "--no-zero-arity",
    ]
    if rebuild_egraph:
        cmd.append("--rebuild-egraph")
    if rewrites_path is not None:
        cmd += ["-r", rewrites_path]
    for k, v in search_flags.items():
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


@dataclass(frozen=True)
class OursBf:
    """Egg-stitch in best-first ("enum") search mode, on a single input file."""

    method: ClassVar[str] = "enum"
    is_ours: ClassVar[bool] = True

    num_steps: int = 500
    rebuild_egraph: bool = False
    max_arity: int = MAX_ARITY

    def scaled_for_domain(self, domain: str) -> "OursBf":
        """Return a copy with ``num_steps`` reduced for multi-file domains.

        The runner calls this once per (domain, runner) so dreamcoder runs
        — which fan out to N independent per-file invocations — don't
        over-spend the search budget vs. the single-file cogsci runs.
        """
        from ..runner import scale_budget_for_domain
        return replace(self, num_steps=scale_budget_for_domain(domain, self.num_steps))

    def __call__(self, rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
        return _run(
            rounds=rounds, input_path=input_path, rewrites_path=rewrites_path,
            weighting=weighting, search="best-first",
            max_arity=self.max_arity, rebuild_egraph=self.rebuild_egraph,
            search_flags={"num_steps": self.num_steps},
        )


@dataclass(frozen=True)
class OursSmc:
    """Egg-stitch in SMC search mode, on a single input file."""

    method: ClassVar[str] = "smc"
    is_ours: ClassVar[bool] = True

    num_steps: int = 100
    num_particles: int = 1000
    temperature: float = 1000.0
    rebuild_egraph: bool = False
    max_arity: int = MAX_ARITY

    def scaled_for_domain(self, domain: str) -> "OursSmc":
        """Return a copy with ``num_particles`` reduced for multi-file domains.

        SMC's compute scales linearly in particle count, so that's the dial
        we shrink to keep total work comparable across cogsci/dreamcoder.
        """
        from ..runner import scale_budget_for_domain
        return replace(self, num_particles=scale_budget_for_domain(domain, self.num_particles))

    def __call__(self, rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
        return _run(
            rounds=rounds, input_path=input_path, rewrites_path=rewrites_path,
            weighting=weighting, search="smc",
            max_arity=self.max_arity, rebuild_egraph=self.rebuild_egraph,
            search_flags={
                "num_steps": self.num_steps,
                "num_particles": self.num_particles,
                "temperature": self.temperature,
            },
        )

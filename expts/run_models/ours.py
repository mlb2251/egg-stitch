"""Wrappers around our egg-stitch compressor binary.

Two callable dataclasses (:class:`OursBf`, :class:`OursSmc`) carry their own
hyperparameters as fields rather than mutating module-level state.
:func:`egg_stitch` is a low-level escape hatch for ad-hoc dev runs.
"""

import json
import math
import os
from dataclasses import dataclass
from functools import cache
from pathlib import Path

from .._build import cargo_build
from .._subproc import run as _subproc_run
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


def egg_stitch(input, output="out.json", rewrites=None, **kwargs) -> Path:
    """Low-level escape hatch: run the egg-stitch binary with arbitrary CLI flags.

    Used by ``run.py`` for ad-hoc dev experiments where the table-runner API
    is too coarse. ``output`` is interpreted relative to the current results
    folder. All other kwargs are forwarded as ``--key value`` (or ``--key``
    for ``True`` booleans).
    """
    output_path = unique_path(current_folder_path() / output)
    cmd = [str(egg_stitch_bin()), "-i", input, "--output", str(output_path)]
    if rewrites is not None:
        cmd += ["-r", rewrites]
    for k, v in kwargs.items():
        flag = "--" + k.replace("_", "-")
        if isinstance(v, bool):
            if v:
                cmd.append(flag)
            continue
        cmd += [flag, str(v)]
    _subproc_run(cmd, env=dict(os.environ, RUST_BACKTRACE="1"))
    return output_path


def _run(*, rounds: int, input_path: Path, rewrites_path: str | None,
         weighting: Weighting, search: str, max_arity: int,
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
    ]
    # 0-arity (constant) abstractions are allowed: stitch finds them by
    # default, babble's dreamcoder ``benchmark`` binary hardcodes
    # ``learn_constants=true``, and our cogsci ``Babble`` wrapper passes
    # ``--learn-constants`` to drawings. Forbidding them only here would
    # handicap the comparison, so we don't pass ``--no-zero-arity``.
    if rewrites_path is not None:
        cmd += ["-r", rewrites_path]
    for k, v in search_flags.items():
        flag = "--" + k.replace("_", "-")
        if isinstance(v, bool):
            if v:
                cmd.append(flag)
        else:
            cmd += [flag, str(v)]
    _subproc_run(cmd, cwd=EGG_STITCH_DIR, env=dict(os.environ, RUST_BACKTRACE="1"))
    with open(output_path) as f:
        data = json.load(f)
    # egg-stitch's RunResult serialises ``pattern`` as ``"<fn_name>: <body>"``
    # (see src/lib.rs ~ line 208); split it back into the BenchResult shape.
    abstractions: list[Abstraction] = []
    for i, a in enumerate(data.get("library", [])):
        s = a["pattern"]
        name, _, body = s.partition(": ")
        abstractions.append(Abstraction(name=name or f"fn_{i}", body=body or s))
    return BenchResult(
        elapsed_secs=float(data["elapsed_secs"]),
        initial_corpus=list(data["original_programs"]),
        final_corpus=list(data["rewritten_programs"]),
        abstractions=abstractions,
        # Only meaningful when DSRs were applied; leave NaN otherwise so it
        # propagates through the cross-file sum in the runner.
        cost_after_rewrites=float(data["cost_after_rewrites"]) if rewrites_path is not None else math.nan,
    )


@dataclass(frozen=True)
class OursBf:
    """Egg-stitch in best-first ("enum") search mode, on a single input file."""

    num_steps: int = 500
    max_arity: int = MAX_ARITY

    def __call__(self, rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
        return _run(
            rounds=rounds, input_path=input_path, rewrites_path=rewrites_path,
            weighting=weighting, search="best-first",
            max_arity=self.max_arity,
            search_flags={"num_steps": self.num_steps},
        )


@dataclass(frozen=True)
class OursSmc:
    """Egg-stitch in SMC search mode, on a single input file."""

    num_steps: int = 100
    num_particles: int = 1000
    temperature: float = 1000.0
    max_arity: int = MAX_ARITY

    def __call__(self, rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
        return _run(
            rounds=rounds, input_path=input_path, rewrites_path=rewrites_path,
            weighting=weighting, search="smc",
            max_arity=self.max_arity,
            search_flags={
                "num_steps": self.num_steps,
                "num_particles": self.num_particles,
                "temperature": self.temperature,
            },
        )

"""Wrappers around our egg-stitch compressor binary.

Two callable dataclasses (:class:`OursBf`, :class:`OursSmc`) carry their own
hyperparameters as fields rather than mutating module-level state.
:func:`egg_stitch` is a low-level escape hatch for ad-hoc dev runs.
"""

import json
import math
import os
import time
from dataclasses import dataclass, field
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
         search_flags: dict[str, object],
         only_use_dsrs_at_start: bool = False,
         iter_limit: int | None = None,
         timeout: float | None = None,
         mem_limit: int | None = None) -> BenchResult:
    """Shared subprocess body for the SMC/best-first runners.

    ``search_flags`` carries only the runner-specific dials (num_steps,
    particles, temperature, …); the rest is identical between the two
    search modes. ``only_use_dsrs_at_start`` switches DSRs from live-during-
    search to a one-shot canonicalisation pass; ``iter_limit`` caps e-saturation
    iterations (None = the binary default, 100); ``timeout`` caps wall-clock;
    ``mem_limit`` caps address space.
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
    if iter_limit is not None:
        cmd += ["--iter-limit", str(iter_limit)]
    # 0-arity (constant) abstractions are allowed: stitch finds them by
    # default, babble's dreamcoder ``benchmark`` binary hardcodes
    # ``learn_constants=true``, and our cogsci ``Babble`` wrapper passes
    # ``--learn-constants`` to drawings. Forbidding them only here would
    # handicap the comparison, so we don't pass ``--no-zero-arity``.
    if rewrites_path is not None:
        cmd += ["-r", rewrites_path]
    if only_use_dsrs_at_start:
        cmd.append("--only-use-dsrs-at-start")
    for k, v in search_flags.items():
        flag = "--" + k.replace("_", "-")
        if isinstance(v, bool):
            if v:
                cmd.append(flag)
        else:
            cmd += [flag, str(v)]
    start = time.time()
    _subproc_run(cmd, cwd=EGG_STITCH_DIR, env=dict(os.environ, RUST_BACKTRACE="1"),
                 timeout=timeout, mem_limit=mem_limit)
    elapsed = time.time() - start
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
        # Wall-clock from the wrapper (incl. process spawn + corpus read + JSON
        # write), not egg-stitch's internal ``elapsed_secs``, so the time is
        # comparable to babble (which also reports wrapper wall-clock).
        elapsed_secs=elapsed,
        initial_corpus=list(data["original_programs"]),
        final_corpus=list(data["rewritten_programs"]),
        abstractions=abstractions,
        # Only meaningful when DSRs were applied; leave NaN otherwise so it
        # propagates through the cross-file sum in the runner.
        cost_after_rewrites=float(data["cost_after_rewrites"]) if rewrites_path is not None else math.nan,
        cost_at_end_of_each_iter=data.get("cost_at_end_of_each_iter"),
    )


@dataclass(frozen=True)
class OursBf:
    """Egg-stitch in best-first ("enum") search mode, on a single input file.

    Best-first needs at least one cutoff. ``num_steps`` is the expansion-count
    budget; ``max_forced_expansion`` is the self-draining forced-expansion cap
    (the heap bounds to the ``{forced ≤ cap}`` subspace and empties, so the
    search runs to *convergence* instead of being clipped). Set either or both
    (whichever is not None is forwarded as a flag).
    """

    num_steps: int | None = 500
    max_forced_expansion: int | None = None
    max_arity: int = MAX_ARITY
    # only_use_dsrs_at_start = canonicalise once up front (dsrs-only-at-start
    # baseline); no_dsrs = drop DSRs entirely (no-rules baseline). repr=False keeps
    # the method label unchanged.
    only_use_dsrs_at_start: bool = field(default=False, repr=False)
    no_dsrs: bool = field(default=False, repr=False)
    iter_limit: int | None = field(default=None, repr=False)
    timeout: float | None = field(default=None, repr=False)
    mem_limit: int | None = field(default=None, repr=False)

    def __call__(self, rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
        search_flags: dict[str, object] = {}
        if self.num_steps is not None:
            search_flags["num_steps"] = self.num_steps
        if self.max_forced_expansion is not None:
            search_flags["max_forced_expansion"] = self.max_forced_expansion
        return _run(
            rounds=rounds, input_path=input_path,
            rewrites_path=None if self.no_dsrs else rewrites_path,
            weighting=weighting, search="best-first",
            max_arity=self.max_arity,
            search_flags=search_flags,
            only_use_dsrs_at_start=self.only_use_dsrs_at_start,
            iter_limit=self.iter_limit,
            timeout=self.timeout, mem_limit=self.mem_limit,
        )


@dataclass(frozen=True)
class OursSmc:
    """Egg-stitch in SMC search mode, on a single input file."""

    num_steps: int = 100
    num_particles: int = 1000
    temperature: float = 1000.0
    max_arity: int = MAX_ARITY
    iter_limit: int | None = field(default=None, repr=False)
    timeout: float | None = field(default=None, repr=False)
    mem_limit: int | None = field(default=None, repr=False)

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
            iter_limit=self.iter_limit,
            timeout=self.timeout, mem_limit=self.mem_limit,
        )

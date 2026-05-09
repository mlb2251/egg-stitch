"""Per-file wrappers around the four compression tools.

Each ``run_*`` function takes the same ``(rounds, input_path, rewrites_path,
weighting)`` signature, runs the tool's subprocess on a single corpus file,
parses its JSON dump, and returns a :class:`BenchResult`. Domain dispatch,
multi-file aggregation, cost recomputation, and geomean across runs all live
one layer up in :mod:`expts.runner`.

The ``weighting`` argument selects the corpus shape and the tool flags that
score it consistently across all four wrappers:

- ``"no-apps"`` — flat cogsci-style s-expressions (operators take all children
  directly, no curried application nodes). Maps to egg-stitch
  ``--language op-children`` and to babble's ``drawings`` binary.
- ``"apps-equal"`` — curried dreamcoder-style s-expressions where every
  application is a binary ``App`` node. Maps to egg-stitch
  ``--language lambda-calc`` and to babble's ``benchmark`` binary.

Hyperparameters that aren't part of the signature live as module-level
constants below; one-off overrides happen by patching the constant.
"""

import csv
import json
import os
import subprocess
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Literal

from . import (
    BABBLE_BENCH_BIN,
    BABBLE_BIN,
    BABBLE_DIR,
    EGG_STITCH_BIN,
    EGG_STITCH_DIR,
    STITCH_BIN,
)
from .folders import current_folder_path, unique_path

Weighting = Literal["no-apps", "apps-equal"]


@dataclass
class Abstraction:
    """A single learned abstraction, in the tool's native s-expression form."""

    name: str
    body: str


@dataclass
class BenchResult:
    """Per-file output of one tool invocation, in a tool-agnostic shape.

    ``initial_corpus`` and ``final_corpus`` are the program strings before and
    after the tool's rewrite/compression step. The runner recomputes costs from
    these uniformly via :func:`expts.runner.ast_size`, so the four tools
    contribute apples-to-apples numbers regardless of their internal cost
    metrics.
    """

    elapsed_secs: float
    initial_corpus: list[str]
    final_corpus: list[str]
    abstractions: list[Abstraction]
    # Optional: the minimum AST size reachable in the e-graph after DSR
    # rewrites are applied, before any abstraction is found. Only egg-stitch
    # reports this (it falls out of the e-graph extraction it does anyway);
    # other tools leave it ``None``.
    cost_after_rewrites: int | None = None


# ─── Hyperparameters ───────────────────────────────────────────────────────
# Patch these at module level for one-off overrides; otherwise treat as fixed.

MAX_ARITY = 2

# egg-stitch SMC
SMC_NUM_STEPS = 100
SMC_NUM_PARTICLES = 1000
SMC_TEMPERATURE = 1000.0

# egg-stitch best-first
BF_NUM_STEPS = 500

# Pass ``--rebuild-egraph`` to egg-stitch. Required when stacking many
# abstractions in one run (Tables 3/4) so the e-graph stays consistent
# after each successive abstraction is applied; off for single-abstraction
# runs since rebuilding is wasted work then.
OURS_REBUILD_EGRAPH = False

# babble beam search
BABBLE_BEAMS = 400
BABBLE_LPS = 1


# ─── Path inference for babble apps-equal ───────────────────────────────────
# babble's ``benchmark`` binary loads its DSRs by domain name from a fixed
# location; the wrapper recovers the domain from the input file's parent
# directory so it can pass ``--domain``.
DREAMCODER_DOMAIN_PATHS: dict[Path, str] = {
    Path("data/domains/list"):    "list",
    Path("data/domains/physics"): "physics",
    Path("data/domains/text"):    "text",
    Path("data/domains/logo"):    "logo",
    Path("data/domains/towers"):  "towers",
}


# ─── egg-stitch (ours) ──────────────────────────────────────────────────────


def egg_stitch(input, output="out.json", rewrites=None, flamegraph=False, samply=False, **kwargs) -> Path:
    """Low-level escape hatch: run the egg-stitch binary with arbitrary CLI flags.

    Used by ``run.py`` for ad-hoc dev experiments where the table-runner API is
    too coarse. ``output`` is interpreted relative to the current results
    folder. ``flamegraph=True`` profiles via ``cargo flamegraph`` (macOS, needs
    sudo); ``samply=True`` profiles via ``samply record``. All other kwargs
    are forwarded as ``--key value`` (or ``--key`` for ``True`` booleans).
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


def _run_ours(
    *,
    rounds: int,
    input_path: Path,
    rewrites_path: str | None,
    weighting: Weighting,
    search: str,
    extra_flags: dict[str, object],
) -> BenchResult:
    """Shared body for SMC/best-first wrappers; only the search-kind-specific
    flags differ between them."""
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
    if OURS_REBUILD_EGRAPH:
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
    return _run_ours(
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
    return _run_ours(
        rounds=rounds, input_path=input_path, rewrites_path=rewrites_path,
        weighting=weighting, search="best-first",
        extra_flags={"num_steps": BF_NUM_STEPS},
    )


# ─── stitch ─────────────────────────────────────────────────────────────────


def run_stitch(rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
    """Run stitch on a single ``input_path``.

    stitch doesn't accept DSRs, so ``rewrites_path`` is ignored (asserted unset).
    Cost flags are picked so stitch's internal scoring lines up with the runner's
    uniform :func:`ast_size`: at ``no-apps`` weighting all non-app costs are
    huge so App=1 is negligible; at ``apps-equal`` they're all 1.
    """
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


# ─── babble ─────────────────────────────────────────────────────────────────


def run_babble(rounds: int, input_path: Path, rewrites_path: str | None, weighting: Weighting) -> BenchResult:
    """Run babble on a single ``input_path``.

    Dispatches between the two babble binaries based on ``weighting``: the
    ``drawings`` binary for cogsci-style flat s-expressions, ``benchmark``
    for curried dreamcoder programs. Both expose ``--dump-json`` for a uniform
    output format.
    """
    if weighting == "no-apps":
        return _run_babble_drawings(rounds, input_path, rewrites_path)
    assert weighting == "apps-equal"
    return _run_babble_benchmark(rounds, input_path, rewrites_path)


def _run_babble_drawings(rounds: int, input_path: Path, rewrites_path: str | None) -> BenchResult:
    """Run babble's ``drawings`` binary on the ``.bab`` file matching ``input_path``.

    The cogsci JSON corpus and babble's ``.bab`` text format hold the same
    s-expressions, so we map ``data/domains/cogsci/<stem>.json`` →
    ``<BABBLE_DIR>/harness/data/cogsci/<stem>.bab``.
    """
    bab = BABBLE_DIR / "harness" / "data" / "cogsci" / f"{input_path.stem}.bab"
    json_dump = unique_path(current_folder_path() / f"{input_path.stem}_babble.json")
    csv_out = unique_path(current_folder_path() / f"{input_path.stem}_babble.csv")
    cmd = [
        str(BABBLE_BIN),
        str(bab),
        f"--beams={BABBLE_BEAMS}",
        f"--lps={BABBLE_LPS}",
        f"--rounds={rounds}",
        f"--max-arity={MAX_ARITY}",
        f"--output={csv_out}",
        f"--dump-json={json_dump}",
    ]
    if rewrites_path is not None:
        cmd += [f"--dsr={rewrites_path}"]
    print("+", " ".join(cmd), flush=True)
    start = time.time()
    subprocess.run(cmd, check=True, cwd=BABBLE_DIR)
    elapsed = time.time() - start
    with open(json_dump) as f:
        data = json.load(f)
    return _babble_result_from_dump(data, elapsed)


def _run_babble_benchmark(rounds: int, input_path: Path, rewrites_path: str | None) -> BenchResult:
    """Run babble's ``benchmark`` binary in single-file mode (``--input-file``).

    The binary auto-loads its DSRs from ``<DSR_PATH>/<domain>.rewrites``; the
    caller must pass that exact path (or ``None`` for ``--mode au``), since
    babble has no flag for an arbitrary DSR location.
    """
    domain = DREAMCODER_DOMAIN_PATHS.get(input_path.parent.relative_to(EGG_STITCH_DIR))
    if domain is None:
        # Allow absolute paths too: try the absolute parent.
        domain = DREAMCODER_DOMAIN_PATHS.get(input_path.parent)
    assert domain is not None, (
        f"can't resolve dreamcoder domain for {input_path}; "
        f"add its parent to DREAMCODER_DOMAIN_PATHS"
    )
    if rewrites_path is not None:
        from .runner import rewrites_path as _expected_rewrites
        expected = _expected_rewrites(domain)
        assert rewrites_path == expected, (
            f"babble auto-loads its own DSRs for {domain!r}; passed "
            f"rewrites_path={rewrites_path!r} but only {expected!r} would be used"
        )
    mode = "babble" if rewrites_path is not None else "au"
    json_dump = unique_path(current_folder_path() / f"{input_path.stem}_babble.json")
    csv_out = unique_path(current_folder_path() / f"{input_path.stem}_babble.csv")
    cmd = [
        str(BABBLE_BENCH_BIN),
        "--domain", domain,
        "--input-file", str(input_path),
        "--output", str(csv_out),
        "--dump-json", str(json_dump),
        "--beam-size", str(BABBLE_BEAMS),
        "--lps", str(BABBLE_LPS),
        "--rounds", str(rounds),
        "--max-arity", str(MAX_ARITY),
        "--lib-iter-limit", "1",
        "--use-all", "0",
        "--mode", mode,
    ]
    print("+", " ".join(cmd), flush=True)
    start = time.time()
    subprocess.run(cmd, check=True, cwd=BABBLE_DIR)
    elapsed = time.time() - start
    with open(json_dump) as f:
        data = json.load(f)
    # benchmark dump nests under "files"; in --input-file mode there's exactly one.
    files = data["files"]
    assert len(files) == 1, f"expected 1 file in babble dump, got {len(files)}"
    return _babble_result_from_dump(files[0], elapsed)


def _babble_result_from_dump(data: dict, elapsed: float) -> BenchResult:
    """Common BenchResult constructor for both babble dump shapes.

    Both dumps expose ``original``, ``rewritten``, and ``abstractions=[{id,body}]``.
    Wall-clock time is taken from the wrapper's own ``time.time()`` rather than
    babble's reported value so it's comparable to the other tools.
    """
    abstractions = [
        Abstraction(name=f"fn_{a['id']}", body=a["body"])
        for a in data.get("abstractions", [])
    ]
    return BenchResult(
        elapsed_secs=elapsed,
        initial_corpus=list(data["original"]),
        final_corpus=list(data["rewritten"]),
        abstractions=abstractions,
    )

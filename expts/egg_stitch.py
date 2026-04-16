"""Wrappers around our egg-stitch compressor binary."""

import json
import os
import subprocess

from . import EGG_STITCH_BIN, rewrites_path
from .folders import current_folder_path, unique_path
from .result import Result, ratio


def egg_stitch(input, output="out.json", rewrites=None, flamegraph=False, samply=False, **kwargs):
    """Run `cargo run --release` with the given input/output (relative to results dir)/optional rewrites.

    Pass ``flamegraph=True`` to profile via ``cargo flamegraph`` (macOS, needs sudo).
    Pass ``samply=True`` to profile via ``samply record`` (builds release binary first,
    then opens Firefox Profiler automatically).
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
        k = k.replace("_", "-")
        if isinstance(v, bool):
            if v:
                cmd.append(f"--{k}")
            continue
        cmd += [f"--{k}", str(v)]
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, check=True, env=dict(os.environ, RUST_BACKTRACE="1"))
    return output_path


_UNSET = object()


def run_ours(domain: str, search: str, *, num_steps: int, max_arity: int, rewrites=_UNSET, **extra) -> tuple[Result, int | None]:
    """Run our compressor on ``domain``.

    ``rewrites`` defaults to the domain's standard rewrite file; pass
    ``None`` to run without any DSRs.

    Returns ``(Result, egraph_min_size)`` where ``egraph_min_size`` is the
    corpus min cost after the rewrite rules are applied (our tool exposes
    this as ``cost_after_rewrites``). When no rewrites are applied the
    second tuple element is ``None``.
    """
    if rewrites is _UNSET:
        rewrites = rewrites_path(domain)
    output = egg_stitch(
        f"data/domains/cogsci/{domain}.json",
        rewrites=rewrites,
        output=f"{domain}_{search.replace('-', '_')}.json",
        search=search,
        num_steps=num_steps,
        max_arity=max_arity,
        **extra,
    )
    with open(output) as f:
        data = json.load(f)
    initial_cost = int(data["initial_cost"])
    final_cost = int(data.get("final_cost") or initial_cost)
    abstractions = data.get("library") or []
    method = "enum" if search == "best-first" else search
    size_abstractions = sum(a["pattern_size"] for a in abstractions)
    cost_after = int(data["cost_after_rewrites"]) + size_abstractions if rewrites is not None else None
    result = Result(
        method=method,
        domain=domain,
        initial_cost=initial_cost,
        final_cost=final_cost,
        compression_ratio=ratio(initial_cost, final_cost),
        elapsed_secs=float(data["elapsed_secs"]),
        library=[a["pattern"] for a in abstractions],
        extra={
            "cost_after_rewrites": cost_after,
            "abstractions": abstractions,
            "output_file": str(output),
        },
    )
    return result, cost_after

"""Wrappers around the external stitch compressor."""

import json
import subprocess as sp
import time

from . import STITCH_BIN, STITCH_DIR
from .result import Result, ratio

from s_expression_parser import parse, ParserConfig, Pair, nil

# make sure stitch's cost multiplier is large enough that our AstSize metric
# isn't that far off it's metric, which counts App as 1 (we count it as 0)
COST_MULTIPLIER = str(10_000)


def recursive_pair_size(p: type[nil] | Pair | str) -> int:
    if isinstance(p, str):
        return 1
    result = 0
    while p is not nil:
        result += recursive_pair_size(p.car)
        p = p.cdr
    return result


def ast_size(programs: list[str]) -> int:
    total = 0
    for prog in programs:
        [p] = parse(prog, ParserConfig(prefix_symbols={}, dots_are_cons=False))
        total += recursive_pair_size(p)
    return total


def run_stitch(domain: str, *, num_abstractions: int = 1, max_arity: int) -> Result:
    """Run stitch on ``domain``.

    ``num_abstractions`` maps to stitch's ``-i`` (iterations) flag, so it
    controls how many abstractions stitch is asked to learn.
    """
    outfile = f"out/for-egg-stitch/{domain}.json"
    print(f"\033[92mRunning stitch on {domain}\033[0m", flush=True)
    cmd = [
        str(STITCH_BIN),
        f"data/cogsci/{domain}.json",
        f"-i{num_abstractions}",
        "-a2",
        "--out",
        outfile,
        "--no-curried-bodies",
        "--no-curried-metavars",
        "--silent",
        "--allow-single-task",
        "--cost-var",
        COST_MULTIPLIER,
        "--cost-ivar",
        COST_MULTIPLIER,
        "--cost-prim-default",
        COST_MULTIPLIER,
        "--cost-lam",
        COST_MULTIPLIER,
    ]
    start = time.time()
    sp.run(cmd, check=True, cwd=STITCH_DIR)
    wall_secs = time.time() - start
    with open(STITCH_DIR / outfile) as f:
        data = json.load(f)
    initial_cost = ast_size(data["original"])
    final_cost = ast_size(data["rewritten"]) + ast_size(data.get("library", []))
    library = [
        f"{a.get('name', f'fn_{i}')}: {a['body']}"
        for i, a in enumerate(data.get("abstractions", []))
    ]
    return Result(
        method="stitch",
        domain=domain,
        initial_cost=initial_cost,
        final_cost=final_cost,
        compression_ratio=ratio(initial_cost, final_cost),
        elapsed_secs=wall_secs,
        library=library,
        extra={
            "stitch_reported_compression": float(data.get("compression_ratio", 0.0)),
            "num_abstractions": int(data.get("num_abstractions", len(library))),
        },
    )

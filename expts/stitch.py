"""Wrappers around the external stitch compressor."""

import json
import subprocess as sp
import time

from . import STITCH_BIN, STITCH_DIR, dreamcoder_files, domain_type
from .result import Result, aggregate_per_file, ratio

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

    For cogsci domains the corpus is the single ``data/cogsci/<domain>.json``
    file. For dreamcoder domains stitch is invoked once per file under
    ``data/domains/<domain>/`` (passed in via an absolute path) and the
    per-file Results are combined via :func:`aggregate_per_file`.

    ``num_abstractions`` maps to stitch's ``-i`` (iterations) flag.
    """
    if domain_type(domain) == "dreamcoder":
        return _run_stitch_dreamcoder(domain, num_abstractions=num_abstractions, max_arity=max_arity)
    assert domain_type(domain) == "cogsci"
    return _run_stitch_single(
        domain,
        f"data/cogsci/{domain}.json",
        f"out/for-egg-stitch/{domain}.json",
        num_abstractions=num_abstractions,
        max_arity=max_arity,
        cost=COST_MULTIPLIER,
    )


def _run_stitch_single(domain: str, input_path: str, outfile: str, *, num_abstractions: int, max_arity: int, cost: str) -> Result:
    """Run the stitch binary on a single corpus file and parse its JSON output.

    All non-app costs are set to ``cost``; ``--cost-app`` is left at stitch's
    default of 1. For cogsci ``cost`` is large (``COST_MULTIPLIER``) so the
    fixed App=1 is negligible vs our AstSize (which counts App=0). For
    dreamcoder ``cost=1``, matching babble's all-nodes-cost-1 metric.
    """
    print(f"\033[92mRunning stitch on {domain} ({input_path})\033[0m", flush=True)
    cmd = [
        str(STITCH_BIN),
        input_path,
        f"-i{num_abstractions}",
        f"-a{max_arity}",
        "--out",
        outfile,
        "--no-curried-bodies",
        "--no-curried-metavars",
        "--silent",
        "--allow-single-task",
        "--cost-var",
        cost,
        "--cost-ivar",
        cost,
        "--cost-prim-default",
        cost,
        "--cost-lam",
        cost,
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


def _run_stitch_dreamcoder(domain: str, *, num_abstractions: int, max_arity: int) -> Result:
    """Iterate stitch over every file in ``data/domains/<domain>/`` and aggregate.

    Dreamcoder runs use cost 1 for app/lam/var/ivar/prim to match babble's
    ``Expr::len``-based cost (every AST node = 1) and our egg-stitch defaults
    (``Weights {sym_var:1, app:1, lam:1}``), so all three tools score the same
    AST identically.
    """
    per_file: list[Result] = []
    for f in dreamcoder_files(domain):
        # stitch runs in STITCH_DIR; pass an absolute input path so it doesn't
        # need a copy of our data tree.
        result = _run_stitch_single(
            domain,
            str(f),
            f"out/for-egg-stitch/{domain}__{f.stem}.json",
            num_abstractions=num_abstractions,
            max_arity=max_arity,
            cost="1",
        )
        per_file.append(result)
    return aggregate_per_file(per_file)

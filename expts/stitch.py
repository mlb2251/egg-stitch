"""Wrappers around the external stitch compressor."""

import json
import subprocess as sp
import time

from . import STITCH_BIN, STITCH_DIR
from .result import Result, ratio
from .stackpath import save_run


def run_stitch(domain: str) -> Result:
    outfile = f"out/for-egg-stitch/{domain}.json"
    print(f"\033[92mRunning stitch on {domain}\033[0m", flush=True)
    cmd = [
        str(STITCH_BIN),
        f"data/cogsci/{domain}.json",
        "-i1", "-a2", "--out", outfile,
        "--no-curried-bodies", "--no-curried-metavars", "--silent",
    ]
    start = time.time()
    sp.run(cmd, check=True, cwd=STITCH_DIR)
    wall_secs = time.time() - start
    with open(STITCH_DIR / outfile) as f:
        data = json.load(f)
    initial_cost = int(data["original_cost"])
    final_cost = int(data["final_cost"])
    library = [
        f"{a.get('name', f'fn_{i}')}: {a['body']}"
        for i, a in enumerate(data.get("abstractions", []))
    ]
    result = Result(
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
    config = {"domain": domain}
    save_run(result.to_dict(), config, "stitch")
    return result

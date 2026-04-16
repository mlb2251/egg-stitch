"""Wrappers around the external babble compressor."""

import subprocess as sp
import time

from . import BABBLE_BIN, BABBLE_DIR
from .result import Result, ratio


def run_babble(domain: str, *, dsr: str | None = None, num_abstractions: int = 1) -> Result:
    """Run babble on a cogsci ``domain`` and return a :class:`Result`.

    When ``dsr`` is provided, it is passed to babble as ``--dsr`` so the
    domain-specific rewrites get applied during library learning (this is
    what makes the comparison apples-to-apples with our rewrite-on runs).

    ``num_abstractions`` controls how many libraries babble is asked to
    learn serially; it maps to babble's ``--rounds`` parameter.
    """
    outfile = f"harness/data_gen/cache/{domain}.csv"
    print(f"\033[92mRunning babble on {domain}{' (with DSRs)' if dsr else ''}\033[0m", flush=True)
    cmd = [
        str(BABBLE_BIN),
        f"harness/data/cogsci/{domain}.bab",
        "--beams=400", "--lps=1", f"--rounds={num_abstractions}", "--max-arity=2",
        f"--output={outfile}",
    ]
    if dsr is not None:
        cmd += [f"--dsr={dsr}"]
    start = time.time()
    proc = sp.run(cmd, check=True, cwd=BABBLE_DIR, capture_output=True, text=True)
    wall_secs = time.time() - start
    with open(BABBLE_DIR / outfile) as f:
        # With ``--rounds=N`` babble writes one CSV row per round; the last
        # row holds the cumulative post-final-round numbers, which is what
        # we report.
        row = f.read().strip().splitlines()[-1].split(",")
    # CSV fields: type,round,beams_start,beams_end,lps,?,rounds,initial_cost,final_cost,compression,num_libs,time
    initial_cost, final_cost = int(row[7]), int(row[8])
    # Parse "lib <name> =\n  <body>\nin" pairs out of babble's stdout.
    libs: list[str] = []
    lines = proc.stdout.splitlines()
    for i, l in enumerate(lines):
        if l.startswith("lib "):
            name = l.strip().removesuffix(" =")
            body = lines[i + 1].strip() if i + 1 < len(lines) else "?"
            libs.append(f"{name}: {body}")
    return Result(
        method="babble",
        domain=domain,
        initial_cost=initial_cost,
        final_cost=final_cost,
        compression_ratio=ratio(initial_cost, final_cost),
        elapsed_secs=wall_secs,
        library=libs,
        extra={
            "babble_reported_secs": float(row[11]),
            "babble_reported_compression": float(row[9]),
            "dsr": dsr,
        },
    )
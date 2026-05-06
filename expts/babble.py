"""Wrappers around the external babble compressor."""

import csv
import math
import subprocess as sp
import time

from . import BABBLE_BENCH_BIN, BABBLE_BIN, BABBLE_DIR, domain_type
from .result import Result, ratio


def run_babble(domain: str, *, dsr: str | None = None, num_abstractions: int = 1, max_arity: int) -> Result:
    """Run babble on ``domain`` and return a :class:`Result`.

    Cogsci domains use the ``drawings`` binary on ``harness/data/cogsci/<domain>.bab``;
    dreamcoder domains (list, physics, ...) use the ``benchmark`` binary,
    which iterates over every file in ``harness/data/dreamcoder-benchmarks/benches/``
    matching the domain prefix and applies babble's bundled DSRs automatically.

    ``dsr`` is only meaningful for cogsci domains; dreamcoder runs always use
    babble's own DSRs from ``harness/data/benchmark-dsrs/<domain>.rewrites``
    (passing ``dsr=None`` is how the caller asks for the no-DSR variant, in
    which case we run with ``--mode au``).

    ``num_abstractions`` maps to babble's ``--rounds`` parameter.
    """
    if domain_type(domain) == "dreamcoder":
        return _run_babble_dreamcoder(domain, use_dsrs=dsr is not None, num_abstractions=num_abstractions, max_arity=max_arity)
    assert domain_type(domain) == "cogsci"
    outfile = f"harness/data_gen/cache/{domain}.csv"
    print(f"\033[92mRunning babble on {domain}{' (with DSRs)' if dsr else ''}\033[0m", flush=True)
    cmd = [
        str(BABBLE_BIN),
        f"harness/data/cogsci/{domain}.bab",
        "--beams=400", "--lps=1", f"--rounds={num_abstractions}", f"--max-arity={max_arity}",
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


def _run_babble_dreamcoder(domain: str, *, use_dsrs: bool, num_abstractions: int, max_arity: int) -> Result:
    """Run babble's ``benchmark`` binary over every file in a dreamcoder domain.

    The binary writes a per-file CSV with ``(name, iter, initial cost,
    final cost, compression, total time, num libs)``; we parse it,
    aggregate (sum costs/time, geomean compression ratios) and return a
    single :class:`Result`. With ``use_dsrs=False`` the binary runs in
    ``au`` mode which disables the bundled DSRs; ``use_dsrs=True`` runs
    ``babble`` mode which applies them.
    """
    mode = "babble" if use_dsrs else "au"
    out_csv = f"harness/data_gen/cache/{domain}_dreamcoder_{mode}.csv"
    print(f"\033[92mRunning babble (benchmark/{mode}) on {domain}\033[0m", flush=True)
    cmd = [
        str(BABBLE_BENCH_BIN),
        "--domain", domain,
        "--output", out_csv,
        "--beam-size", "400",
        "--lps", "1",
        "--rounds", str(num_abstractions),
        "--max-arity", str(max_arity),
        "--lib-iter-limit", "1",
        "--use-all", "0",
        "--mode", mode,
    ]
    start = time.time()
    sp.run(cmd, check=True, cwd=BABBLE_DIR)
    wall_secs = time.time() - start
    with open(BABBLE_DIR / out_csv) as f:
        rows = list(csv.DictReader(f))
    if not rows:
        raise RuntimeError(f"babble benchmark produced no rows for {domain}")
    initial_cost = sum(int(r["initial cost"]) for r in rows)
    final_cost = sum(int(r["final cost"]) for r in rows)
    ratios = [float(r["compression"]) for r in rows]
    geo_cr = math.exp(sum(math.log(c) for c in ratios) / len(ratios))
    babble_secs = sum(float(r["total time"]) for r in rows)
    return Result(
        method="babble",
        domain=domain,
        initial_cost=initial_cost,
        final_cost=final_cost,
        compression_ratio=ratio(initial_cost, final_cost),
        elapsed_secs=wall_secs,
        library=None,
        extra={
            "babble_reported_secs": babble_secs,
            "num_files": len(rows),
            "mode": mode,
            "geomean_compression_ratio": geo_cr,
        },
    )
"""Wrappers around the external babble compressor."""

import csv
import math
import subprocess as sp
import time

from . import BABBLE_BENCH_BIN, BABBLE_BIN, BABBLE_DIR, domain_type, rewrites_path
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
        return _run_babble_dreamcoder(domain, dsr=dsr, num_abstractions=num_abstractions, max_arity=max_arity)
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


def _run_babble_dreamcoder(domain: str, *, dsr: str | None, num_abstractions: int, max_arity: int) -> Result:
    """Run babble's ``benchmark`` binary over every file in a dreamcoder domain.

    The binary writes a CSV with columns ``(name, iter, initial cost,
    final cost, compression, total time, num libs)``. NB: the column
    babble labels ``iter`` is actually the input filename (see
    ``babble/src/bin/benchmark/main.rs`` ``plot_raw_data``, which
    serializes ``&file`` into that slot); ``--rounds`` are collapsed
    inside babble before serialization, so there is exactly one row
    per ``(benchmark, file)`` pair. We parse, aggregate (sum
    costs/time, geomean compression ratios) and return a single
    :class:`Result`. ``dsr=None`` runs ``--mode au`` (no DSRs); a
    non-``None`` ``dsr`` runs ``--mode babble`` and must equal the
    path that the benchmark binary auto-loads (it has no flag for a
    custom DSR path), otherwise we'd silently apply the wrong rewrites.
    """
    if dsr is not None:
        expected = rewrites_path(domain)
        assert dsr == expected, (
            f"babble's benchmark binary auto-loads its own DSRs for {domain!r}; "
            f"passed dsr={dsr!r} but only {expected!r} would actually be used"
        )
    mode = "babble" if dsr is not None else "au"
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
        per_file_rows = list(csv.DictReader(f))
    if not per_file_rows:
        raise RuntimeError(f"babble benchmark produced no rows for {domain}")
    initial_cost = sum(int(row["initial cost"]) for row in per_file_rows)
    final_cost = sum(int(row["final cost"]) for row in per_file_rows)
    ratios = [float(row["compression"]) for row in per_file_rows]
    for c in ratios:
        assert 0 < c < math.inf, (
            f"per-file compression={c} from babble on {domain} would make the geomean degenerate"
        )
    geo_cr = math.exp(sum(math.log(c) for c in ratios) / len(ratios))
    babble_secs = sum(float(row["total time"]) for row in per_file_rows)
    # compression_ratio uses the geometric mean of per-file ratios to match
    # how the babble paper (Fig. 12) aggregates dreamcoder benchmarks; the
    # raw per-file CSV rows are preserved in extra for traceability.
    return Result(
        method="babble",
        domain=domain,
        initial_cost=initial_cost,
        final_cost=final_cost,
        compression_ratio=geo_cr,
        elapsed_secs=wall_secs,
        library=None,
        extra={
            "babble_reported_secs": babble_secs,
            "num_files": len(per_file_rows),
            "mode": mode,
            "per_file": per_file_rows,
        },
    )
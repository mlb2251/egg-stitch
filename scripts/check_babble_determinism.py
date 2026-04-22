#!/usr/bin/env python3
"""Run babble multiple times on each cogsci domain and check for determinism.

Invokes the babble ``drawings`` binary with the same flags used by
:mod:`expts.babble` (``--beams=400 --lps=1 --rounds=20 --max-arity=2`` plus
``--dsr=<rewrites>``) N times per domain, then compares the CSV row babble
writes and the ``lib ... = ...`` library definitions parsed out of stdout.
Reports per-domain: all-identical vs a spread summary (min/max
compression, distinct final_cost values, distinct library sets).
"""

import argparse
import subprocess as sp
import sys
import tempfile
from pathlib import Path

EXPT_DIR = Path(__file__).parent.parent / "expts"
sys.path.insert(0, str(EXPT_DIR.parent))
from expts import ALL_DOMAINS, BABBLE_BIN, BABBLE_DIR, rewrites_path


def _one_run(domain: str, rounds: int, tmp_csv: Path) -> dict:
    """Run babble once on ``domain`` and return parsed outputs."""
    cmd = [
        str(BABBLE_BIN),
        f"harness/data/cogsci/{domain}.bab",
        "--beams=400", "--lps=1", f"--rounds={rounds}", "--max-arity=2",
        f"--dsr={rewrites_path(domain).removeprefix('../babble/')}",
        f"--output={tmp_csv}",
    ]
    proc = sp.run(cmd, check=True, cwd=BABBLE_DIR, capture_output=True, text=True)
    row = tmp_csv.read_text().strip().splitlines()[-1].split(",")
    initial_cost, final_cost = int(row[7]), int(row[8])
    compression = float(row[9])
    libs: list[str] = []
    lines = proc.stdout.splitlines()
    for i, l in enumerate(lines):
        if l.startswith("lib "):
            name = l.strip().removesuffix(" =")
            body = lines[i + 1].strip() if i + 1 < len(lines) else "?"
            libs.append(f"{name}: {body}")
    return {
        "initial_cost": initial_cost,
        "final_cost": final_cost,
        "compression": compression,
        "libs": tuple(libs),
    }


def check_domain(domain: str, num_runs: int, rounds: int) -> bool:
    """Run babble ``num_runs`` times on ``domain``; return True iff all match."""
    print(f"\n=== {domain} ({num_runs} runs, rounds={rounds}) ===", flush=True)
    results = []
    with tempfile.TemporaryDirectory() as td:
        for i in range(num_runs):
            out = Path(td) / f"{domain}_{i}.csv"
            r = _one_run(domain, rounds, out)
            print(f"  run {i+1}: final_cost={r['final_cost']} "
                  f"compression={r['compression']:.4f} libs={len(r['libs'])}",
                  flush=True)
            results.append(r)

    final_costs = {r["final_cost"] for r in results}
    lib_sets = {r["libs"] for r in results}
    compressions = [r["compression"] for r in results]

    if len(final_costs) == 1 and len(lib_sets) == 1:
        print(f"  -> DETERMINISTIC: all {num_runs} runs identical "
              f"(final_cost={results[0]['final_cost']}, "
              f"compression={results[0]['compression']:.4f})")
        return True

    print(f"  -> NONDETERMINISTIC:")
    print(f"     distinct final_cost values: {sorted(final_costs)}")
    print(f"     compression min/max: "
          f"{min(compressions):.4f} / {max(compressions):.4f}")
    print(f"     distinct library sets: {len(lib_sets)}")
    return False


def main() -> int:
    """Parse args and check each requested domain; exit nonzero on mismatch."""
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--runs", type=int, default=5,
                    help="runs per domain (default 5)")
    ap.add_argument("--rounds", type=int, default=20,
                    help="babble --rounds value (default 20)")
    ap.add_argument("--domains", nargs="+", default=ALL_DOMAINS,
                    help=f"domains to test (default: {ALL_DOMAINS})")
    args = ap.parse_args()

    all_ok = True
    for d in args.domains:
        all_ok &= check_domain(d, args.runs, args.rounds)

    print()
    print("RESULT:", "deterministic" if all_ok else "NONDETERMINISTIC")
    return 0 if all_ok else 1


if __name__ == "__main__":
    sys.exit(main())

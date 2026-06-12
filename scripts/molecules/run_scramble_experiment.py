#!/usr/bin/env python3
"""Run the DSR-canonicalisation scramble comparison on each corpus family.

For every family under data/domains/molecules/scramble/, runs four conditions
and prints the compression ratio (initial_cost / final_cost) of each:

  canonical + no rules         reference -- what a canonical encoding gives
  scrambled + no rules         random encoding, no recovery
  scrambled + DSR canon        `--only-use-dsrs-at-start`: DSRs saturate the
                               symmetries once, a min-term is extracted per
                               molecule, then the search runs rule-free on that
  scrambled + search-uses-DSRs DSRs kept live during the abstraction search

The takeaway: scrambling destroys the shared-structure alignment a canonical
form provides; the DSRs recover most of it. `search-uses-DSRs` >= `DSR canon`,
and the gap grows with how much large, globally-alignable shared backbone the
family has (small for a functional group, large for a shared chain).

Saturation needs a generous --iter-limit/--node-limit; the binary's defaults
(100 / 50_000_000) are sufficient for these <=14-heavy-atom corpora.

Usage: python3 scripts/run_scramble_experiment.py
"""
import json
import os
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))  # scripts/molecules/.. -> project root
DDIR = os.path.join(ROOT, "data/domains/molecules/scramble")
BIN = os.path.join(ROOT, "target/release/egg-stitch")
FAMILIES = ["hexyl", "ester", "glycol"]
COMMON = ["--search", "best-first", "--num-abstractions", "8", "--num-steps", "80000", "--max-arity", "12"]


def ratio(input_path, extra):
    """Run one condition and return its compression ratio (or None on failure)."""
    out = tempfile.NamedTemporaryFile(suffix=".json", delete=False).name
    subprocess.run([BIN, "--input", input_path, *COMMON, *extra, "--output", out], cwd=ROOT, capture_output=True)
    try:
        d = json.load(open(out))
        return d["initial_cost"] / d["final_cost"]
    except Exception:
        return None
    finally:
        if os.path.exists(out):
            os.unlink(out)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    rules = os.path.join(ROOT, "data/domains/molecules/molecules.rewrites")  # general molecule symmetry DSRs
    print(f"{'family':8s} {'canon':>7s} {'scram':>7s} {'DSR-canon':>10s} {'search-DSR':>11s}  search-DSR edge")
    for fam in FAMILIES:
        scram = os.path.join(DDIR, f"{fam}.scram.json")
        c = ratio(os.path.join(DDIR, f"{fam}.canon.json"), [])
        s = ratio(scram, [])
        dc = ratio(scram, ["-r", rules, "--only-use-dsrs-at-start"])
        sd = ratio(scram, ["-r", rules])
        print(f"{fam:8s} {c:6.2f}x {s:6.2f}x {dc:9.2f}x {sd:10.2f}x   +{sd - dc:.2f}")


if __name__ == "__main__":
    main()

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

ROOT = os.path.dirname(
    os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
)  # scripts/molecules/.. -> project root
DDIR = os.path.join(ROOT, "data/domains/molecules/scramble")
BIN = os.path.join(ROOT, "target/release/egg-stitch")
FAMILIES = ["hexyl", "ester", "glycol"]
COMMON = [
    "--search",
    "best-first",
    "--num-abstractions",
    "8",
    "--num-steps",
    "80000",
    "--max-arity",
    "12",
]


def run(input_path, extra):
    """Run one condition and return its compression ratio (or None on failure)."""
    out = tempfile.NamedTemporaryFile(suffix=".json", delete=False).name
    subprocess.run(
        [BIN, "--input", input_path, *COMMON, *extra, "--output", out],
        cwd=ROOT,
        capture_output=True,
    )
    try:
        d = json.load(open(out))
        initial_cost, at_end_each = d["initial_cost"], d["cost_at_end_of_each_iter"]
        library_short = [x["pattern"] for x in d["library"]]
        times = d["iteration_times"]
        summary_result = dict(
            initial_cost=initial_cost,
            cost_at_end_of_each_iter=at_end_each,
            library=library_short,
            iteration_times=times,
        )
        return summary_result
    finally:
        if os.path.exists(out):
            os.unlink(out)


def compression_ratio(result):
    return result["initial_cost"] / result["cost_at_end_of_each_iter"][-1]


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    rules = os.path.join(
        ROOT, "data/domains/molecules/molecules.rewrites"
    )  # general molecule symmetry DSRs
    print(
        f"{'family':8s} {'canon':>7s} {'scram':>7s} {'DSR-canon':>10s} {'search-DSR':>11s}  search-DSR edge"
    )
    results_by_family = {}
    for fam in FAMILIES:
        cano_path = os.path.join(DDIR, f"{fam}.canon.json")
        scram_path = os.path.join(DDIR, f"{fam}.scram.json")
        results = {
            "canon": run(cano_path, []),
            "scram": run(scram_path, []),
            "DSR-canon": run(scram_path, ["-r", rules, "--only-use-dsrs-at-start"]),
            "search-DSR": run(scram_path, ["-r", rules]),
        }
        c, s, dc, sd = [
            compression_ratio(results[key])
            for key in ["canon", "scram", "DSR-canon", "search-DSR"]
        ]
        print(f"{fam:8s} {c:6.2f}x {s:6.2f}x {dc:9.2f}x {sd:10.2f}x   {sd - dc:+.2f}")
        results_by_family[fam] = results
    with open(os.path.join(ROOT, "results/scramble_results.json"), "w") as f:
        json.dump(results_by_family, f, indent=2)

if __name__ == "__main__":
    main()

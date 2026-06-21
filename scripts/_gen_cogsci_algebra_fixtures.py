#!/usr/bin/env python3
"""Generate the `*.algebra.out.json` fixtures for tests/cogsci_bfs_test.rs.

Runs the built egg-stitch binary with the SAME flags the test uses
(`check_algebra`) and strips the SAME non-deterministic / bookkeeping fields,
so the frozen fixture matches what the test will produce. One-shot helper; the
canonical regeneration path is `BLESS=1 cargo test --test cogsci_bfs_test`.
"""
import json
import os
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
RULES = "data/domains/cogsci/drawings.rewrites"
DOMAINS = ["dials", "wheels", "furniture", "nuts-bolts"]
TOP_STRIP = ["timestamp", "elapsed_secs", "iteration_times", "input_file", "rules_file", "search"]
LIB_STRIP = ["num_steps_run", "num_expansions", "best_iteration", "best_history"]


def run(domain):
    out = os.path.join(tempfile.gettempdir(), f"gen-algebra-{domain}.json")
    # --iter-limit 6 is REQUIRED for these rules: without it the matmul/choice
    # rules saturate the DSR e-graph without bound and best-first explodes.
    # Hard 300s wall-clock cap so a regression here can't run away again.
    subprocess.run([BIN, "--search", "best-first", "--input", f"data/domains/cogsci/{domain}.json",
                    "--num-steps", "50000", "--num-abstractions", "3", "--max-arity", "2",
                    "--output", out, "--rules", RULES, "--max-match-set", "2000",
                    "--iter-limit", "6"],
                   cwd=ROOT, check=True, timeout=300)
    v = json.load(open(out))
    for k in TOP_STRIP:
        v.pop(k, None)
    for entry in v.get("library", []):
        for k in LIB_STRIP:
            entry.pop(k, None)
    return v


def main():
    dst = os.path.join(ROOT, "data", "expected_outputs", "cogsci")
    os.makedirs(dst, exist_ok=True)
    for d in DOMAINS:
        v = run(d)
        path = os.path.join(dst, f"{d}.algebra.out.json")
        with open(path, "w") as f:
            json.dump(v, f, indent=2, sort_keys=True)
            f.write("\n")
        traj = v.get("cost_at_end_of_each_iter")
        print(f"wrote {path}  (final cost {traj[-1] if traj else '?'})")


if __name__ == "__main__":
    main()

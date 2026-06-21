#!/usr/bin/env python3
"""Reproduce the AIG multiplier live-DSR experiment: baseline vs live-AC vs
at-start abstraction, printing the per-abstraction cost trajectory and the
discovered libraries. SMC, seed 1 (deterministic). Env: ABSTS (default 10).

This is the analysis driver; scripts/test_aig_mult_regression.py locks a fixed
4-abstraction subset as a regression test."""
import json, os, subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
CORPUS = os.path.join(ROOT, "data", "domains", "mult", "all.json")
RULES = os.path.join(ROOT, "data", "domains", "mult", "and_ac.rewrites")
ABSTS = int(os.environ.get("ABSTS", 10))


def run(name, at_start):
    out = os.path.join("/tmp", f"aigexp_{name}.json")
    cmd = [BIN, "-i", CORPUS, "--output", out, "--search", "smc",
           "--language", "op-children", "--max-arity", "4", "--num-abstractions", str(ABSTS),
           "--num-particles", "5000", "--num-steps", "100", "--temperature", "1000",
           "--seed", "1", "--iter-limit", "30"]
    if at_start:
        cmd.append("--only-use-dsrs-at-start")
    if name != "baseline":
        cmd += ["-r", RULES]
    subprocess.run(cmd, cwd=ROOT, check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    d = json.load(open(out))
    return d["cost_at_end_of_each_iter"], [a["pattern"].split(": ", 1)[1] for a in d["library"]]


def main():
    cb, lb = run("baseline", False)
    cl, ll = run("live", False)
    ca, la = run("at-start", True)
    print(f"AIG multiplier cones (800), SMC seed 1, {ABSTS} abstractions. lower=better.\n")
    print(f"FINAL:  baseline {cb[-1]}   live-AC {cl[-1]}   at-start {ca[-1]}\n")
    print(f"{'abst':>4} {'baseline':>9} {'live-AC':>9} {'at-start':>9}")
    for i in range(ABSTS):
        print(f"{i:>4} {cb[i]:>9} {cl[i]:>9} {ca[i]:>9}")
    print("\nlibraries:")
    for i in range(ABSTS):
        print(f"  fn_{i}\n    base: {lb[i]}\n    live: {ll[i]}\n    atst: {la[i]}")


if __name__ == "__main__":
    main()

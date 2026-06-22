#!/usr/bin/env python3
"""Compare DSRs-live vs DSRs-only-at-start on the unfolded list corpus.

Mirrors the molecules Table 5 baseline: best-first, 10k pops, 4 abstractions,
max-arity 2, list DSRs. For each unfolded-list run file we invoke egg-stitch
twice (live DSRs vs one-shot canonicalisation) and diff the outcomes.
"""
import json
import os
import subprocess
import sys
import glob
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
DSR = os.path.join(ROOT, "..", "babble", "harness", "data", "benchmark-dsrs", "list.rewrites")
INPUTS = sorted(glob.glob(os.path.join(ROOT, "data", "domains", "list-unfolded", "*.json")))

STEPS, ROUNDS, ARITY, TIMEOUT = 10_000, 4, 2, 600
OUT = tempfile.mkdtemp(prefix="dsrcmp_")


def run(input_path, at_start):
    """Run egg-stitch once; return parsed result dict (or None on failure)."""
    tag = "atstart" if at_start else "live"
    outp = os.path.join(OUT, f"{os.path.basename(input_path)}.{tag}.json")
    cmd = [BIN, "-i", input_path, "--output", outp, "--search", "best-first",
           "--language", "lambda-calc", "--max-arity", str(ARITY),
           "--num-abstractions", str(ROUNDS), "--num-steps", str(STEPS), "-r", DSR]
    if at_start:
        cmd.append("--only-use-dsrs-at-start")
    try:
        subprocess.run(cmd, cwd=ROOT, check=True, timeout=TIMEOUT,
                       env=dict(os.environ, RUST_BACKTRACE="1"),
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    except subprocess.CalledProcessError as e:
        print(f"  FAIL {tag}: {e.stderr.decode()[-300:]}", file=sys.stderr)
        return None
    except subprocess.TimeoutExpired:
        print(f"  TIMEOUT {tag}", file=sys.stderr)
        return None
    return json.load(open(outp))


def summary(d):
    """Pull the comparable metrics out of a run result."""
    traj = d.get("cost_at_end_of_each_iter") or []
    return dict(
        cost_after_rewrites=d.get("cost_after_rewrites"),
        final_cost=traj[-1] if traj else None,
        traj=traj,
        absts=[a["pattern"] for a in d.get("library", [])],
    )


def main():
    print(f"binary: {BIN}\nDSR: {DSR}\nsteps={STEPS} rounds={ROUNDS} arity={ARITY}\n")
    for inp in INPUTS:
        name = os.path.basename(inp).replace(".json", "")
        nprog = len(json.load(open(inp)))
        print(f"=== {name}  ({nprog} programs) ===")
        live, atstart = run(inp, False), run(inp, True)
        if not live or not atstart:
            print("  (skipped — a run failed)\n")
            continue
        L, A = summary(live), summary(atstart)
        print(f"  cost_after_rewrites:  live={L['cost_after_rewrites']}   at-start={A['cost_after_rewrites']}")
        print(f"  final_cost:           live={L['final_cost']}   at-start={A['final_cost']}")
        print(f"  cost trajectory live:    {L['traj']}")
        print(f"  cost trajectory atstart: {A['traj']}")
        same = L["absts"] == A["absts"]
        print(f"  abstractions identical? {same}")
        if not same:
            for i, (a, b) in enumerate(zip(L["absts"], A["absts"])):
                mark = "" if a == b else "  <-- differ"
                print(f"    [{i}] live:    {a}{mark}")
                if a != b:
                    print(f"        atstart: {b}")
        print()


if __name__ == "__main__":
    main()

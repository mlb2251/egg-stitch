#!/usr/bin/env python3
"""Scaling sweep of the live-vs-at-start DSR gap on the Lample derivative
(poly) corpus. For each corpus size we run no-DSR, DSRs-at-start, and DSRs-live
with one fixed config, and report the live-vs-at-start gap. Runs are strictly
sequential (no CPU/RAM contention) and results stream to stdout as they land.
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
SIZES = [4000, 8000, 16000, 24000]
ABSTS, STEPS, ARITY = 15, 10000, 3

BASE_DSR = """\
add_comm: (@ (@ +. ?x) ?y) => (@ (@ +. ?y) ?x)
mul_comm: (@ (@ *. ?x) ?y) => (@ (@ *. ?y) ?x)
add_iden: (@ (@ +. ?x) 0.) => ?x
sub_iden: (@ (@ -. ?x) 0.) => ?x
mul_iden: (@ (@ *. ?x) 1.) => ?x
div_iden: (@ (@ /. ?x) 1.) => ?x
div_self: (@ (@ /. ?x) ?x) => 1.
pow_iden: (@ (@ power ?x) 1.) => ?x
"""
RW = "/tmp/scale_base.rw"
open(RW, "w").write(BASE_DSR)


def run(size, mode, timeout):
    """Run one egg-stitch; return final cost or None on timeout/error."""
    inp = os.path.join(ROOT, "data", "domains", f"scale-{size}", "all.json")
    outp = f"/tmp/scale_{size}_{mode}.json"
    cmd = [BIN, "-i", inp, "--output", outp, "--search", "best-first",
           "--language", "lambda-calc", "--max-arity", str(ARITY),
           "--num-abstractions", str(ABSTS), "--num-steps", str(STEPS)]
    if mode != "nodsr":
        cmd += ["-r", RW]
    if mode == "atstart":
        cmd.append("--only-use-dsrs-at-start")
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=timeout,
                           stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        return None
    if p.returncode != 0 or not os.path.exists(outp):
        return None
    return json.load(open(outp))["cost_at_end_of_each_iter"][-1]


print(f"config: {ABSTS} absts, {STEPS} steps, arity {ARITY}, base DSRs\n", flush=True)
print(f"{'size':>6} {'no-DSR':>8} {'at-start':>9} {'live':>8} {'gap(at-live)':>13} {'gap%':>6}", flush=True)
rows = {}
for size in SIZES:
    # generous, size-scaled timeouts; live is the expensive mode
    nd = run(size, "nodsr", 1800)
    a = run(size, "atstart", 2400)
    l = run(size, "live", 4500)
    rows[size] = (nd, a, l)
    gap = (a - l) if (a is not None and l is not None) else None
    gp = f"{100*gap/l:.1f}%" if (gap is not None and l) else "-"
    print(f"{size:>6} {str(nd):>8} {str(a):>9} {str(l):>8} {str(gap):>13} {gp:>6}", flush=True)

print("\nSCALE_DONE", flush=True)

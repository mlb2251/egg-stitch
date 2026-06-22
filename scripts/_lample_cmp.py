#!/usr/bin/env python3
"""Live-vs-at-start AC/distrib comparison on the Lample (Facebook) derivative
equations corpus, where prior work says the live-DSR benefit is live-exclusive.
Uses --match-limit to keep distrib from exploding; sweeps num-steps."""
import json, os, subprocess, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "lample-deriv", "all.json")
OUT = tempfile.mkdtemp(prefix="lam_")

ASSOC = ("add_comm: (@ (@ +. ?x) ?y) => (@ (@ +. ?y) ?x)\n"
         "mul_comm: (@ (@ *. ?x) ?y) => (@ (@ *. ?y) ?x)\n"
         "add_assoc_r: (@ (@ +. (@ (@ +. ?a) ?b)) ?c) => (@ (@ +. ?a) (@ (@ +. ?b) ?c))\n"
         "mul_assoc_r: (@ (@ *. (@ (@ *. ?a) ?b)) ?c) => (@ (@ *. ?a) (@ (@ *. ?b) ?c))\n")
DISTRIB = ASSOC + ("distrib: (@ (@ *. ?a) (@ (@ +. ?b) ?c)) => (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c))\n"
                   "factor: (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c)) => (@ (@ *. ?a) (@ (@ +. ?b) ?c))\n")


def run(rules, match_limit, at_start, steps):
    rp = os.path.join(OUT, "r.rw"); open(rp, "w").write(rules or "")
    outp = os.path.join(OUT, "o.json")
    cmd = [BIN, "-i", INPUT, "--output", outp, "--search", "best-first",
           "--language", "lambda-calc", "--max-arity", "3",
           "--num-abstractions", "30", "--num-steps", str(steps), "--iter-limit", "30"]
    if rules is not None: cmd += ["-r", rp]
    if match_limit is not None: cmd += ["--match-limit", str(match_limit)]
    if at_start: cmd.append("--only-use-dsrs-at-start")
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=290, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except subprocess.TimeoutExpired:
        return None, ">290"
    if p.returncode != 0:
        return None, f"rc{p.returncode}"
    d = json.load(open(outp))
    return d["cost_at_end_of_each_iter"][-1], (round(d["elapsed_secs"], 1), d["initial_cost"])


print("Lample-deriv (3000 eqns), 30 abstractions. lower final = better.\n")
base, (be, init) = run(None, None, False, 2000)
print(f"initial cost = {init};  no-rules baseline (2000 steps) = {base} ({be}s)\n")
for steps in (2000, 1000):
    al, ai = run(ASSOC, None, False, steps)
    aa, aai = run(ASSOC, None, True, steps)
    dl, di = run(DISTRIB, 20, False, steps)
    da, dai = run(DISTRIB, 20, True, steps)
    print(f"steps={steps}", flush=True)
    print(f"  assoc   : live={al} {ai}   at-start={aa} {aai}", flush=True)
    print(f"  distrib : live={dl} {di}   at-start={da} {dai}", flush=True)
    print(flush=True)

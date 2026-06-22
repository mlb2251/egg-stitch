#!/usr/bin/env python3
"""Sweep --num-steps (heap-pop cap per abstraction) at FULL 30 abstractions.
Capping steps below the ~5000 the search currently runs cuts compute_cost time
per abstraction, so live distrib completes. Does it stay competitive with assoc
/ at-start as steps shrink?"""
import json, os, subprocess, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "physics-unfolded", "all.json")
OUT = tempfile.mkdtemp(prefix="cmp_")

ASSOC = ("add_comm: (@ (@ +. ?x) ?y) => (@ (@ +. ?y) ?x)\n"
         "mul_comm: (@ (@ *. ?x) ?y) => (@ (@ *. ?y) ?x)\n"
         "add_assoc_r: (@ (@ +. (@ (@ +. ?a) ?b)) ?c) => (@ (@ +. ?a) (@ (@ +. ?b) ?c))\n"
         "mul_assoc_r: (@ (@ *. (@ (@ *. ?a) ?b)) ?c) => (@ (@ *. ?a) (@ (@ *. ?b) ?c))\n")
DISTRIB = ASSOC + ("distrib: (@ (@ *. ?a) (@ (@ +. ?b) ?c)) => (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c))\n"
                   "factor: (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c)) => (@ (@ *. ?a) (@ (@ +. ?b) ?c))\n")


def run(rules, match_limit, at_start, steps):
    rp = os.path.join(OUT, "r.rw"); open(rp, "w").write(rules)
    outp = os.path.join(OUT, "o.json")
    cmd = [BIN, "-i", INPUT, "--output", outp, "--search", "best-first",
           "--language", "lambda-calc", "--max-arity", "3",
           "--num-abstractions", "30", "--num-steps", str(steps),
           "--iter-limit", "30", "-r", rp]
    if match_limit is not None: cmd += ["--match-limit", str(match_limit)]
    if at_start: cmd.append("--only-use-dsrs-at-start")
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=290, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except subprocess.TimeoutExpired:
        return None, ">290"
    if p.returncode != 0:
        return None, f"rc{p.returncode}"
    d = json.load(open(outp))
    return d["cost_at_end_of_each_iter"][-1], round(d["elapsed_secs"], 1)


print("30 abstractions; sweeping num-steps. (lower final = better compression)\n")
for steps in (20000, 2000, 1000, 500, 200):
    al, ae = run(ASSOC, None, False, steps)
    dl, de = run(DISTRIB, 20, False, steps)
    da, dae = run(DISTRIB, 20, True, steps)
    print(f"steps={steps:<6}  assoc-live={str(al):>6}({ae}s)   "
          f"distrib-live={str(dl):>6}({de}s)   distrib-atstart={str(da):>6}({dae}s)",
          flush=True)

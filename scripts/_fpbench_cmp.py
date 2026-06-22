#!/usr/bin/env python3
"""Live-vs-at-start AC/distrib comparison on the real FPBench corpus, using
--match-limit to keep the explosive rules from blowing up live abstraction."""
import json, os, subprocess, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "fpbench", "all.json")
OUT = tempfile.mkdtemp(prefix="fpb_")

ASSOC = ("add_comm: (@ (@ +. ?x) ?y) => (@ (@ +. ?y) ?x)\n"
         "mul_comm: (@ (@ *. ?x) ?y) => (@ (@ *. ?y) ?x)\n"
         "add_assoc_r: (@ (@ +. (@ (@ +. ?a) ?b)) ?c) => (@ (@ +. ?a) (@ (@ +. ?b) ?c))\n"
         "mul_assoc_r: (@ (@ *. (@ (@ *. ?a) ?b)) ?c) => (@ (@ *. ?a) (@ (@ *. ?b) ?c))\n")
DISTRIB = ASSOC + ("distrib: (@ (@ *. ?a) (@ (@ +. ?b) ?c)) => (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c))\n"
                   "factor: (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c)) => (@ (@ *. ?a) (@ (@ +. ?b) ?c))\n")


def run(rules, match_limit, at_start, steps):
    rp = os.path.join(OUT, "r.rw");
    if rules is not None: open(rp, "w").write(rules)
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
    return d["cost_at_end_of_each_iter"][-1], round(d["elapsed_secs"], 1)


print("FPBench (131 real numerical kernels), 30 abstractions. lower=better.\n")
b, be = run(None, None, False, 2000)
print(f"initial=8058;  no-rules baseline (2000 steps) = {b} ({be}s)\n")
for steps in (2000, 1000):
    al, ae = run(ASSOC, None, False, steps)
    aa, aae = run(ASSOC, None, True, steps)
    dl, de = run(DISTRIB, 20, False, steps)
    da, dae = run(DISTRIB, 20, True, steps)
    print(f"steps={steps}", flush=True)
    print(f"  assoc   : live={al}({ae})   at-start={aa}({aae})", flush=True)
    print(f"  distrib : live={dl}({de})   at-start={da}({dae})", flush=True)
    print(flush=True)

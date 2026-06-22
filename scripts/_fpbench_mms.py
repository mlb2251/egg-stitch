#!/usr/bin/env python3
"""Does --max-match-set rescue the LIVE AC/distrib runs (which timed out at 2000
steps) by pruning commutativity-blown best-first successors? FPBench, 30 absts,
2000 steps. Refs: baseline 5897, assoc at-start 5900, distrib at-start 5960."""
import json, os, subprocess, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "fpbench", "all.json")
OUT = tempfile.mkdtemp(prefix="mms_")

ASSOC = ("add_comm: (@ (@ +. ?x) ?y) => (@ (@ +. ?y) ?x)\n"
         "mul_comm: (@ (@ *. ?x) ?y) => (@ (@ *. ?y) ?x)\n"
         "add_assoc_r: (@ (@ +. (@ (@ +. ?a) ?b)) ?c) => (@ (@ +. ?a) (@ (@ +. ?b) ?c))\n"
         "mul_assoc_r: (@ (@ *. (@ (@ *. ?a) ?b)) ?c) => (@ (@ *. ?a) (@ (@ *. ?b) ?c))\n")
DISTRIB = ASSOC + ("distrib: (@ (@ *. ?a) (@ (@ +. ?b) ?c)) => (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c))\n"
                   "factor: (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c)) => (@ (@ *. ?a) (@ (@ +. ?b) ?c))\n")


def run(rules, mms):
    rp = os.path.join(OUT, "r.rw"); open(rp, "w").write(rules)
    outp = os.path.join(OUT, "o.json")
    cmd = [BIN, "-i", INPUT, "--output", outp, "--search", "best-first",
           "--language", "lambda-calc", "--max-arity", "3",
           "--num-abstractions", "30", "--num-steps", "2000", "--iter-limit", "30",
           "--match-limit", "20", "-r", rp]
    if mms is not None: cmd += ["--max-match-set", str(mms)]
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=290, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except subprocess.TimeoutExpired:
        return ">290s"
    if p.returncode != 0:
        return f"rc{p.returncode}"
    d = json.load(open(outp))
    c = d.get("cost_at_end_of_each_iter")
    if not c:
        return f"NO-ABSTRACTION-FOUND ({d['elapsed_secs']:.1f}s)"
    return f"final={c[-1]} ({d['elapsed_secs']:.1f}s)"


print("FPBench live, 30 absts, 2000 steps, match-limit 20. refs: baseline 5897, assoc@start 5900, distrib@start 5960\n")
for name, rules in (("assoc", ASSOC), ("distrib", DISTRIB)):
    for mms in (100000, 50000, 30000, 10000):
        print(f"  {name:8} max-match-set={str(mms):>6}  live {run(rules, mms)}", flush=True)
    print(flush=True)

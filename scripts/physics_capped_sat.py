#!/usr/bin/env python3
"""Can the explosive rules be used WITHOUT blowup by capping saturation depth?

The blowup is non-convergence of saturation, so the obvious lever is a low
--iter-limit: apply distributivity / linear div rules for only a few rounds
(exposing a handful of useful regroupings) instead of to fixpoint. We test
whether a capped-saturation run completes and whether it beats the
associativity-only result (live final 7178).
"""
import json
import os
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "physics-unfolded", "all.json")
OUT = tempfile.mkdtemp(prefix="cap_")

BASE_ASSOC = """\
add_comm: (@ (@ +. ?x) ?y) => (@ (@ +. ?y) ?x)
mul_comm: (@ (@ *. ?x) ?y) => (@ (@ *. ?y) ?x)
add_iden: (@ (@ +. ?x) 0.) => ?x
sub_iden: (@ (@ -. ?x) 0.) => ?x
mul_iden: (@ (@ *. ?x) 1.) => ?x
div_iden: (@ (@ /. ?x) 1.) => ?x
div_self: (@ (@ /. ?x) ?x) => 1.
pow_iden: (@ (@ power ?x) 1.) => ?x
add_assoc_r: (@ (@ +. (@ (@ +. ?a) ?b)) ?c) => (@ (@ +. ?a) (@ (@ +. ?b) ?c))
add_assoc_l: (@ (@ +. ?a) (@ (@ +. ?b) ?c)) => (@ (@ +. (@ (@ +. ?a) ?b)) ?c)
mul_assoc_r: (@ (@ *. (@ (@ *. ?a) ?b)) ?c) => (@ (@ *. ?a) (@ (@ *. ?b) ?c))
mul_assoc_l: (@ (@ *. ?a) (@ (@ *. ?b) ?c)) => (@ (@ *. (@ (@ *. ?a) ?b)) ?c)
"""
DISTRIB = """\
distrib: (@ (@ *. ?a) (@ (@ +. ?b) ?c)) => (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c))
factor:  (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c)) => (@ (@ *. ?a) (@ (@ +. ?b) ?c))
"""
LINEAR = """\
mul_div: (@ (@ *. ?a) (@ (@ /. ?b) ?c)) => (@ (@ /. (@ (@ *. ?a) ?b)) ?c)
div_mul: (@ (@ /. (@ (@ *. ?a) ?b)) ?c) => (@ (@ *. ?a) (@ (@ /. ?b) ?c))
div_div: (@ (@ /. (@ (@ /. ?a) ?b)) ?c) => (@ (@ /. ?a) (@ (@ *. ?b) ?c))
"""

CONFIGS = [
    ("distrib", BASE_ASSOC + DISTRIB),
    ("linear",  BASE_ASSOC + LINEAR),
]


def run(rules_path, iter_limit):
    """Run live-DSR egg-stitch with a given saturation cap; return (final, secs, hit)."""
    outp = os.path.join(OUT, "o.json")
    cmd = [BIN, "-i", INPUT, "--output", outp, "--search", "best-first",
           "--language", "lambda-calc", "--max-arity", "3",
           "--num-abstractions", "30", "--num-steps", "20000",
           "--iter-limit", str(iter_limit), "-r", rules_path]
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=400,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except subprocess.TimeoutExpired:
        return None, ">400", None
    if p.returncode != 0:
        return None, "ERR", None
    d = json.load(open(outp))
    return d["cost_at_end_of_each_iter"][-1], round(d["elapsed_secs"], 1), \
        b"reached expansion budget" in p.stdout


print("baseline refs: assoc-only(full sat) live=7178   base live=7265\n")
for name, rules in CONFIGS:
    rp = os.path.join(OUT, name + ".rw")
    open(rp, "w").write(rules)
    for il in (3, 5, 10):
        final, secs, hit = run(rp, il)
        print(f"{name:8} iter-limit={il:<3}  final={final}  elapsed={secs}s  budget_hit={hit}",
              flush=True)
    print()

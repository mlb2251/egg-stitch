#!/usr/bin/env python3
"""Does capping per-rule matches (--match-limit) let the explosive AC rule sets
(distributivity / linear div) run to completion AND beat the assoc-only result?

The blowup is non-convergence: distrib/linear mint fresh operand combinations
that re-trigger the whole AC ruleset. --match-limit bans a rule for an iteration
once it produces too many matches, so AC fires while the egraph is small then
gets cut off. We sweep the cap (and a tight iter-limit) and report whether the
run completes and whether live finally pulls ahead of at-start.

Refs (full saturation, no cap): base live=7265 at-start=7410;
assoc-only live=7178 at-start=7325; distrib/linear explode (>400s or node cap).
"""
import json
import os
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "physics-unfolded", "all.json")
OUT = tempfile.mkdtemp(prefix="matchlim_")

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
    ("assoc",   BASE_ASSOC),
    ("distrib", BASE_ASSOC + DISTRIB),
    ("linear",  BASE_ASSOC + LINEAR),
]


def run(rules_path, match_limit, iter_limit, at_start):
    """Run egg-stitch once; return (final_cost, elapsed_secs, egraph_size_after) or Nones on timeout/err."""
    outp = os.path.join(OUT, "o.json")
    cmd = [BIN, "-i", INPUT, "--output", outp, "--search", "best-first",
           "--language", "lambda-calc", "--max-arity", "3",
           "--num-abstractions", "30", "--num-steps", "20000",
           "--iter-limit", str(iter_limit), "-r", rules_path]
    if match_limit is not None:
        cmd += ["--match-limit", str(match_limit)]
    if at_start:
        cmd.append("--only-use-dsrs-at-start")
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=400,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except subprocess.TimeoutExpired:
        return None, ">400", None
    if p.returncode != 0:
        return None, "ERR", None
    d = json.load(open(outp))
    # Last "Egraph size:" line printed by load_egraph (post-saturation).
    sizes = [int(l.split()[-1]) for l in p.stdout.decode().splitlines()
             if l.startswith("Egraph size:")]
    return d["cost_at_end_of_each_iter"][-1], round(d["elapsed_secs"], 1), \
        (sizes[1] if len(sizes) > 1 else None)


def main():
    print("refs: assoc-only(full sat) live=7178 at-start=7325 | base live=7265\n")
    for name, rules in CONFIGS:
        rp = os.path.join(OUT, name + ".rw")
        open(rp, "w").write(rules)
        print(f"=== {name} ===", flush=True)
        # assoc converges on its own; cap only matters for distrib/linear.
        for ml in (None, 50, 20):
            for il in (30,):
                lf, ls, lsz = run(rp, ml, il, at_start=False)
                af, _, _ = run(rp, ml, il, at_start=True)
                gap = (af - lf) if (lf and af) else None
                print(f"  match-limit={str(ml):>4} iter-limit={il:<3} "
                      f"live={str(lf):>6} at-start={str(af):>6} "
                      f"gap={str(gap):>5} egraph={str(lsz):>7} elapsed={ls}s",
                      flush=True)
        print(flush=True)


if __name__ == "__main__":
    main()

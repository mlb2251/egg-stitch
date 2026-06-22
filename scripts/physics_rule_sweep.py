#!/usr/bin/env python3
"""Sweep over progressively richer physics rewrite sets to test whether adding
*expanding* algebraic rules (associativity, distributivity, power/division
laws) lets live DSRs find better abstractions than the baseline rule set, which
is mostly reducing/canonicalizing.

For each tier we run egg-stitch in live mode (DSRs kept active during search)
and, for reference, at-start mode (one-shot canonicalization). We report final
cost, whether the abstraction search hit its pop budget, and the abstractions
that appear only at this tier.
"""
import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "physics-unfolded", "all.json")

STEPS, ROUNDS, ARITY, TIMEOUT = 20_000, 30, 3, 400
OUT = tempfile.mkdtemp(prefix="physsweep_")

# Baseline = current babble physics.rewrites (commutativity + reducing identities).
BASE = """\
add_comm: (@ (@ +. ?x) ?y) => (@ (@ +. ?y) ?x)
mul_comm: (@ (@ *. ?x) ?y) => (@ (@ *. ?y) ?x)
add_iden: (@ (@ +. ?x) 0.) => ?x
sub_iden: (@ (@ -. ?x) 0.) => ?x
mul_iden: (@ (@ *. ?x) 1.) => ?x
div_iden: (@ (@ /. ?x) 1.) => ?x
div_self: (@ (@ /. ?x) ?x) => 1.
pow_iden: (@ (@ power ?x) 1.) => ?x
"""

# Associativity, both directions, for + and *.
ASSOC = """\
add_assoc_r: (@ (@ +. (@ (@ +. ?a) ?b)) ?c) => (@ (@ +. ?a) (@ (@ +. ?b) ?c))
add_assoc_l: (@ (@ +. ?a) (@ (@ +. ?b) ?c)) => (@ (@ +. (@ (@ +. ?a) ?b)) ?c)
mul_assoc_r: (@ (@ *. (@ (@ *. ?a) ?b)) ?c) => (@ (@ *. ?a) (@ (@ *. ?b) ?c))
mul_assoc_l: (@ (@ *. ?a) (@ (@ *. ?b) ?c)) => (@ (@ *. (@ (@ *. ?a) ?b)) ?c)
"""

# Distributivity / factoring of * over + (both directions).
DISTRIB = """\
distrib: (@ (@ *. ?a) (@ (@ +. ?b) ?c)) => (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c))
factor:  (@ (@ +. (@ (@ *. ?a) ?b)) (@ (@ *. ?a) ?c)) => (@ (@ *. ?a) (@ (@ +. ?b) ?c))
"""

# Division / power restructuring that the reducing set can't reach.
DIVPOW = """\
mul_div:   (@ (@ *. ?a) (@ (@ /. ?b) ?c)) => (@ (@ /. (@ (@ *. ?a) ?b)) ?c)
div_mul:   (@ (@ /. (@ (@ *. ?a) ?b)) ?c) => (@ (@ *. ?a) (@ (@ /. ?b) ?c))
div_div:   (@ (@ /. (@ (@ /. ?a) ?b)) ?c) => (@ (@ /. ?a) (@ (@ *. ?b) ?c))
pow_mul:   (@ (@ power (@ (@ *. ?a) ?b)) ?n) => (@ (@ *. (@ (@ power ?a) ?n)) (@ (@ power ?b) ?n))
pow_addex: (@ (@ *. (@ (@ power ?a) ?m)) (@ (@ power ?a) ?n)) => (@ (@ power ?a) (@ (@ +. ?m) ?n))
"""

TIERS = [
    ("T0_base", BASE),
    ("T1_assoc", BASE + ASSOC),
    ("T2_distrib", BASE + ASSOC + DISTRIB),
    ("T3_divpow", BASE + ASSOC + DISTRIB + DIVPOW),
]


def run(rules_path, at_start, tag):
    """Run egg-stitch once; return (result_dict_or_None, hit_budget_bool)."""
    outp = os.path.join(OUT, f"{tag}.json")
    cmd = [BIN, "-i", INPUT, "--output", outp, "--search", "best-first",
           "--language", "lambda-calc", "--max-arity", str(ARITY),
           "--num-abstractions", str(ROUNDS), "--num-steps", str(STEPS),
           "-r", rules_path]
    if at_start:
        cmd.append("--only-use-dsrs-at-start")
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=TIMEOUT,
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except subprocess.TimeoutExpired:
        return None, False
    if p.returncode != 0:
        sys.stderr.write(p.stdout.decode()[-400:] + "\n")
        return None, False
    hit = b"reached expansion budget" in p.stdout
    return json.load(open(outp)), hit


def bodies(d):
    """Abstraction bodies (strip the fn_N: prefix)."""
    return [a["pattern"].split(": ", 1)[1] for a in d.get("library", [])]


def main():
    results = {}
    for tag, rules in TIERS:
        rp = os.path.join(OUT, f"{tag}.rewrites")
        open(rp, "w").write(rules)
        nrules = len([l for l in rules.splitlines() if l.strip()])
        print(f"=== {tag}  ({nrules} rules) ===", flush=True)
        live, lhit = run(rp, False, tag + ".live")
        atst, ahit = run(rp, True, tag + ".atstart")
        if not live or not atst:
            print("  (run failed/timeout)\n", flush=True)
            continue
        lf = live["cost_at_end_of_each_iter"][-1]
        af = atst["cost_at_end_of_each_iter"][-1]
        results[tag] = (lf, af, bodies(live))
        print(f"  live final={lf}  (budget hit: {lhit})   "
              f"at-start final={af}  (budget hit: {ahit})", flush=True)
        print(f"  initial={live['initial_cost']}  after_dsr(live)={live['cost_after_rewrites']}\n",
              flush=True)

    # What new abstractions does each tier unlock vs the baseline?
    if "T0_base" in results:
        base_bodies = set(results["T0_base"][2])
        print("=== abstractions unlocked beyond T0_base (live) ===")
        for tag, _ in TIERS:
            if tag == "T0_base" or tag not in results:
                continue
            new = [b for b in results[tag][2] if b not in base_bodies]
            print(f"\n{tag}: {len(new)} new abstraction bodies, "
                  f"final {results[tag][0]} vs base {results['T0_base'][0]}")
            for b in new:
                algebraic = any(op in b for op in ("+.", "*.", "/.", "power"))
                print(f"   {'[alg] ' if algebraic else '      '}{b[:130]}")


if __name__ == "__main__":
    main()

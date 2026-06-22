#!/usr/bin/env python3
"""Refined physics rule test.

The full sweep showed subterm-*duplicating* rules (distributivity, pow_mul)
explode the e-graph past a 400s budget. This isolates the rules that help
without blowing up: associativity plus division/power *rearrangements* where
every metavariable appears exactly once on each side (linear -> no blowup).

Runs each tier in live mode (+ at-start reference) and lists which abstraction
bodies appear only once the richer rules are added.
"""
import json
import os
import subprocess
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "physics-unfolded", "all.json")
STEPS, ROUNDS, ARITY, TIMEOUT = 20_000, 30, 3, 400
OUT = tempfile.mkdtemp(prefix="physref_")

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
ASSOC = """\
add_assoc_r: (@ (@ +. (@ (@ +. ?a) ?b)) ?c) => (@ (@ +. ?a) (@ (@ +. ?b) ?c))
add_assoc_l: (@ (@ +. ?a) (@ (@ +. ?b) ?c)) => (@ (@ +. (@ (@ +. ?a) ?b)) ?c)
mul_assoc_r: (@ (@ *. (@ (@ *. ?a) ?b)) ?c) => (@ (@ *. ?a) (@ (@ *. ?b) ?c))
mul_assoc_l: (@ (@ *. ?a) (@ (@ *. ?b) ?c)) => (@ (@ *. (@ (@ *. ?a) ?b)) ?c)
"""
# Linear rearrangements: every ?var used once per side -> no subterm duplication.
LINEAR = """\
mul_div: (@ (@ *. ?a) (@ (@ /. ?b) ?c)) => (@ (@ /. (@ (@ *. ?a) ?b)) ?c)
div_mul: (@ (@ /. (@ (@ *. ?a) ?b)) ?c) => (@ (@ *. ?a) (@ (@ /. ?b) ?c))
div_div: (@ (@ /. (@ (@ /. ?a) ?b)) ?c) => (@ (@ /. ?a) (@ (@ *. ?b) ?c))
sub_neg: (@ (@ +. ?a) (@ (@ -. 0.) ?b)) => (@ (@ -. ?a) ?b)
"""

TIERS = [
    ("T0_base", BASE),
    ("T1_assoc", BASE + ASSOC),
    ("T2_linear", BASE + ASSOC + LINEAR),
]


def run(rules_path, at_start, tag):
    """Run egg-stitch once; return (result_dict_or_None, hit_budget)."""
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
        return None, False
    return json.load(open(outp)), b"reached expansion budget" in p.stdout


def bodies(d):
    """Abstraction bodies (strip fn_N: prefix)."""
    return [a["pattern"].split(": ", 1)[1] for a in d.get("library", [])]


results = {}
for tag, rules in TIERS:
    rp = os.path.join(OUT, f"{tag}.rewrites")
    open(rp, "w").write(rules)
    n = len([l for l in rules.splitlines() if l.strip()])
    live, lhit = run(rp, False, tag + ".live")
    atst, _ = run(rp, True, tag + ".atstart")
    if not live or not atst:
        print(f"=== {tag} ({n} rules): FAILED/TIMEOUT ===", flush=True)
        continue
    lf, af = live["cost_at_end_of_each_iter"][-1], atst["cost_at_end_of_each_iter"][-1]
    results[tag] = (lf, af, bodies(live))
    print(f"=== {tag} ({n} rules) ===  live={lf} (budget hit {lhit})  at-start={af}", flush=True)

if "T0_base" in results:
    base = set(results["T0_base"][2])
    for tag, _ in TIERS:
        if tag == "T0_base" or tag not in results:
            continue
        new = [b for b in results[tag][2] if b not in base]
        print(f"\n--- {tag}: {len(new)} abstractions beyond baseline "
              f"(live final {results[tag][0]} vs {results['T0_base'][0]}) ---")
        for b in new:
            print(f"   {b[:130]}")

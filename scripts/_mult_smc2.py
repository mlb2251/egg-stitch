#!/usr/bin/env python3
"""Multiplier cones, SMC at temperature 1000 with more resources. Can live AC
(uncapped) beat BOTH baseline and at-start if given enough particles/steps?
Refs (SMC temp100, 5k/100): baseline 13597, live 14364, at-start 15526."""
import json, os, subprocess, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "mult", "all.json")
OUT = tempfile.mkdtemp(prefix="msmc2_")
AC = ("and_comm: (and ?x ?y) => (and ?y ?x)\n"
      "and_assoc: (and (and ?a ?b) ?c) => (and ?a (and ?b ?c))\n"
      "notnot: (not (not ?x)) => ?x\n")


def run(rules, at_start, particles, steps, dead):
    outp = os.path.join(OUT, "o.json")
    cmd = [BIN, "-i", INPUT, "--output", outp, "--search", "smc",
           "--language", "op-children", "--max-arity", "4",
           "--num-abstractions", "10", "--num-particles", str(particles),
           "--num-steps", str(steps), "--temperature", "1000",
           "--dead-runs", str(dead), "--iter-limit", "30", "--seed", "1"]
    if rules is not None:
        rp = os.path.join(OUT, "r.rw"); open(rp, "w").write(rules)
        cmd += ["-r", rp]
    if at_start:
        cmd.append("--only-use-dsrs-at-start")
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=900, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except subprocess.TimeoutExpired:
        return ">900s"
    if p.returncode != 0:
        return f"rc{p.returncode}"
    d = json.load(open(outp)); c = d.get("cost_at_end_of_each_iter")
    return f"{c[-1] if c else None} ({d['elapsed_secs']:.1f}s)"


print("Multiplier cones (800), SMC temp=1000, 10 absts. lower=better.\n")
print("baseline  20k/300   :", run(None, False, 20000, 300, 100), flush=True)
print("at-start  20k/300   :", run(AC, True, 20000, 300, 100), flush=True)
print("AC live   20k/300   :", run(AC, False, 20000, 300, 100), flush=True)
print("AC live   40k/500   :", run(AC, False, 40000, 500, 200), flush=True)

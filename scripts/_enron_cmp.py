#!/usr/bin/env python3
"""Live-vs-at-start on the real Enron formula corpus (op-children). Tests whether
live AC/distrib rewriting beats at-start — i.e. whether the predicted ~0 AC
opportunity (merges 1/112) actually shows up as ~0 compression benefit."""
import json, os, subprocess, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "enron", "all.json")
OUT = tempfile.mkdtemp(prefix="enron_")

AC = ("add_comm: (add ?x ?y) => (add ?y ?x)\n"
      "mul_comm: (mul ?x ?y) => (mul ?y ?x)\n"
      "add_assoc: (add (add ?a ?b) ?c) => (add ?a (add ?b ?c))\n"
      "mul_assoc: (mul (mul ?a ?b) ?c) => (mul ?a (mul ?b ?c))\n")
DISTRIB = AC + ("distrib: (mul ?a (add ?b ?c)) => (add (mul ?a ?b) (mul ?a ?c))\n"
                "factor: (add (mul ?a ?b) (mul ?a ?c)) => (mul ?a (add ?b ?c))\n")


def run(rules, at_start, mms):
    outp = os.path.join(OUT, "o.json")
    cmd = [BIN, "-i", INPUT, "--output", outp, "--search", "best-first",
           "--language", "op-children", "--max-arity", "4",
           "--num-abstractions", "12", "--num-steps", "12000", "--iter-limit", "30"]
    if rules is not None:
        rp = os.path.join(OUT, "r.rw"); open(rp, "w").write(rules)
        cmd += ["-r", rp, "--match-limit", "20"]
        if mms is not None: cmd += ["--max-match-set", str(mms)]
    if at_start: cmd.append("--only-use-dsrs-at-start")
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=290, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except subprocess.TimeoutExpired:
        return ">290s"
    if p.returncode != 0:
        return f"rc{p.returncode}"
    d = json.load(open(outp)); c = d.get("cost_at_end_of_each_iter")
    return f"{c[-1] if c else None} ({d['elapsed_secs']:.1f}s)"


print("Enron formulas (3000 progs, init 32362), op-children, 20 absts, 2000 steps\n")
print("baseline (no rules) :", run(None, False, None), flush=True)
print("AC      live        :", run(AC, False, 30000), flush=True)
print("AC      at-start    :", run(AC, True, 30000), flush=True)
print("distrib live        :", run(DISTRIB, False, 30000), flush=True)
print("distrib at-start    :", run(DISTRIB, True, 30000), flush=True)

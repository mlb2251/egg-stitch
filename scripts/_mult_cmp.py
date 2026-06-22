#!/usr/bin/env python3
"""Decisive test: live vs at-start on the multiplier cone corpus -- the first
corpus measured to have non-canonical shared structure (AC-census merges 63%).
If live-DSR ever beats at-start on real data, it should be here."""
import json, os, subprocess, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "mult", "all.json")
OUT = tempfile.mkdtemp(prefix="mult_")

AC = ("and_comm: (and ?x ?y) => (and ?y ?x)\n"
      "and_assoc: (and (and ?a ?b) ?c) => (and ?a (and ?b ?c))\n"
      "notnot: (not (not ?x)) => ?x\n")


def run(rules, at_start, steps, mms):
    outp = os.path.join(OUT, "o.json")
    cmd = [BIN, "-i", INPUT, "--output", outp, "--search", "best-first",
           "--language", "op-children", "--max-arity", "4",
           "--num-abstractions", "10", "--num-steps", str(steps), "--iter-limit", "30"]
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


S = 10000
print(f"Multiplier cones (800 progs), op-children, 10 absts, {S} steps. lower=better.\n")
print("baseline (no rules)  :", run(None, False, S, None), flush=True)
print("AC      live  nocap  :", run(AC, False, S, None), flush=True)
print("AC      live  ms30k  :", run(AC, False, S, 30000), flush=True)
print("AC      at-start     :", run(AC, True, S, 30000), flush=True)

#!/usr/bin/env python3
"""Multiplier cones, live vs at-start, using SMC search (no cap — SMC samples
actions instead of materializing the full match set, so it may dodge the
best-first match-set blowup). Refs (best-first): baseline 13599, AC live 14521,
AC at-start 15524."""
import json, os, subprocess, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
INPUT = os.path.join(ROOT, "data", "domains", "mult", "all.json")
OUT = tempfile.mkdtemp(prefix="msmc_")
AC = ("and_comm: (and ?x ?y) => (and ?y ?x)\n"
      "and_assoc: (and (and ?a ?b) ?c) => (and ?a (and ?b ?c))\n"
      "notnot: (not (not ?x)) => ?x\n")


def run(rules, at_start, particles, steps):
    outp = os.path.join(OUT, "o.json")
    cmd = [BIN, "-i", INPUT, "--output", outp, "--search", "smc",
           "--language", "op-children", "--max-arity", "4",
           "--num-abstractions", "10", "--num-particles", str(particles),
           "--num-steps", str(steps), "--iter-limit", "30", "--seed", "1"]
    if rules is not None:
        rp = os.path.join(OUT, "r.rw"); open(rp, "w").write(rules)
        cmd += ["-r", rp]
    if at_start:
        cmd.append("--only-use-dsrs-at-start")
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=290, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except subprocess.TimeoutExpired:
        return ">290s"
    if p.returncode != 0:
        return f"rc{p.returncode}: " + p.stdout.decode().strip().splitlines()[-1][:80]
    d = json.load(open(outp)); c = d.get("cost_at_end_of_each_iter")
    return f"{c[-1] if c else None} ({d['elapsed_secs']:.1f}s)"


PART, STEPS = 5000, 100
print(f"Multiplier cones (800), SMC {PART} particles / {STEPS} steps, 10 absts\n")
print("baseline (no rules) :", run(None, False, PART, STEPS), flush=True)
print("AC      live nocap  :", run(AC, False, PART, STEPS), flush=True)
print("AC      at-start    :", run(AC, True, PART, STEPS), flush=True)

#!/usr/bin/env python3
"""Test the 'corpus is already canonical' hypothesis via the molecule scramble
corpora. For each family we run DSR at-start vs DSR live on both the canonical
and the scrambled encoding, and report the live-vs-at-start gap in each.

Prediction if the hypothesis holds (our unfolded physics is near-canonical, so
its gap is tiny): canonical encodings show a small gap, scrambled encodings show
a large one — i.e. the gap tracks how non-canonical the input is.
"""
import json
import os
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
D = os.path.join(ROOT, "data", "domains", "molecules", "scramble")
DSR = os.path.join(ROOT, "data", "domains", "molecules", "molecules.rewrites")
FAMILIES = ["hexyl", "ester", "glycol"]
COMMON = ["--search", "best-first", "--num-abstractions", "8",
          "--num-steps", "80000", "--max-arity", "12"]


def run(inp, at_start):
    """One run; return final cost (or None)."""
    cmd = [BIN, "--input", inp, "--output", "/tmp/mol.json", "-r", DSR] + COMMON
    if at_start:
        cmd.append("--only-use-dsrs-at-start")
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=600,
                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    except subprocess.TimeoutExpired:
        return None
    if p.returncode != 0:
        print("  ERR:", p.stderr.decode()[-200:])
        return None
    return json.load(open("/tmp/mol.json"))["cost_at_end_of_each_iter"][-1]


print(f"{'family':8} {'encoding':6}  live  at-start   gap(at-live)")
for fam in FAMILIES:
    for enc in ["canon", "scram"]:
        inp = os.path.join(D, f"{fam}.{enc}.json")
        live = run(inp, False)
        atst = run(inp, True)
        if live is None or atst is None:
            print(f"{fam:8} {enc:6}  (failed)")
            continue
        gap = atst - live
        print(f"{fam:8} {enc:6}  {live:5} {atst:7}     {gap:+d}", flush=True)

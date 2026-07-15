#!/usr/bin/env python3
"""Ablation comparison for the algebraic drawing rewrites.

Runs the default rule set (data/domains/cogsci/drawings.rewrites) against each
ablation in this directory, plus babble's per-domain rewrites as a baseline. For
each (domain, ruleset) we run egg-stitch best-first and capture, from stdout,
(a) the canonical corpus cost after the rules are applied ("Weight ... after rules")
and the e-graph size, and (b) the sequence of abstractions best-first finds
("[expansion N] new best: COST PATTERN"). A hard wall-clock cap keeps it bounded;
best-first prints incrementally, so we still see the best abstractions found so far.

Env: ABSTS (5), STEPS (20000), ARITY (2), TIMEOUT s (240).
"""
import os
import re
import subprocess
import sys

# scripts/drawings_ablation/<this file> -> repo root is three levels up.
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
ABL = "scripts/drawings_ablation"  # rule paths below are relative to ROOT (the cwd)
ABSTS = int(os.environ.get("ABSTS", 5))
STEPS = int(os.environ.get("STEPS", 20000))
ARITY = int(os.environ.get("ARITY", 2))
ITER_LIMIT = int(os.environ.get("ITER_LIMIT", 6))
# Per-factor match-set cap: bounds the abstraction-search blowup the non-confluent
# choice rules cause (iter-limit can't catch it). Results were measured at 2000.
MAX_MATCH_SET = int(os.environ.get("MAX_MATCH_SET", 2000))
TIMEOUT = int(os.environ.get("TIMEOUT", 240))

DOMAINS = ["dials", "nuts-bolts"]
RULESETS = {
    "baseline": "../babble/harness/data/benchmark-dsrs/drawings.{d}.rewrites",
    "default": "data/domains/cogsci/drawings.rewrites",
    "ablate-matmul": f"{ABL}/drawings.ablate-matmul.rewrites",
    "ablate-choice": f"{ABL}/drawings.ablate-choice.rewrites",
    "forward-arith-commute": f"{ABL}/drawings.forward-arith-commute.rewrites",
    "forward-matmul-fold": f"{ABL}/drawings.forward-matmul-fold.rewrites",
}

W_AFTER = re.compile(r"after rules:\s*(\d+)")
EG_SIZE = re.compile(r"Egraph size:\s*(\d+)")
NEW_BEST = re.compile(r"new best:\s*(\d+)\s*(.*?)\s*\(t=")


def run(domain, rules_rel):
    """Run once; return (after_rules_weight, egraph_size, [(cost, pattern), ...])."""
    rules = rules_rel.format(d=domain)
    cmd = [BIN, "-i", f"data/domains/cogsci/{domain}.json",
           "--search", "best-first", "--language", "op-children",
           "--max-arity", str(ARITY), "--num-abstractions", str(ABSTS),
           "--num-steps", str(STEPS), "--iter-limit", str(ITER_LIMIT),
           "--max-match-set", str(MAX_MATCH_SET), "-r", rules]
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=TIMEOUT,
                           capture_output=True, text=True, check=True)
        out = p.stdout
    except subprocess.TimeoutExpired as e:
        out = (e.stdout or b"").decode() if isinstance(e.stdout, bytes) else (e.stdout or "")
    after = [int(m) for m in W_AFTER.findall(out)]
    egs = [int(m) for m in EG_SIZE.findall(out)]
    bests = [(int(c), pat) for c, pat in NEW_BEST.findall(out)]
    return (after[-1] if after else None, max(egs) if egs else None, bests)


def main():
    print(f"config: {ABSTS} absts, {STEPS} steps, arity {ARITY}, cap {TIMEOUT}s\n", flush=True)
    for domain in DOMAINS:
        print(f"################## {domain} ##################", flush=True)
        for name, rel in RULESETS.items():
            aw, egs, bests = run(domain, rel)
            print(f"\n--- {name} ---", flush=True)
            print(f"  cost after rules: {aw}    e-graph size: {egs}", flush=True)
            print(f"  abstractions found (cost, pattern):", flush=True)
            for c, pat in bests:
                print(f"    {c:>8}  {pat}", flush=True)
            if not bests:
                print("    (none within the time cap)", flush=True)
        print(flush=True)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Comprehensive cogsci rule comparison at a fixed 4 abstractions.

Covers all four cogsci domains x {baseline, ablate-choice (confluent core), default} x
{live, DSRs-only-at-start}. Live = rules active during search and re-applied
between abstractions; at-start = normalize once, extract min-term, search a
rule-free e-graph (--only-use-dsrs-at-start). Reports final cost (lower = better,
comparable across rulesets) and the live-vs-at-start delta.

Env: ABSTS (4), STEPS (50000), ARITY (2), ITER_LIMIT (6), TIMEOUT s (500).
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
ABSTS = int(os.environ.get("ABSTS", 4))
STEPS = int(os.environ.get("STEPS", 50000))
ARITY = int(os.environ.get("ARITY", 2))
ITER_LIMIT = int(os.environ.get("ITER_LIMIT", 6))
TIMEOUT = int(os.environ.get("TIMEOUT", 500))

DOMAINS = ["dials", "furniture", "nuts-bolts", "wheels"]
# ablate-choice = confluent core (live should TIE at-start);
# default = full set incl. non-confluent choice rules (live should BEAT at-start).
RULESETS = {
    "baseline":     "../babble/harness/data/benchmark-dsrs/drawings.{d}.rewrites",
    "ablate-choice": "data/domains/cogsci/drawings.ablate-choice.rewrites",
    "default":       "data/domains/cogsci/drawings.rewrites",
}


def run(domain, rules_rel, at_start):
    """Run once; return final cost (last trajectory point) or None."""
    rules = rules_rel.format(d=domain)
    tag = f"{domain}_{os.path.basename(rules)}_{'as' if at_start else 'live'}"
    outp = f"/tmp/lvas_{tag}.json"
    cmd = [BIN, "-i", f"data/domains/cogsci/{domain}.json", "--output", outp,
           "--search", "best-first", "--language", "op-children",
           "--max-arity", str(ARITY), "--num-abstractions", str(ABSTS),
           "--num-steps", str(STEPS), "--iter-limit", str(ITER_LIMIT), "-r", rules]
    if at_start:
        cmd.append("--only-use-dsrs-at-start")
    try:
        subprocess.run(cmd, cwd=ROOT, check=True, timeout=TIMEOUT,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    except subprocess.CalledProcessError as e:
        print(f"  FAIL {tag}: {e.stderr.decode()[-200:]}", file=sys.stderr)
        return None
    except subprocess.TimeoutExpired:
        print(f"  TIMEOUT {tag}", file=sys.stderr)
        return None
    traj = json.load(open(outp)).get("cost_at_end_of_each_iter") or []
    return traj[-1] if traj else None


def main():
    print(f"config: FIXED {ABSTS} absts, {STEPS} steps, arity {ARITY}, iter-limit {ITER_LIMIT}\n", flush=True)
    print(f"final cost (lower=better); live-vs-at-start delta positive => live wins\n", flush=True)
    hdr = f"{'domain':<11}{'ruleset':<10}{'live':>9}{'at-start':>10}{'live wins':>11}"
    print(hdr, flush=True)
    print("-" * len(hdr), flush=True)
    results = {}
    for domain in DOMAINS:
        for name, rel in RULESETS.items():
            live = run(domain, rel, at_start=False)
            atst = run(domain, rel, at_start=True)
            results[(domain, name)] = (live, atst)
            delta = (f"{(atst - live) / atst * 100:+.1f}%" if live and atst else "n/a")
            print(f"{domain:<11}{name:<10}{str(live):>9}{str(atst):>10}{delta:>11}", flush=True)
        print(flush=True)
    # baseline-vs-algebra summary (live mode), final cost
    print("\nlive final cost, baseline vs best algebra (lower=better):", flush=True)
    for domain in DOMAINS:
        b = results[(domain, "baseline")][0]
        c = results[(domain, "ablate-choice")][0]
        f = results[(domain, "default")][0]
        best = min(x for x in (c, f) if x is not None) if (c or f) else None
        impr = (f"{(b - best) / b * 100:+.1f}%" if b and best else "n/a")
        print(f"  {domain:<11} baseline={b}  ablate-choice={c}  default={f}   algebra best vs baseline: {impr}", flush=True)


if __name__ == "__main__":
    main()

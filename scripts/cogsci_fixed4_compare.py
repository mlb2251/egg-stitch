#!/usr/bin/env python3
"""Fixed-abstraction comparison (molecules Table-5 style): existing babble rewrites
vs the affine-algebra rule sets, at a FIXED 4 abstractions run to convergence.

More abstractions found in a time window is not better compression; the honest
comparison holds the abstraction count fixed and compares the resulting cost.
For each (domain, ruleset) we run egg-stitch best-first with --num-abstractions 4
and a step budget large enough to converge each round, then report final cost,
compression ratio, and the 4 learned abstractions.

Env: ABSTS (4), STEPS (50000), ARITY (2), TIMEOUT s (600).
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
TIMEOUT = int(os.environ.get("TIMEOUT", 600))
# Cap DSR e-saturation rounds so the expanding rules (c_comm/assoc/unroll) can't
# blow the e-graph up. Applied uniformly: baseline/ablate-choice converge well within
# it, so they're unaffected; only the explosive "default" set is bounded.
ITER_LIMIT = int(os.environ.get("ITER_LIMIT", 6))
# Per-factor match-set cap: bounds the abstraction-search blowup the non-confluent
# choice rules cause (iter-limit can't catch it). Results were measured at 2000.
MAX_MATCH_SET = int(os.environ.get("MAX_MATCH_SET", 2000))

DOMAINS = ["dials", "nuts-bolts"]
RULESETS = {
    "baseline":      "../babble/harness/data/benchmark-dsrs/drawings.{d}.rewrites",
    "ablate-choice": "data/domains/cogsci/drawings.ablate-choice.rewrites",
    "default":       "data/domains/cogsci/drawings.rewrites",
}


def run(domain, rules_rel):
    """Run once at fixed ABSTS; return result dict or None (timeout/error)."""
    rules = rules_rel.format(d=domain)
    outp = f"/tmp/fixed4_{domain}_{os.path.basename(rules)}.json"
    cmd = [BIN, "-i", f"data/domains/cogsci/{domain}.json", "--output", outp,
           "--search", "best-first", "--language", "op-children",
           "--max-arity", str(ARITY), "--num-abstractions", str(ABSTS),
           "--num-steps", str(STEPS), "--iter-limit", str(ITER_LIMIT),
           "--max-match-set", str(MAX_MATCH_SET), "-r", rules]
    try:
        subprocess.run(cmd, cwd=ROOT, check=True, timeout=TIMEOUT,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    except subprocess.CalledProcessError as e:
        print(f"  FAIL: {e.stderr.decode()[-300:]}", file=sys.stderr)
        return None
    except subprocess.TimeoutExpired:
        print(f"  TIMEOUT (>{TIMEOUT}s) — did not converge 4 abstractions", file=sys.stderr)
        return None
    return json.load(open(outp))


def main():
    print(f"config: FIXED {ABSTS} abstractions, {STEPS} steps, arity {ARITY}, "
          f"iter-limit {ITER_LIMIT}, cap {TIMEOUT}s\n", flush=True)
    for domain in DOMAINS:
        print(f"################## {domain} ##################", flush=True)
        rows = {}
        for name, rel in RULESETS.items():
            d = run(domain, rel)
            rows[name] = d
            print(f"\n--- {name} ---", flush=True)
            if d is None:
                print("  (no result)", flush=True)
                continue
            traj = d.get("cost_at_end_of_each_iter") or []
            print(f"  initial_cost={d.get('initial_cost')}  final_cost={traj[-1] if traj else None}"
                  f"  compression={d.get('compression_ratio'):.4f}", flush=True)
            print(f"  cost trajectory: {traj}", flush=True)
            print(f"  abstractions:", flush=True)
            for a in d.get("library", []):
                print(f"    [{a['arity']}] {a['pattern']}", flush=True)
        # headline: final cost, lower is better
        print(f"\n  >>> {domain} final_cost:", {n: (r.get('cost_at_end_of_each_iter') or [None])[-1]
              if r else None for n, r in rows.items()}, flush=True)
        print(flush=True)


if __name__ == "__main__":
    main()

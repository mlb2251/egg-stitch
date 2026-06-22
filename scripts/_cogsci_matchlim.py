#!/usr/bin/env python3
"""Does --match-limit let the FULL (explosive) drawing algebra run live and beat
the contract-only workaround? cogsci, op-children, fixed-4 abstractions.

Compares: contract-only (current safe workaround) | full-algebra live no cap
(expected to blow up) | full-algebra live with match-limit | full at-start."""
import json, os, subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
CONTRACT = "data/domains/cogsci/drawings.algebra-contract.rewrites"
FULL = "data/domains/cogsci/drawings.algebra.rewrites"
ABSTS, STEPS, ARITY, ITER = 4, 50000, 2, 6
DOMAINS = ["dials", "nuts-bolts"]


def run(domain, rules, match_limit, at_start):
    outp = f"/tmp/cml_{domain}.json"
    cmd = [BIN, "-i", f"data/domains/cogsci/{domain}.json", "--output", outp,
           "--search", "best-first", "--language", "op-children",
           "--max-arity", str(ARITY), "--num-abstractions", str(ABSTS),
           "--num-steps", str(STEPS), "--iter-limit", str(ITER), "-r", rules]
    if match_limit is not None: cmd += ["--match-limit", str(match_limit)]
    if at_start: cmd.append("--only-use-dsrs-at-start")
    try:
        p = subprocess.run(cmd, cwd=ROOT, timeout=300, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    except subprocess.TimeoutExpired:
        return ">300s"
    out = p.stdout.decode()
    egs = [int(l.split()[-1]) for l in out.splitlines() if l.startswith("Egraph size:")]
    if p.returncode != 0:
        return f"FAIL rc={p.returncode} egmax={max(egs) if egs else '?'}"
    d = json.load(open(outp))
    return (f"final={d['cost_at_end_of_each_iter'][-1]} "
            f"(init {d['initial_cost']}, {d['elapsed_secs']:.1f}s, egmax={max(egs) if egs else '?'})")


for dom in DOMAINS:
    print(f"########## {dom} ##########", flush=True)
    print(f"  contract   live      : {run(dom, CONTRACT, None, False)}", flush=True)
    print(f"  full       live nocap : {run(dom, FULL, None, False)}", flush=True)
    print(f"  full       live ml50  : {run(dom, FULL, 50, False)}", flush=True)
    print(f"  full       live ml20  : {run(dom, FULL, 20, False)}", flush=True)
    print(f"  full       at-start ml20: {run(dom, FULL, 20, True)}", flush=True)
    print(flush=True)

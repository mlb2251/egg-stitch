#!/usr/bin/env python3
"""Build an egg-stitch op-children corpus from an AIGER circuit's bounded-depth
gate cones (named signal leaves, so abstraction introduces metavars over inputs).
The multiplier carries genuine non-canonical AND/NOT structure (AC-census merges
63% of surface forms) -- the live-DSR lever every prior corpus lacked."""
import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from aig_cones import read_aig, cone, size

AIG = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "data", "domains", "mult", "multiplier.aig")
K = int(sys.argv[2]) if len(sys.argv) > 2 else 4
NAME = sys.argv[3] if len(sys.argv) > 3 else "mult"
N = 800


def sexpr(t):
    if len(t) == 1:                       # leaf: 's<var>' or 'C0'
        return t[0]
    return "(" + t[0] + " " + " ".join(sexpr(c) for c in t[1:]) + ")"


def main():
    M, I, L, O, A, outs, ands = read_aig(AIG)
    cones = (cone(2 * (I + 1 + j), ands, I, K, named=True) for j in range(A))
    progs = [sexpr(t) for t in cones if size(t) >= 6]
    if len(progs) > N:
        step = len(progs) / N
        progs = [progs[int(i * step)] for i in range(N)]
    out = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                       "data", "domains", NAME, "all.json")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    json.dump(progs, open(out, "w"), indent=0)
    print(f"wrote {len(progs)} programs ({len(set(progs))} distinct) -> {out}")
    for p in progs[:5]:
        print("  ", p[:120])


if __name__ == "__main__":
    main()

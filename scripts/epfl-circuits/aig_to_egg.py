#!/usr/bin/env python3
"""Build an egg-stitch op-children corpus from an AIGER circuit's **input-bounded**
gate cones (k-feasible cuts, named signal leaves, so abstraction introduces
metavars over inputs). The multiplier carries genuine non-canonical AND/NOT
structure; with the best (factoring) ruleset, *live* abstraction beats both the
no-rules baseline and at-start canonicalisation by a wide margin on these cuts.
Usage: aig_to_egg.py [file.aig] [K] [circuit] [out.json]"""
import sys, os, json
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from aig_cones import read_aig, kcone, to_tup, size

HERE = os.path.dirname(os.path.abspath(__file__))
# Repo root is three levels up: scripts/epfl-circuits/aig_to_egg.py.
ROOT = os.path.dirname(os.path.dirname(HERE))
DOMAIN = os.path.join(ROOT, "data", "domains", "epfl-circuits")
# The source .aig lives alongside the generators (it's input data, not a corpus).
AIG = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "multiplier.aig")
K = int(sys.argv[2]) if len(sys.argv) > 2 else 6
CIRCUIT = sys.argv[3] if len(sys.argv) > 3 else "mult"
# Optional explicit output path (argv[4]); defaults to
# data/domains/epfl-circuits/<circuit>.json. The byte-identity regression test
# passes a temp path so it can regenerate without clobbering the committed corpus.
OUT = sys.argv[4] if len(sys.argv) > 4 else None
N = 800


def sexpr(t):
    if len(t) == 1:                       # leaf: 's<var>' or 'C0'
        return t[0]
    return "(" + t[0] + " " + " ".join(sexpr(c) for c in t[1:]) + ")"


def main():
    M, I, L, O, A, outs, ands = read_aig(AIG)
    cones = (to_tup(kcone(2 * (I + 1 + j), ands, I, K), named=True) for j in range(A))
    progs = [sexpr(t) for t in cones if size(t) >= 6]
    if len(progs) > N:
        step = len(progs) / N
        progs = [progs[int(i * step)] for i in range(N)]
    out = OUT or os.path.join(DOMAIN, f"{CIRCUIT}.json")
    os.makedirs(os.path.dirname(out), exist_ok=True)
    json.dump(progs, open(out, "w"), indent=0)
    print(f"wrote {len(progs)} programs ({len(set(progs))} distinct) -> {out}")
    for p in progs[:5]:
        print("  ", p[:120])


if __name__ == "__main__":
    main()

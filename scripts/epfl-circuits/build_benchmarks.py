#!/usr/bin/env python3
"""One-sweep benchmark builder for the epfl-circuits family.

Survey every EPFL Combinational Benchmark Suite circuit, score each on two axes,
keep the ones above the median on BOTH, and write their corpora to
data/domains/epfl-circuits/ (removing any others). The two axes:

  - distinct cone shapes -- structural diversity (variety the DSRs can merge);
  - no-rules compression  -- run egg-stitch with no DSRs; high ratio means real
                             repeated structure to abstract.

Diverse AND redundant is the regime where live DSRs help; either alone is a poor
predictor (diverse-but-unique control logic and redundant-but-canonical adders
both fall flat). Run once to (re)build the set:
    cargo build --release && python3 scripts/epfl-circuits/build_benchmarks.py
"""
import os
import re
import json
import statistics
import subprocess
import sys
import tempfile
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, HERE)
from aig_cones import read_aig, kcone, to_tup, size, free_vars

REF = "52b26f0e2cf1e88298a8b76c5e68e75013ba3977"
K, N = 6, 800
DOMAIN = os.path.join(ROOT, "data", "domains", "epfl-circuits")
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")

# The EPFL Combinational Benchmark Suite: arithmetic + random/control.
CIRCUITS = {
    "arithmetic": ["adder", "bar", "div", "hyp", "log2", "max", "multiplier", "sin", "sqrt", "square"],
    "random_control": ["arbiter", "cavlc", "ctrl", "dec", "i2c", "int2float", "mem_ctrl", "priority", "router", "voter"],
}

_LEAF = re.compile(r"s\d+|\$\d+|C0")  # signal / free-var / constant leaf tokens


def sexpr(t):
    return t[0] if len(t) == 1 else "(" + t[0] + " " + " ".join(sexpr(c) for c in t[1:]) + ")"


def fetch(sub, name, dst):
    url = f"https://raw.githubusercontent.com/lsils/benchmarks/{REF}/{sub}/{name}.aig"
    req = urllib.request.Request(url, headers={"User-Agent": "egg-stitch"})
    with urllib.request.urlopen(req, timeout=120) as resp, open(dst, "wb") as f:
        f.write(resp.read())


def build_corpus(aig):
    """The K=6 input-bounded cone corpus, identical to aig_to_egg.py (cones
    >=6 nodes, free-var leaves, stride-sampled to N)."""
    M, I, L, O, A, outs, ands = read_aig(aig)
    cones = (to_tup(kcone(2 * (I + 1 + j), ands, I, K), named=True) for j in range(A))
    progs = [free_vars(sexpr(t)) for t in cones if size(t) >= 6]
    if len(progs) > N:
        step = len(progs) / N
        progs = [progs[int(i * step)] for i in range(N)]
    return progs


def no_rules_compression(corpus_path):
    """initial/final cost from a no-DSR egg-stitch SMC run (seed 1, deterministic).
    Uses `op-children-db` so `$n` is a real free variable (banned from abstraction
    bodies); sym=1 keeps the cost scale matched to the temperature used below."""
    out = corpus_path + ".out"
    subprocess.run(
        [BIN, "-i", corpus_path, "--output", out, "--search", "smc", "--language", "op-children-db",
         "--max-arity", "4", "--num-abstractions", "4", "--num-particles", "500", "--num-steps", "100",
         "--temperature", "100", "--seed", "1", "--iter-limit", "30"],
        check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    d = json.load(open(out))
    return d["initial_cost"] / d["final_cost"]


def main():
    if not os.path.exists(BIN):
        raise SystemExit(f"{BIN} not found -- run `cargo build --release` first.")
    tmp = tempfile.mkdtemp(prefix="epfl-build-")
    rows = []  # (name, distinct_shapes, no_rules_compression, corpus_path)
    for sub, names in CIRCUITS.items():
        for name in names:
            aig = os.path.join(tmp, f"{name}.aig")
            corpus = os.path.join(tmp, f"{name}.json")
            try:
                fetch(sub, name, aig)
                progs = build_corpus(aig)
                json.dump(progs, open(corpus, "w"), indent=0)
                shapes = len({_LEAF.sub("_", p) for p in progs})
                comp = no_rules_compression(corpus)
                rows.append((name, shapes, comp, corpus))
                print(f"  {name:10} distinct_shapes={shapes:>4}  no_rules_compression={comp:.2f}x", flush=True)
            except Exception as e:
                print(f"  {name:10} FAILED: {e}", flush=True)

    med_shapes = statistics.median(r[1] for r in rows)
    med_comp = statistics.median(r[2] for r in rows)
    selected = [r for r in rows if r[1] > med_shapes and r[2] > med_comp]
    print(f"\nmedians: distinct_shapes={med_shapes}  no_rules_compression={med_comp:.2f}x")
    print("selected (above both):", ", ".join(sorted(r[0] for r in selected)))

    # Write the selected corpora; drop any other *.json already in the domain.
    os.makedirs(DOMAIN, exist_ok=True)
    keep = {r[0] for r in selected}
    for existing in os.listdir(DOMAIN):
        if existing.endswith(".json") and existing[:-5] not in keep:
            os.remove(os.path.join(DOMAIN, existing))
    for name, _, _, corpus in selected:
        with open(corpus) as src, open(os.path.join(DOMAIN, f"{name}.json"), "w") as dst:
            dst.write(src.read())
    print(f"wrote {len(selected)} corpora to {DOMAIN}")


if __name__ == "__main__":
    main()

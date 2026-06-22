#!/usr/bin/env python3
"""Survey the whole EPFL benchmark suite for corpus suitability.

For each circuit, extract the K=6 input-bounded cones the corpus generator uses
and report the cone-size distribution plus the number of distinct cone
*structures* (shapes, with leaf names blanked). Distinct-shape count is the
metric that tracks the live-DSR abstraction edge -- diverse structure (the same
logic recurring in many non-canonical forms) is what the rules can merge; a few
canonical cells repeated (adder, div) leaves them little to do. Ranks the suite
and reports the top 3.
"""
import os
import sys
import tempfile
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from aig_cones import read_aig, kcone, to_tup, size

REF = "52b26f0e2cf1e88298a8b76c5e68e75013ba3977"
K, N = 6, 800

# The EPFL Combinational Benchmark Suite: arithmetic + random/control.
CIRCUITS = {
    "arithmetic": ["adder", "bar", "div", "hyp", "log2", "max", "multiplier", "sin", "sqrt", "square"],
    "random_control": ["arbiter", "cavlc", "ctrl", "dec", "i2c", "int2float", "mem_ctrl", "priority", "router", "voter"],
}


def shape(t):
    """S-expression with every leaf blanked to `_` -- the cone's structure only."""
    return "_" if len(t) == 1 else "(" + t[0] + " " + " ".join(shape(c) for c in t[1:]) + ")"


def fetch(sub, name, dst):
    url = f"https://raw.githubusercontent.com/lsils/benchmarks/{REF}/{sub}/{name}.aig"
    req = urllib.request.Request(url, headers={"User-Agent": "egg-stitch"})
    with urllib.request.urlopen(req, timeout=120) as resp, open(dst, "wb") as f:
        f.write(resp.read())


def analyze(aig):
    """(ANDs, n_cones, median, max, distinct_shapes) for one circuit's K=6 cones,
    filtered to >=6 nodes and stride-sampled to N like the corpus generator."""
    M, I, L, O, A, outs, ands = read_aig(aig)
    items = []  # (size, shape) per non-trivial cone; trees discarded to bound memory
    for j in range(A):
        t = to_tup(kcone(2 * (I + 1 + j), ands, I, K), named=True)
        s = size(t)
        if s >= 6:
            items.append((s, shape(t)))
    if len(items) > N:
        step = len(items) / N
        items = [items[int(i * step)] for i in range(N)]
    if not items:
        return A, 0, 0, 0, 0
    sizes = sorted(s for s, _ in items)
    shapes = {sh for _, sh in items}
    return A, len(items), sizes[len(sizes) // 2], sizes[-1], len(shapes)


def main():
    tmp = tempfile.mkdtemp(prefix="epfl-survey-")
    rows = []
    print(f"K={K} cones, sampled to {N}:\n", flush=True)
    for sub, names in CIRCUITS.items():
        for name in names:
            try:
                fetch(sub, name, os.path.join(tmp, f"{name}.aig"))
                ands, n, med, mx, shapes = analyze(os.path.join(tmp, f"{name}.aig"))
                rows.append((name, ands, n, med, mx, shapes))
                ratio = mx / med if med else 0
                print(f"  {name:10} ANDs={ands:>6}  cones={n:>4}  med/max={med}/{mx}  "
                      f"max/med={ratio:.1f}  distinct_shapes={shapes}", flush=True)
            except Exception as e:
                print(f"  {name:10} FAILED: {e}", flush=True)

    rows.sort(key=lambda r: r[5], reverse=True)
    print("\nRanked by distinct shapes:")
    for name, ands, n, med, mx, shapes in rows:
        print(f"  {name:10} distinct_shapes={shapes:>4}/{n}  med/max={med}/{mx}")
    print("\nTOP 3:", ", ".join(r[0] for r in rows[:3]))


if __name__ == "__main__":
    main()

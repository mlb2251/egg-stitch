#!/usr/bin/env python3
"""Parse a binary-AIGER (.aig) circuit, extract bounded-depth fan-in cones for
every AND gate (DAG -> set of small boolean trees, cutting at primary inputs /
depth limit -> named leaves), and run the AC-census: how much does AND
commutativity+associativity + double-negation removal collapse distinct cone
structures? A big collapse = real non-canonical shared structure (the live-DSR
lever). Usage: aig_cones.py <file.aig> [depth]"""
import sys, collections


def read_aig(path):
    with open(path, "rb") as f:
        parts = f.readline().split()
        assert parts[0] == b"aig"
        M, I, L, O, A = map(int, parts[1:6])
        outputs = [int(f.readline()) for _ in range(O)]
        ands = {}

        def dec():
            x = i = 0
            while True:
                b = f.read(1)[0]
                x |= (b & 0x7f) << (7 * i)
                if not (b & 0x80):
                    return x
                i += 1
        for j in range(A):
            lhs = 2 * (I + L + 1 + j)
            d0 = dec(); d1 = dec()
            r0 = lhs - d0; r1 = r0 - d1
            ands[lhs] = (r0, r1)
    return M, I, L, O, A, outputs, ands


def cone(lit, ands, I, depth, named):
    """Bounded-depth fan-in tree. named=True keeps real signal ids as leaves
    (corpus); named=False collapses every leaf to LEAF (structural sketch)."""
    if lit & 1:
        return ("not", cone(lit ^ 1, ands, I, depth, named))
    if lit == 0:
        return ("C0",)
    var = lit >> 1
    if var <= I or depth == 0 or lit not in ands:
        return (("s%d" % var) if named else "LEAF",)
    r0, r1 = ands[lit]
    return ("and", cone(r0, ands, I, depth - 1, named), cone(r1, ands, I, depth - 1, named))


def size(t): return 1 if len(t) == 1 else 1 + sum(size(c) for c in t[1:])


def sk(t):
    return t[0] if len(t) == 1 else t[0] + "(" + ",".join(sk(c) for c in t[1:]) + ")"


def norm(t):
    """AC normal form: flatten+sort AND chains, kill double negation."""
    if len(t) == 1:
        return t
    if t[0] == "not":
        c = norm(t[1])
        return c[1] if c[0] == "not" else ("not", c)          # ¬¬x = x
    if t[0] == "and":
        kids = []
        for c in (norm(t[1]), norm(t[2])):
            kids.extend(c[1:] if c[0] == "and" else [c])
        kids.sort(key=sk)
        return ("and", *kids)
    return t


def main():
    path = sys.argv[1]
    K = int(sys.argv[2]) if len(sys.argv) > 2 else 4
    M, I, L, O, A, outs, ands = read_aig(path)
    print(f"{path}: M={M} I={I} O={O} ANDs={A}; cone depth={K}\n")

    cones = [cone(2 * (I + 1 + j), ands, I, K, named=False) for j in range(A)]
    NT = [t for t in cones if size(t) >= 6]                     # non-trivial cells
    print(f"AND gates={A}  non-trivial cones(>=6 nodes)={len(NT)}")
    raw = collections.Counter(sk(t) for t in NT)
    acn = collections.Counter(sk(norm(t)) for t in NT)
    print(f"unique raw structures:      {len(raw)}")
    print(f"unique AC+¬¬ normalized:    {len(acn)}   "
          f"(collapses {len(raw)-len(acn)} distinct surface forms -> live-DSR lever)\n")
    print("top recurring normalized cells (count : structure):")
    for s, n in acn.most_common(12):
        print(f"  {n:6} : {s[:110]}")


if __name__ == "__main__":
    main()

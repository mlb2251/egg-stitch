"""Library for parsing a binary-AIGER (.aig) circuit and extracting an
**input-bounded** fan-in cone for every AND gate: grow each cone until taking the
next gate would exceed K distinct input signals (a k-feasible cut), rather than
cutting at a fixed depth. Because the AIG is a DAG, forcing a cut to a tree
re-expands reconvergent subgates, so a node-size CAP guards against blow-up.
Leaves are named signal ids (`sN`) so abstraction can introduce metavars over
inputs. Imported by `aig_to_egg.py` and `build_benchmarks.py`.
"""

CAP = 400  # max tree nodes before we stop growing a cone (reconvergence guard)


def read_aig(path):
    """Parse a binary-AIGER file into (M, I, L, O, A, outputs, ands), where
    `ands` maps each AND-gate output literal to its two input literals."""
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


def kcone(root, ands, I, K, cap=CAP):
    """Greedy k-feasible cut as a mutable tree (nested lists). Repeatedly expand
    the frontier gate with the largest literal whose expansion keeps the cut's
    support (distinct input vars) <= K; freely unwrap negations (they cost no
    inputs). Stop at <= K inputs or when the tree reaches `cap` nodes."""
    tree = ["leaf", root]
    nodes = 1

    def leaves(node, acc):
        if node[0] == "leaf":
            acc.append(node)
        elif node[0] == "not":
            leaves(node[1], acc)
        else:
            leaves(node[1], acc); leaves(node[2], acc)
        return acc

    while nodes <= cap:
        ls = leaves(tree, [])
        if any(n[1] & 1 for n in ls):                     # unwrap negations first
            for n in ls:
                if n[1] & 1:
                    n[:] = ["not", ["leaf", n[1] ^ 1]]; nodes += 1
            continue
        best = None
        for n in ls:
            lit = n[1]
            if (lit >> 1) > I and lit in ands:            # an expandable gate leaf
                r0, r1 = ands[lit]
                others = [m[1] for m in ls if m is not n]
                vs = {x >> 1 for x in others + [r0, r1] if x > 1}
                if len(vs) <= K and (best is None or lit > best[0]):
                    best = (lit, n, r0, r1)
        if best is None:
            break
        _, n, r0, r1 = best
        n[:] = ["and", ["leaf", r0], ["leaf", r1]]; nodes += 2
    return tree


def to_tup(node, named):
    """Convert a `kcone` tree to the tuple form used downstream: ('and', a, b),
    ('not', a), ('s<var>',) / ('LEAF',) for leaves, ('C0',) for constant."""
    if node[0] == "leaf":
        lit = node[1]
        return ("C0",) if lit == 0 else ((("s%d" % (lit >> 1)),) if named else ("LEAF",))
    if node[0] == "not":
        return ("not", to_tup(node[1], named))
    return ("and", to_tup(node[1], named), to_tup(node[2], named))


def size(t):
    """Node count of a tuple tree."""
    return 1 if len(t) == 1 else 1 + sum(size(c) for c in t[1:])

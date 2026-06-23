#!/usr/bin/env python3
"""Encode a Mixed Boolean-Arithmetic (MBA) obfuscation corpus into the egg-stitch
op-children domain.

Source: the SiMBA datasets (`e1_2vars`, `e1_3vars`, ...) from
  Reichenwallner & Meerwald-Stadler, "Efficient Deobfuscation of Linear Mixed
  Boolean-Arithmetic Expressions", CheckMATE 2022
  (github.com/DenuvoSoftwareSolutions/SiMBA, datasets/).
Each line is `obfuscated_expr, ground_truth`. These are many syntactically
*disjoint* obfuscations of the same simple expression (the 2-var set is all
forms of `x+y`), authored by an obfuscator that rewrites through the MBA algebra
-- so the shared structure is latent behind a non-trivial equivalence, which is
what the live-vs-at-start experiment needs.

Encoding: an infix parser (C precedence: ~/unary- , * , +/- , & , ^ , |) emits
op-children trees `(+ a b)`,`(* a b)`,`(- a b)`,`(& a b)`,`(| a b)`,`(^ a b)`,
`(~ a)`,`(neg a)`, with variables/constants as leaves. Filtered to moderate size
(6-40 nodes) and deduplicated; the frozen `data/domains/mba/mba.json` (400 exprs,
from e1_2vars + e1_3vars) is what the experiment uses.

Usage: python3 scripts/mba_corpus.py <src1> [<src2> ...] [--n 400]"""
import json, os, re, sys

def tokenize(s):
    return re.findall(r"\d+|[A-Za-z_]\w*|[~()&|^*+\-]", s)

class P:
    def __init__(s, t): s.t = t; s.i = 0
    def peek(s): return s.t[s.i] if s.i < len(s.t) else None
    def eat(s): c = s.t[s.i]; s.i += 1; return c
    def por(s):
        a = s.pxor()
        while s.peek() == "|": s.eat(); a = ("|", a, s.pxor())
        return a
    def pxor(s):
        a = s.pand()
        while s.peek() == "^": s.eat(); a = ("^", a, s.pand())
        return a
    def pand(s):
        a = s.padd()
        while s.peek() == "&": s.eat(); a = ("&", a, s.padd())
        return a
    def padd(s):
        a = s.pmul()
        while s.peek() in ("+", "-"): op = s.eat(); a = (op, a, s.pmul())
        return a
    def pmul(s):
        a = s.pun()
        while s.peek() == "*": s.eat(); a = ("*", a, s.pun())
        return a
    def pun(s):
        c = s.peek()
        if c == "~": s.eat(); return ("~", s.pun())
        if c == "-": s.eat(); return ("neg", s.pun())
        return s.patom()
    def patom(s):
        c = s.eat()
        if c == "(":
            a = s.por()
            if s.peek() != ")": raise ValueError("unbalanced")
            s.eat(); return a
        return ("lit", c)            # number or identifier

def emit(n):
    if n[0] == "lit": return n[1]
    if n[0] == "neg": return f"(neg {emit(n[1])})"
    if n[0] == "~": return f"(~ {emit(n[1])})"
    return f"({n[0]} {emit(n[1])} {emit(n[2])})"
def size(n): return 1 if n[0] == "lit" else 1 + sum(size(c) for c in n[1:])

args = [a for a in sys.argv[1:] if a != "--n"]
N = 400
if "--n" in sys.argv:
    N = int(sys.argv[sys.argv.index("--n") + 1]); args = args[:-1]
srcs = args or [os.path.join("datasets", "e1_2vars.txt")]
OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                   "data", "domains", "mba", "mba.json")

progs = []
for src in srcs:
    for line in open(src):
        line = line.strip()
        if not line or line.startswith("#"): continue
        obf = line.split(",")[0].strip()
        try:
            p = P(tokenize(obf)); tree = p.por()
            if p.i != len(p.t): continue
            progs.append((size(tree), emit(tree)))
        except Exception: pass
progs.sort()
seen = set(); uniq = []
for sz, e in progs:
    if 6 <= sz <= 40 and e not in seen: seen.add(e); uniq.append(e)
json.dump(uniq[:N], open(OUT, "w"), indent=0)
print(f"parsed {len(progs)}; moderate(6-40) uniq {len(uniq)}; wrote {min(N, len(uniq))} -> {OUT}")
for e in uniq[:4]: print("  ", e[:120])

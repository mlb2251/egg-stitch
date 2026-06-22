#!/usr/bin/env python3
"""Option 1: learn co-structure between a function F and its derivative f=dF/dx.

The poly-deriv corpus only kept the already-computed derivative, so abstraction
over it finds nothing but a polynomial basis. Here we instead build *paired*
programs `(lam (dpair F f))` where F is a randomly sampled elementary function
(sympy) and f is its symbolic derivative. Now a single abstraction can span
both halves, so recurring differentiation structure — the inner term of a chain
rule, the shared factors of a product rule — can surface as a learned function.

We emit the corpus, run egg-stitch best-first with the physics DSRs, and dump
the library, flagging the abstractions whose pattern spans F and f (contains a
`dpair`) — those are the derivative-meaningful ones.

Env knobs: N (default 2500), DEPTH (2), ABSTS (12), STEPS (10000), ARITY (3),
SEED (0), TIMEOUT s (1800).
"""
import json
import os
import random
import subprocess
import sys
import tempfile

import sympy as sp

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
N = int(os.environ.get("N", 2500))
DEPTH = int(os.environ.get("DEPTH", 2))
ABSTS = int(os.environ.get("ABSTS", 12))
STEPS = int(os.environ.get("STEPS", 10000))
ARITY = int(os.environ.get("ARITY", 3))
SEED = int(os.environ.get("SEED", 0))
TIMEOUT = int(os.environ.get("TIMEOUT", 1800))

x = sp.Symbol("x")
FUNCS = [sp.sin, sp.cos, sp.exp, sp.log, sp.tan, sp.sinh, sp.tanh]
FUNC_HEAD = {sp.sin: "sin", sp.cos: "cos", sp.tan: "tan", sp.exp: "exp",
             sp.log: "ln", sp.sinh: "sinh", sp.cosh: "cosh", sp.tanh: "tanh",
             sp.asin: "asin", sp.acos: "acos", sp.atan: "atan"}

BASE_DSR = """\
add_comm: (@ (@ +. ?x) ?y) => (@ (@ +. ?y) ?x)
mul_comm: (@ (@ *. ?x) ?y) => (@ (@ *. ?y) ?x)
add_iden: (@ (@ +. ?x) 0.) => ?x
sub_iden: (@ (@ -. ?x) 0.) => ?x
mul_iden: (@ (@ *. ?x) 1.) => ?x
div_iden: (@ (@ /. ?x) 1.) => ?x
div_self: (@ (@ /. ?x) ?x) => 1.
pow_iden: (@ (@ power ?x) 1.) => ?x
"""


def num(n: sp.Expr) -> str:
    """sympy number -> egg-stitch float-literal atom (rationals as /. p. q.)."""
    if n.is_Integer:
        return f"{int(n)}."
    if n.is_Rational:
        return f"(/. {n.p}. {n.q}.)"
    return f"{float(n)}"


def fold(op: str, parts: list[str]) -> str:
    """Left-fold a list of s-exprs with a binary op head."""
    acc = parts[0]
    for p in parts[1:]:
        acc = f"({op} {acc} {p})"
    return acc


def conv(e: sp.Expr) -> str:
    """sympy expr -> egg-stitch lambda-calc s-expr over the physics float ops."""
    if e == x:
        return "$0"
    if e.is_Number:
        return num(e)
    if e.is_Add:
        return fold("+.", [conv(a) for a in e.args])
    if e.is_Mul:
        return fold("*.", [conv(a) for a in e.args])
    if e.is_Pow:
        return f"(power {conv(e.args[0])} {conv(e.args[1])})"
    if e.func in FUNC_HEAD and len(e.args) == 1:
        return f"({FUNC_HEAD[e.func]} {conv(e.args[0])})"
    raise ValueError(f"unhandled {e.func}: {e}")


def rand_expr(rnd: random.Random, depth: int) -> sp.Expr:
    """Random elementary function of x, biased toward products and compositions
    so the derivative reuses subterms (product/chain rule structure)."""
    if depth <= 0:
        return x if rnd.random() < 0.7 else sp.Integer(rnd.randint(2, 5))
    r = rnd.random()
    if r < 0.30:
        return rand_expr(rnd, depth - 1) * rand_expr(rnd, depth - 1)
    if r < 0.50:
        return rand_expr(rnd, depth - 1) + rand_expr(rnd, depth - 1)
    if r < 0.68:
        return rand_expr(rnd, depth - 1) ** rnd.randint(2, 4)
    if r < 0.88:
        return rnd.choice(FUNCS)(rand_expr(rnd, depth - 1))
    if r < 0.96:
        den = rand_expr(rnd, depth - 1) + sp.Integer(rnd.randint(1, 3))
        return rand_expr(rnd, depth - 1) / den
    return x


def gen_pairs() -> list[str]:
    """Sample N distinct `(lam (dpair F f))` programs (f = dF/dx)."""
    rnd = random.Random(SEED)
    out, seen, tries = [], set(), 0
    while len(out) < N and tries < N * 60:
        tries += 1
        try:
            F = rand_expr(rnd, DEPTH)
            if x not in F.free_symbols:
                continue
            f = sp.diff(F, x)
            if f == 0 or x not in f.free_symbols:
                continue
            prog = f"(lam (dpair {conv(F)} {conv(f)}))"
        except (ValueError, TypeError, RecursionError, ZeroDivisionError):
            continue
        if len(prog) > 600 or prog in seen:
            continue
        seen.add(prog)
        out.append(prog)
    return out


def run(corpus_path: str, mode: str):
    """Run egg-stitch best-first; return the parsed result dict or None."""
    rw = "/tmp/deriv_base.rw"
    open(rw, "w").write(BASE_DSR)
    outp = corpus_path + f".{mode}.out.json"
    cmd = [BIN, "-i", corpus_path, "--output", outp, "--search", "best-first",
           "--language", "lambda-calc", "--max-arity", str(ARITY),
           "--num-abstractions", str(ABSTS), "--num-steps", str(STEPS)]
    if mode != "nodsr":
        cmd += ["-r", rw]
    if mode == "atstart":
        cmd.append("--only-use-dsrs-at-start")
    try:
        subprocess.run(cmd, cwd=ROOT, check=True, timeout=TIMEOUT,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    except subprocess.CalledProcessError as e:
        print(f"  FAIL {mode}: {e.stderr.decode()[-400:]}", file=sys.stderr)
        return None
    except subprocess.TimeoutExpired:
        print(f"  TIMEOUT {mode}", file=sys.stderr)
        return None
    return json.load(open(outp))


def main():
    progs = gen_pairs()
    tmp = tempfile.mkdtemp(prefix="derivpairs_")
    corpus = os.path.join(tmp, "pairs.json")
    json.dump(progs, open(corpus, "w"))
    print(f"generated {len(progs)} (F, f) pairs -> {corpus}", flush=True)
    for p in progs[:4]:
        print("  ", p)
    print(f"\nconfig: {ABSTS} absts, {STEPS} steps, arity {ARITY}, physics DSRs (live)\n", flush=True)
    d = run(corpus, "live")
    if not d:
        return
    traj = d.get("cost_at_end_of_each_iter") or []
    print(f"initial_cost={d.get('initial_cost')}  final_cost={traj[-1] if traj else None}"
          f"  compression={d.get('compression_ratio')}\n")
    print("=== abstractions (★ = spans F and f, i.e. contains dpair) ===")
    for i, a in enumerate(d.get("library", [])):
        star = "★" if "dpair" in a["pattern"] else " "
        print(f"{star}[{i}] arity={a['arity']} usage={a['usage_matches']} size={a['pattern_size']}")
        print(f"      {a['pattern']}")


if __name__ == "__main__":
    main()

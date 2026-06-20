#!/usr/bin/env python3
"""Convert the derivative (integrand) column of Lample/Charton prim_bwd into an
egg-stitch corpus. Each input line is `<count>|sub Y' <f> \t <F>`; we take <f>
(the derivative of the sampled solution F) in prefix notation and emit it as a
lambda-calc s-expr `(lam <expr>)` with the single variable x -> de Bruijn $0.

Operators are mapped onto the physics float ops so the existing
physics.rewrites DSRs apply directly. Integers become float-literal atoms.
Optionally filter to polynomial/rational expressions (no transcendentals).

The output directory is suffixed with the sample size, so the
``physics-deriv-8k`` benchmark corpus (data/domains/lample-deriv-poly-8k/) is
reproduced exactly by::

    python scripts/lample_convert.py <prim_bwd_partial> 8000 --poly

where ``<prim_bwd_partial>`` is a raw Lample/Charton prim_bwd data shard. The
seed-0 shuffle is deterministic, so a given (size, --poly) always yields the
same corpus.
"""
import json
import os
import random
import sys

BIN_UNARY = {  # Lample op -> (sexpr head, arity)
    "add": ("+.", 2), "sub": ("-.", 2), "mul": ("*.", 2),
    "div": ("/.", 2), "pow": ("power", 2),
    "sin": ("sin", 1), "cos": ("cos", 1), "tan": ("tan", 1),
    "exp": ("exp", 1), "ln": ("ln", 1), "sqrt": ("sqrt", 1),
    "sinh": ("sinh", 1), "cosh": ("cosh", 1), "tanh": ("tanh", 1),
    "asin": ("asin", 1), "acos": ("acos", 1), "atan": ("atan", 1),
    "asinh": ("asinh", 1), "acosh": ("acosh", 1), "atanh": ("atanh", 1),
    "abs": ("abs", 1), "inv": ("inv", 1), "neg": ("neg", 1),
}
TRANSC = {"sin", "cos", "tan", "exp", "ln", "sqrt", "sinh", "cosh", "tanh",
          "asin", "acos", "atan", "asinh", "acosh", "atanh"}
DIGITS = set("0123456789")


def parse(toks, i):
    """Recursive prefix parse; returns (sexpr_str, next_index, has_transc)."""
    t = toks[i]
    if t in ("INT+", "INT-"):
        i += 1
        digs = ""
        while i < len(toks) and toks[i] in DIGITS:
            digs += toks[i]
            i += 1
        val = (digs or "0")
        return (f"{'-' if t == 'INT-' else ''}{val}.", i, False)
    if t == "x":
        return ("$0", i + 1, False)
    if t in BIN_UNARY:
        head, arity = BIN_UNARY[t]
        i += 1
        args, ht = [], (t in TRANSC)
        for _ in range(arity):
            s, i, h = parse(toks, i)
            args.append(s)
            ht = ht or h
        return (f"({head} {' '.join(args)})", i, ht)
    raise ValueError(f"unknown token {t!r}")


def convert(partial_path, n_sample, poly_only, seed=0):
    """Sample n lines, return list of distinct (lam ...) program strings."""
    lines = [l for l in open(partial_path, errors="ignore") if "\t" in l and "|" in l]
    rnd = random.Random(seed)
    rnd.shuffle(lines)
    out, seen = [], set()
    for line in lines:
        inp = line.split("\t")[0].split("|", 1)[1].split()
        if inp[:2] == ["sub", "Y'"]:
            inp = inp[2:]
        else:
            continue
        try:
            s, j, ht = parse(inp, 0)
        except (ValueError, IndexError):
            continue
        if j != len(inp):
            continue
        if poly_only and ht:
            continue
        prog = f"(lam {s})"
        if prog not in seen:
            seen.add(prog)
            out.append(prog)
        if len(out) >= n_sample:
            break
    return out


if __name__ == "__main__":
    partial = sys.argv[1]
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 8000
    poly = "--poly" in sys.argv
    progs = convert(partial, n, poly)
    # Size-suffix the corpus dir so it's self-documenting and the benchmark
    # path (e.g. lample-deriv-poly-8k) is reproducible from the invocation.
    size_tag = f"{n // 1000}k" if n % 1000 == 0 else str(n)
    base = "lample-deriv-poly" if poly else "lample-deriv"
    tag = f"{base}-{size_tag}"
    out_dir = os.path.join(os.path.dirname(__file__), "..", "data", "domains", tag)
    os.makedirs(out_dir, exist_ok=True)
    json.dump(progs, open(os.path.join(out_dir, "all.json"), "w"))
    print(f"wrote {len(progs)} programs to {tag}/all.json")
    for p in progs[:6]:
        print(" ", p[:120])

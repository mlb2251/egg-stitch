#!/usr/bin/env python3
"""Fully unfold DreamCoder inventions in the compression benchmark.

The raw benches in ../compression_benchmark/benches carry every learned
invention inline as `#(lambda ...)`. Stripping the `#` marker turns each
invention into an ordinary lambda; beta-normalizing the program then inlines
them all, leaving only base primitives + genuine argument-lambdas.

We keep all top-K programs per task (near-duplicate alternates are fine) and
union across every iteration of a run, then dedup the unfolded forms — so the
cumulative earlier-iteration subsets collapse rather than being re-counted.
Output is one flat egg-stitch-style list per run.
"""
import json
import os
import sys
import glob

BENCH_DIR = os.path.expanduser("~/mit/compression_benchmark/benches")
OUT_ROOT = os.path.join(os.path.dirname(__file__), "..", "data", "domains")


# ---- parse DreamCoder s-expressions (with `#(...)` inventions) ----
def tokenize(s):
    """Tokenize, treating `#(...)` as a plain `(...)` and `'x'` as one atom."""
    s = s.replace("#(", "(")
    toks, i, n = [], 0, len(s)
    while i < n:
        c = s[i]
        if c in " \t\n":
            i += 1
        elif c in "()":
            toks.append(c)
            i += 1
        elif c == "'":  # char literal: scan to closing quote
            j = i + 1
            while j < n and s[j] != "'":
                j += 1
            toks.append(s[i : j + 1])
            i = j + 1
        else:
            j = i
            while j < n and s[j] not in " \t\n()":
                j += 1
            toks.append(s[i:j])
            i = j
    return toks


def parse(toks):
    """Tokens -> nested lists / atom strings."""
    pos = [0]

    def go():
        t = toks[pos[0]]
        if t == "(":
            pos[0] += 1
            lst = []
            while toks[pos[0]] != ")":
                lst.append(go())
            pos[0] += 1
            return lst
        pos[0] += 1
        return t

    return go()


def to_ast(n):
    """Nested form -> de Bruijn AST: ('var',i)|('abs',b)|('app',f,x)|('prim',s)."""
    if isinstance(n, str):
        if n.startswith("$") and n[1:].isdigit():
            return ("var", int(n[1:]))
        return ("prim", n)
    if n[0] == "lambda":
        return ("abs", to_ast(n[1]))
    ast = to_ast(n[0])
    for child in n[1:]:
        ast = ("app", ast, to_ast(child))
    return ast


# ---- de Bruijn beta-normalization ----
def shift(d, c, t):
    k = t[0]
    if k == "var":
        return ("var", t[1] + d) if t[1] >= c else t
    if k == "abs":
        return ("abs", shift(d, c + 1, t[1]))
    if k == "app":
        return ("app", shift(d, c, t[1]), shift(d, c, t[2]))
    return t


def subst(j, s, t):
    k = t[0]
    if k == "var":
        return s if t[1] == j else t
    if k == "abs":
        return ("abs", subst(j + 1, shift(1, 0, s), t[1]))
    if k == "app":
        return ("app", subst(j, s, t[1]), subst(j, s, t[2]))
    return t


def beta_once(t):
    """One leftmost-outermost reduction; returns (term, changed)."""
    k = t[0]
    if k == "app":
        f, x = t[1], t[2]
        if f[0] == "abs":
            return shift(-1, 0, subst(0, shift(1, 0, x), f[1])), True
        nf, ch = beta_once(f)
        if ch:
            return ("app", nf, x), True
        nx, ch = beta_once(x)
        if ch:
            return ("app", f, nx), True
        return t, False
    if k == "abs":
        nb, ch = beta_once(t[1])
        return (("abs", nb), True) if ch else (t, False)
    return t, False


def normalize(t):
    while True:
        t, ch = beta_once(t)
        if not ch:
            return t


# ---- print back to egg-stitch surface syntax (`lam`, flattened apps) ----
def pp(t):
    k = t[0]
    if k == "var":
        return f"${t[1]}"
    if k == "prim":
        return t[1]
    if k == "abs":
        return f"(lam {pp(t[1])})"
    spine, cur = [], t
    while cur[0] == "app":
        spine.append(cur[2])
        cur = cur[1]
    spine.append(cur)
    spine.reverse()
    return "(" + " ".join(pp(s) for s in spine) + ")"


def unfold(prog):
    """DreamCoder program string -> fully-unfolded egg-stitch program string."""
    return pp(normalize(to_ast(parse(tokenize(prog)))))


def task_programs(bench_json):
    """All top-K programs of every task frontier."""
    out = []
    for task in bench_json["frontiers"]:
        for p in task["programs"]:
            out.append(p["program"] if isinstance(p, dict) else p)
    return out


def process_run(run_dir):
    """Union all top-K programs across all iterations, unfold, dedup (first-seen)."""
    seen, out = set(), []
    for f in sorted(glob.glob(os.path.join(run_dir, "bench*.json"))):
        for raw in task_programs(json.load(open(f))):
            try:
                u = unfold(raw)
            except Exception as e:
                print(f"  WARN unfold failed in {os.path.basename(run_dir)}: {e}\n    {raw[:120]}", file=sys.stderr)
                continue
            if u not in seen:
                seen.add(u)
                out.append(u)
    return out


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else None  # optional domain filter
    runs = sorted(glob.glob(os.path.join(BENCH_DIR, "*")))
    for run_dir in runs:
        if not os.path.isdir(run_dir):
            continue
        name = os.path.basename(run_dir)
        domain = name.split("_", 1)[0]
        if only and domain != only:
            continue
        progs = process_run(run_dir)
        out_dir = os.path.join(OUT_ROOT, f"{domain}-unfolded")
        os.makedirs(out_dir, exist_ok=True)
        run_id = name.split("_", 1)[1]  # drop leading "<domain>_"
        out_path = os.path.join(out_dir, f"{run_id}.json")
        json.dump(progs, open(out_path, "w"), indent=1)
        print(f"{domain:8} {run_id:50} {len(progs):5} programs")


if __name__ == "__main__":
    main()

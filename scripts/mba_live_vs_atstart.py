#!/usr/bin/env python3
"""The MBA (Mixed Boolean-Arithmetic) live-vs-at-start abstraction experiment.

Runs egg-stitch best-first three ways on the SiMBA-derived MBA corpus
(data/domains/mba/mba.json) under the MBA algebra (data/domains/mba/mba.rewrites):

  * baseline  -- no rewrite rules
  * at-start  -- rules applied once up front (--only-use-dsrs-at-start): each
                 program is collapsed to a single min-size term, then a rule-free
                 e-graph is searched
  * live      -- rules kept live during/between abstraction rounds

with --max-arity 2 --no-zero-arity (the arity cap is a meaningfulness filter --
the recognizable MBA idioms are arity 1-2, higher-arity ones are syntactic glue).
The boolean-distribution rules are expansive, so the e-graph is bounded with
--node-limit and --iter-limit 2 (both well below where results change).

Prints the per-step cumulative cost and compression ratio for all three modes,
then the abstractions live vs at-start find -- rendered back to infix MBA.

Headline: at-start *worsens* the corpus (collapsing to a min-size term destroys
the cross-program alignment); only keeping the algebra live recovers it and beats
baseline. Live learns the linear-MBA basis `a*(x^y) + b*(x&y)` as its #1, highest-
reuse idiom; at-start only finds its single-coefficient fragments. Requires
`cargo build --release` first.
"""
import json, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
D = os.path.join(ROOT, "data", "domains", "mba")
INPUT = os.path.join(D, "mba.json")
RULES = os.path.join(D, "mba.rewrites")
NABST, NSTEPS, ARITY, ITERLIM, NODELIM = "20", "3000", "2", "2", "100000"

def run(label, extra):
    if not os.path.exists(BIN):
        sys.exit("missing %s -- run `cargo build --release` first" % BIN)
    out = "/tmp/mba_%s.json" % label
    cmd = [BIN, "-i", INPUT, "--output", out, "--search", "best-first",
           "--max-arity", ARITY, "--no-zero-arity",
           "--num-abstractions", NABST, "--num-steps", NSTEPS] + extra
    subprocess.run(cmd, cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    return json.load(open(out))

# ---- expand an abstraction (inline nested fns) and render as infix MBA ----
def tok(s): return re.findall(r"\(|\)|[^\s()]+", s)
def parse(ts):
    pos = 0
    def rd():
        nonlocal pos
        t = ts[pos]; pos += 1
        if t == "(":
            l = []
            while ts[pos] != ")": l.append(rd())
            pos += 1; return l
        return t
    return rd()
BINOP = {"+": "+", "-": "-", "*": "*", "&": "&", "|": "|", "^": "^"}
def make_expand(d):
    body = {e["pattern"].split(": ", 1)[0]: e["pattern"].split(": ", 1)[1] for e in d["library"]}
    def expand(n, a):
        if isinstance(n, str):
            if n.startswith("?#") or n.startswith("#"):
                i = int(re.sub(r"\D", "", n)); return a[i] if i < len(a) else "#%d" % i
            if n in body: return expand(parse(tok(body[n])), [])
            return n
        h = n[0]
        if h in body: return expand(parse(tok(body[h])), [expand(c, a) for c in n[1:]])
        return [h] + [expand(c, a) for c in n[1:]]
    return body, expand
def infix(n):
    if isinstance(n, str): return n
    h = n[0]
    if h == "~": return "~" + infix(n[1])
    if h == "neg": return "-" + infix(n[1])
    if h in BINOP: return "(" + infix(n[1]) + BINOP[h] + infix(n[2]) + ")"
    return "(" + h + " " + " ".join(infix(c) for c in n[1:]) + ")"
def rows(d):
    body, expand = make_expand(d); c = d["cost_at_end_of_each_iter"]; out = []
    for i, e in enumerate(d["library"]):
        nm = e["pattern"].split(": ", 1)[0]; ar = e.get("arity", 0)
        ex = expand(parse(tok(body[nm])), ["#%d" % k for k in range(ar)])
        out.append((ar, e.get("num_matches"), c[i], infix(ex)))
    return out

print("running baseline / at-start / live (best-first, --max-arity 2 --no-zero-arity)...\n")
base = run("baseline", [])
atst = run("atstart", ["-r", RULES, "--iter-limit", ITERLIM, "--node-limit", NODELIM, "--only-use-dsrs-at-start"])
live = run("live", ["-r", RULES, "--iter-limit", ITERLIM, "--node-limit", NODELIM])
i0 = base["initial_cost"]
b, a, l = (d["cost_at_end_of_each_iter"] for d in (base, atst, live))

print("initial cost = %d\n" % i0)
print("step |  baseline      at-start        live          (cost / ratio)")
print("-----+--------------------------------------------------------------")
for k in range(len(b)):
    print("%4d | %5d %.3fx  %5d %.3fx  %5d %.3fx" %
          (k + 1, b[k], i0 / b[k], a[k], i0 / a[k], l[k], i0 / l[k]))
print("\nfinal:  baseline %.3fx   at-start %.3fx   live %.3fx" % (i0 / b[-1], i0 / a[-1], i0 / l[-1]))
print("live gap vs at-start: %d   vs baseline: %d\n" % (a[-1] - l[-1], b[-1] - l[-1]))

L, A = rows(live), rows(atst)
print("abstractions  ( cost · m=reuse · infix MBA, #k = parameter ):\n")
for name, R in (("LIVE", L), ("AT-START", A)):
    print("==== %s ====" % name)
    for ar, m, c, s in R:
        print("  ar%d m=%-4s c=%-5s %s" % (ar, m, c, s))
    print()

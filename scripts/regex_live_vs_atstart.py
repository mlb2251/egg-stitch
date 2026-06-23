#!/usr/bin/env python3
"""The regex live-vs-at-start abstraction experiment.

Runs egg-stitch best-first three ways on the RegExLib corpus
(data/domains/regex/regexlib.json) under the regex algebra
(data/domains/regex/regexlib.rewrites):

  * baseline  -- no rewrite rules
  * at-start  -- rules applied once up front (--only-use-dsrs-at-start)
  * live      -- rules kept live during/between abstraction rounds

with --max-arity 2 --no-zero-arity (abstractions must have 1-2 parameters: the
arity cap acts as a "meaningfulness filter" -- the higher-arity abstractions are
mostly syntactic concat/alt glue, while the meaningful idioms are arity 1-2).

Prints, per step, the abstraction each mode discovers -- fully expanded to an
actual regex (holes shown as #k) and flagged meaningful (contains real content:
a character class / shorthand / literal / concrete quantifier) vs syntactic --
plus the cumulative cost gap and the meaningful-abstraction counts.

Headline: live finds the meaningful, shared idioms (digit fields, letter/
alphanumeric classes, day-of-month, ...) and leads in cost at every step; the
gap grows under the arity cap because the high-arity syntactic skeletons that
both modes shared are removed, exposing live's advantage on the low-arity
meaningful ones.  Requires `cargo build --release` first.
"""
import json, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
D = os.path.join(ROOT, "data", "domains", "regex")
INPUT = os.path.join(D, "regexlib.json")
RULES = os.path.join(D, "regexlib.rewrites")
NABST, NSTEPS, ARITY, ITERLIM = "20", "3000", "2", "2"

def run(label, extra):
    if not os.path.exists(BIN):
        sys.exit("missing %s -- run `cargo build --release` first" % BIN)
    out = "/tmp/regexlib_%s.json" % label
    cmd = [BIN, "-i", INPUT, "--output", out, "--search", "best-first",
           "--max-arity", ARITY, "--no-zero-arity",
           "--num-abstractions", NABST, "--num-steps", NSTEPS] + extra
    subprocess.run(cmd, cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    return json.load(open(out))

# ---- expand an abstraction (inline nested fns), render as a regex, classify ----
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
def make_helpers(d):
    body = {e["pattern"].split(": ", 1)[0]: e["pattern"].split(": ", 1)[1] for e in d["library"]}
    def expand(n, a):
        if isinstance(n, str):
            if "#" in n:
                i = int(re.sub(r"\D", "", n)); return a[i] if i < len(a) else n
            if n in body: return expand(parse(tok(body[n])), [])
            return n
        h = n[0]
        if h in body: return expand(parse(tok(body[h])), [expand(c, a) for c in n[1:]])
        return [h] + [expand(c, a) for c in n[1:]]
    return body, expand
dec = lambda t: t.replace("%20", " ").replace("%28", "(").replace("%29", ")").replace("%22", '"').replace("%25", "%")
def wrap(n): return rx(n) if (isinstance(n, str) or n[0] in ("lit", "Alt")) else "(?:" + rx(n) + ")"
def rx(n):
    if isinstance(n, str): return n if "#" in n else dec(n)
    h = n[0]
    if h == "Eps": return ""
    if h == "Cat": return rx(n[1]) + rx(n[2])
    if h == "Alt": return "(" + rx(n[1]) + "|" + rx(n[2]) + ")"
    if h in ("Star", "Plus", "Opt"): return wrap(n[1]) + {"Star": "*", "Plus": "+", "Opt": "?"}[h]
    if h == "Range":
        lo, hi = n[2], n[3]
        q = "{%s}" % lo if lo == hi else ("{%s,}" % lo if hi == "INF" else "{%s,%s}" % (lo, hi))
        return wrap(n[1]) + q
    return str(n)
def meaningful(n):
    if isinstance(n, str): return ("#" not in n) and n not in ("^", "$", "Eps")
    if n[0] == "Range" and (("#" not in n[2]) or ("#" not in n[3])): return True
    return any(meaningful(c) for c in n[1:] if isinstance(c, (list, str)))
def rows(d):
    body, expand = make_helpers(d); c = d["cost_at_end_of_each_iter"]; out = []
    for i, e in enumerate(d["library"]):
        ar = e.get("arity", 0)
        ex = expand(parse(tok(body[e["pattern"].split(': ', 1)[0]])), ["#%d" % k for k in range(ar)])
        out.append((c[i], e.get("num_matches"), meaningful(ex), rx(ex)))
    return out

print("running baseline / at-start / live (best-first, --max-arity 2 --no-zero-arity)...\n")
base = run("baseline", [])
atst = run("atstart", ["-r", RULES, "--iter-limit", ITERLIM, "--only-use-dsrs-at-start"])
live = run("live", ["-r", RULES, "--iter-limit", ITERLIM])
fin = lambda d: d["cost_at_end_of_each_iter"][-1]
print("initial=%s  baseline=%s  at-start=%s  live=%s  (live gap vs at-start: %s)\n"
      % (base["initial_cost"], fin(base), fin(atst), fin(live), fin(atst) - fin(live)))

L, A = rows(live), rows(atst)
print("per step  ( ★=meaningful · cost · m=matches · abstraction ):\n")
for i in range(max(len(L), len(A))):
    lc, lm, lmn, lp = L[i] if i < len(L) else ("", "", False, "")
    ac, am, amn, ap = A[i] if i < len(A) else ("", "", False, "")
    print("step %2d" % (i + 1))
    print("   LIVE     %s %-5s m=%-4s %s" % ("★" if lmn else " ", lc, lm, lp[:60]))
    print("   AT-START %s %-5s m=%-4s %s" % ("★" if amn else " ", ac, am, ap[:60]))
print("\nmeaningful abstractions:  live %d/%d   at-start %d/%d"
      % (sum(r[2] for r in L), len(L), sum(r[2] for r in A), len(A)))

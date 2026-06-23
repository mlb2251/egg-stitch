#!/usr/bin/env python3
"""Reproduce the regex live-vs-at-start abstraction experiment.

Runs egg-stitch best-first three ways on the frozen regex corpus
(data/domains/regex/regex.json) and reports, side by side, the per-step
compression and the abstraction discovered at each step:

  * baseline  -- no rewrite rules
  * at-start  -- regex algebra applied once up front (--only-use-dsrs-at-start)
  * live      -- regex algebra kept live during/between abstraction rounds

Headline finding: live and at-start reach similar *total* compression (the bulk
is generic Cat/Alt skeletons both find), but at every step live extracts a
more-shared, often content-bearing idiom, and surfaces shared content idioms
(e.g. a delimiter-parameterized genomic-locus matcher `(X|Y|M|\\d+) S \\d+ S
[ACGTN]* S [ACGTN]*`) that at-start only ever bakes as one-off singletons.

Requires the release binary: `cargo build --release` first.
"""
import json, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
D = os.path.join(ROOT, "data", "domains", "regex")
INPUT = os.path.join(D, "regex.json")
RULES = os.path.join(D, "regex.rewrites")
LEG = json.load(open(os.path.join(D, "regex_legend.json")))   # Ln -> token

NABST, NSTEPS, ARITY, ITERLIM = "15", "3000", "4", "3"

def run(label, extra):
    if not os.path.exists(BIN):
        sys.exit(f"missing {BIN} -- run `cargo build --release` first")
    out = f"/tmp/regex_{label}.json"
    cmd = [BIN, "-i", INPUT, "--output", out, "--search", "best-first",
           "--max-arity", ARITY, "--num-abstractions", NABST, "--num-steps", NSTEPS] + extra
    subprocess.run(cmd, cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
    return json.load(open(out))

def decode(pat, w=70):
    b = pat.split(": ", 1)[-1].replace("?#", "#")
    b = re.sub(r"L\d+", lambda m: LEG.get(m.group(0), m.group(0)), b)
    return b if len(b) <= w else b[:w-1] + "…"

print("running baseline / at-start / live (best-first, 15 abstractions)…\n")
base = run("baseline", [])
atst = run("atstart", ["-r", RULES, "--iter-limit", ITERLIM, "--only-use-dsrs-at-start"])
live = run("live", ["-r", RULES, "--iter-limit", ITERLIM])

def final(d): return d["cost_at_end_of_each_iter"][-1]
print(f"initial cost      : {base['initial_cost']}")
print(f"baseline (no rules): {final(base)}")
print(f"at-start           : {final(atst)}")
print(f"live               : {final(live)}   (gap vs at-start: {final(atst)-final(live)})\n")

def rows(d):
    c, lib = d["cost_at_end_of_each_iter"], d["library"]
    return [(c[i], lib[i].get("num_matches"), decode(lib[i]["pattern"])) for i in range(len(lib))]
L, A = rows(live), rows(atst)
print("per-step (cost | matches | abstraction):\n")
for i in range(max(len(L), len(A))):
    lc, lm, lp = L[i] if i < len(L) else ("", "", "")
    ac, am, ap = A[i] if i < len(A) else ("", "", "")
    print(f"step {i+1:>2}")
    print(f"   LIVE     {lc:<5} m={str(lm):>4}  {lp}")
    print(f"   AT-START {ac:<5} m={str(am):>4}  {ap}")

def content(d):
    return [(e.get("num_matches"), decode(e["pattern"], 120)) for e in d["library"] if "'" in decode(e["pattern"], 999)]
print("\ncontent-bearing (idiom) abstractions, live vs at-start:")
for label, d in [("LIVE", live), ("AT-START", atst)]:
    cs = content(d); sh = [x for x in cs if (x[0] or 0) > 1]
    print(f"  {label}: {len(cs)} content-bearing, {len(sh)} shared (m>1)")
    for m, s in sorted(cs, reverse=True)[:6]:
        print(f"      m={m:>3} {s}")

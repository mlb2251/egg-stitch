#!/usr/bin/env python3
"""Regression test for the AIG multiplier live-DSR experiment.

Locks three things against the results established in the investigation:
  1. AC-census on multiplier.aig (non-canonical-structure metric).
  2. Corpus regenerates byte-identically from the committed .aig.
  3. A fixed 4-abstraction SMC rollout (seed 1) for baseline / live-AC / at-start
     reproduces the exact cost trajectory and discovered abstractions.

SMC with --seed 1 is deterministic, and SMC needs no cap flag, so this test is
stable and independent of the best-first match-set-cap flag. Run:
    cargo build --release && python3 scripts/test_aig_mult_regression.py
Exits 0 on success, 1 on any mismatch.
"""
import json, os, sys, subprocess, collections

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
from aig_cones import read_aig, cone, size, sk, norm

BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
AIG = os.path.join(ROOT, "data", "domains", "mult", "multiplier.aig")
CORPUS = os.path.join(ROOT, "data", "domains", "mult", "all.json")
RULES = os.path.join(ROOT, "data", "domains", "mult", "and_ac.rewrites")
RULES_DM = os.path.join(ROOT, "data", "domains", "mult", "and_or_demorgan.rewrites")
RULES_FAC = os.path.join(ROOT, "data", "domains", "mult", "and_or_demorgan_factor.rewrites")

# ---- golden values (from the investigation; SMC seed 1, arity 4, 5000/100, temp 1000) ----
CENSUS = {"cones_ge6": 26804, "unique_raw": 1721, "unique_ac": 642}
ROLLOUT = {
    "baseline": {
        "costs": [23131, 20425, 18424, 17054],
        "fns": ["(and (not ?#0) (not ?#1))",
                "(fn_0 (and (not ?#0) ?#1) (and ?#0 (not ?#1)))",
                "(fn_0 (fn_0 ?#0 ?#1) (and (not ?#0) ?#2))",
                "(and (not ?#0) ?#1)"]},
    "live": {
        "costs": [21442, 18514, 17385, 16705],
        "fns": ["(and (not (and (not ?#0) ?#1)) (not ?#2))",
                "(and (not ?#0) ?#1)",
                "(fn_0 ?#0 ?#1 (fn_1 ?#2 ?#3))",
                "(fn_0 ?#0 (not ?#1) ?#2)"]},
    "at-start": {
        "costs": [23887, 21095, 19502, 18445],
        "fns": ["(and (not ?#0) (not ?#1))",
                "(and (not ?#0) ?#1)",
                "(and ?#0 (not ?#1))",
                "(fn_1 ?#0 (fn_0 ?#1 ?#2))"]},
}

# De Morgan ruleset (and_or_demorgan.rewrites): introduces `or` + the AND<->OR
# bridges, capturing real semantic equivalence. This is the ONLY configuration
# where live abstraction beats the no-rules baseline (12980 < 13599 at 10 absts);
# at-start does worst (16212). The 4-abstraction prefixes locked here:
ROLLOUT_DM = {
    "live-DM": {
        "costs": [18075, 15714, 15158, 14620],
        "fns": ["(and (or (not ?#0) ?#1) (or ?#0 (not ?#1)))",   # XNOR, product-of-sums
                "(and (or ?#0 (not ?#1)) ?#2)",
                "(fn_1 ?#0 ?#1 (or ?#2 ?#3))",
                "(and (or ?#0 ?#1) ?#2)"]},
    "at-start-DM": {
        "costs": [20314, 19455, 18867, 18281],
        "fns": ["(and (or ?#0 ?#1) (or ?#2 ?#3))",
                "(not (or ?#0 ?#1))",
                "(and ?#0 (not ?#1))",
                "(and ?#0 (or ?#1 ?#2))"]},
}

# De Morgan + distributivity-factoring + absorption + idempotence
# (and_or_demorgan_factor.rewrites): the size-reducing factoring direction
# collapses redundant product-of-sums (e.g. (a|~b)(a|c) -> a|(~b&c)), simplifying
# further (after-rules 22659 -> 21021) WITHOUT the blow-up of the expanding
# distributive direction. live improves to 12401 at 10 absts (best of all,
# baseline 13599); at-start 15463. The 4-abstraction prefixes locked here:
ROLLOUT_FAC = {
    "live-FAC": {
        "costs": [16495, 14717, 14106, 13701],
        "fns": ["(and (or (not ?#0) ?#1) (or (not ?#1) ?#0))",   # XNOR, product-of-sums
                "(and (or (not ?#0) ?#1) ?#2)",
                "(or (and (not ?#0) ?#1) ?#2)",
                "(and (not ?#0) (and (fn_0 ?#1 ?#2) ?#3))"]},
    "at-start-FAC": {
        "costs": [19372, 18486, 17841, 17305],
        "fns": ["(and (or ?#0 ?#1) (or ?#2 ?#3))",
                "(and (not ?#0) ?#1)",
                "(and ?#0 (not ?#1))",
                "(not (or ?#0 ?#1))"]},
}

fails = []


def check(name, got, want):
    ok = got == want
    print(f"  [{'PASS' if ok else 'FAIL'}] {name}")
    if not ok:
        print(f"        got : {got}")
        print(f"        want: {want}")
        fails.append(name)


def census():
    print("AC-census (multiplier.aig, depth 4):")
    M, I, L, O, A, outs, ands = read_aig(AIG)
    cones = [cone(2 * (I + 1 + j), ands, I, 4, named=False) for j in range(A)]
    nt = [t for t in cones if size(t) >= 6]
    raw = {sk(t) for t in nt}
    acn = {sk(norm(t)) for t in nt}
    check("non-trivial cones (>=6 nodes)", len(nt), CENSUS["cones_ge6"])
    check("unique raw structures", len(raw), CENSUS["unique_raw"])
    check("unique AC+notnot normalized", len(acn), CENSUS["unique_ac"])


def corpus_regen():
    print("Corpus provenance:")
    committed = json.load(open(CORPUS))
    out = subprocess.run([sys.executable, os.path.join(ROOT, "scripts", "aig_to_egg.py")],
                         cwd=ROOT, capture_output=True, text=True)
    regen = json.load(open(CORPUS))
    json.dump(committed, open(CORPUS, "w"), indent=0)  # restore (regen is identical anyway)
    check("all.json regenerates from multiplier.aig", regen, committed)
    check("corpus size", len(committed), 800)


def rollout(name, at_start, rules, golden):
    out = os.path.join("/tmp", f"reg_{name}.json")
    cmd = [BIN, "-i", CORPUS, "--output", out, "--search", "smc",
           "--language", "op-children", "--max-arity", "4", "--num-abstractions", "4",
           "--num-particles", "5000", "--num-steps", "100", "--temperature", "1000",
           "--seed", "1", "--iter-limit", "30"]
    if at_start:
        cmd.append("--only-use-dsrs-at-start")
    if rules is not None:
        cmd += ["-r", rules]
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True)
    if p.returncode != 0:
        check(f"{name} run", f"rc={p.returncode}", "rc=0")
        return
    d = json.load(open(out))
    check(f"{name} costs", d["cost_at_end_of_each_iter"], golden["costs"])
    check(f"{name} abstractions",
          [a["pattern"].split(": ", 1)[1] for a in d["library"]], golden["fns"])


def main():
    if not os.path.exists(BIN):
        print(f"ERROR: {BIN} not found — run `cargo build --release` first.")
        sys.exit(2)
    census()
    corpus_regen()
    print("4-abstraction SMC rollout, AC rules (seed 1):")
    rollout("baseline", False, None, ROLLOUT["baseline"])
    rollout("live", False, RULES, ROLLOUT["live"])
    rollout("at-start", True, RULES, ROLLOUT["at-start"])
    print("4-abstraction SMC rollout, De Morgan rules (seed 1):")
    rollout("live-DM", False, RULES_DM, ROLLOUT_DM["live-DM"])
    rollout("at-start-DM", True, RULES_DM, ROLLOUT_DM["at-start-DM"])
    print("4-abstraction SMC rollout, De Morgan + factoring rules (seed 1):")
    rollout("live-FAC", False, RULES_FAC, ROLLOUT_FAC["live-FAC"])
    rollout("at-start-FAC", True, RULES_FAC, ROLLOUT_FAC["at-start-FAC"])
    print()
    if fails:
        print(f"REGRESSION FAILED: {len(fails)} check(s) — {fails}")
        sys.exit(1)
    print("ALL CHECKS PASSED")


if __name__ == "__main__":
    main()

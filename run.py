#!/usr/bin/env python3
"""Run a named experiment from the README. Usage: ./run.py <name>"""

import sys
import json
from expts import *


def dials_compress():
    egg_stitch(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        num_steps=10,
        num_particles=100,
        debug_log=False,
    )


def dials_follow():
    egg_stitch(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        num_steps=10,
        num_particles=100,
        debug_log=False,
        follow="(T (T (T l (M 1 0 -0.5 0)) (M #0 (/ pi 4) 0 0)) (M 1 0 (* #0 (* 0.5 (cos (/ pi 4)))) (* #0 (* 0.5 (sin (/ pi 4))))))",
    )


def temp_sweep():
    """Temperature sweep for SMC on dials with rewrites."""

    rows = []

    for t in [1, 10, 100, 1000, 10000]:
        rows.append(dict(
            name=f"T{t}",
            config=dict(num_steps=100, num_particles=1000, temperature=t, max_arity=2, ),
            output=None
        ))

    for row in rows:
        print(f"Running {row['name']} ===")
        row["output"] = egg_stitch(
            "data/domains/cogsci/dials.json",
            rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
            output=f"dials_{row['name']}.json",
            **row["config"],
        )

    for row in rows:
        print(f"{row['name']}:")
        res = json.load(open(row["output"]))
        print(f"  compression ratio: {res['compression_ratio']}")
        print(f"  pattern: {res['pattern']}")
    




def bf_dfs():
    """Best-first with depth-first priority."""
    egg_stitch(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        output="dials_bf_dfs.json",
        search="best-first",
        priority="depth-first",
        num_steps=500,
        debug_log=True,
        max_arity=2,
    )


def bf_bfs():
    """Best-first with breadth-first priority."""
    egg_stitch(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        output="dials_bf_bfs.json",
        search="best-first",
        priority="breadth-first",
        num_steps=500,
        debug_log=True,
        max_arity=2,
    )


def bf_matches():
    """Best-first with most-matches priority."""
    egg_stitch(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        output="dials_bf_matches.json",
        search="best-first",
        priority="most-matches",
        num_steps=500,
        debug_log=True,
        max_arity=2,
    )

def best_first():
    """Best-first with cost priority."""
    egg_stitch(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        output="dials_bf_cost.json",
        search="best-first",
        # priority="cost",
        num_steps=5000,
        # debug_log=True,
        max_arity=2,
        # replay="/Users/maddy/proj/rust/egg-stitch/viz/results/2026-04-12_17-29-35/dials_bf_cost_replay.json",
    )


def dev_best_first():
    best_first()



def best_first_all():
    for domain in ALL_DOMAINS:
        egg_stitch(
            f"data/domains/cogsci/{domain}.json",
            rewrites=None,
            output=f"{domain}_bf_cost.json",
            search="best-first",
            # priority="cost",
            num_steps=5000,
            max_arity=2,
        )


def dev():
    table1()
    # best_first()
    # egg_stitch(
    #     "data/domains/cogsci/dials.json",
    #     rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
    #     output="dials_T1000.json",
    #     num_steps=100,
    #     num_particles=1000,
    #     temperature=1000,
    #     max_arity=2,
    # )

def nuts_bolts_enum():
    """Run nuts-bolts in best-first ("enum") mode via the low-level egg_stitch()
    escape hatch, replicating exactly the command bench_pr.py builds for that
    cell (OursBf(max_forced_expansion=12) on the with-DSRs condition).

    The flags below mirror expts.run_models.ours._run for this cell:
      - op-children language  (nuts-bolts is cogsci → "no-apps" weighting)
      - max_arity 2           (expts.bench.MAX_ARITY)
      - num_abstractions 1    (rounds=1)
      - max_forced_expansion 12, no num_steps  (BF_RUNNERS["nuts-bolts"])
      - DSRs live via the drawings.nuts-bolts rewrites
    ``opt_seen`` is passed False for now so timing matches the benchmark (which
    doesn't enable the seen set); flip to True to turn on canonical-pattern dedup.
    """
    egg_stitch(
        "data/domains/cogsci/nuts-bolts-abbv.json",
        rewrites="data/domains/cogsci/nuts-bolts.rewrites",
        output="nuts_bolts_enum.json",
        search="best-first",
        language="op-children",
        max_arity=2,
        num_abstractions=1,
        max_forced_expansion=40,
        opt_seen=True,
        no_opt_useless_inline=True,
        verbose=True,
        # no_freeze_rule=True,
    )


if __name__ == "__main__":
    fn = globals().get(sys.argv[1]) if len(sys.argv) == 2 else None
    if not callable(fn):
        print(f"usage: python run.py <function_name>", file=sys.stderr)
        sys.exit(1)
    fn()

#!/usr/bin/env python3
"""Run a named experiment from the README. Usage: python run.py <name>"""

import sys
import json
from expts import compress, run_domain, runall




def all_mini():
    runall(num_steps=10, num_particles=100)

def dials_debug():
    run_domain("dials", num_steps=10, num_particles=1000, debug_log=True)


def dials_compress():
    compress(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        num_steps=10,
        num_particles=100,
        debug_log=False,
    )


def dials_follow():
    compress(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        num_steps=10,
        num_particles=100,
        debug_log=False,
        follow="(T (T (T l (M 1 0 -0.5 0)) (M #0 (/ pi 4) 0 0)) (M 1 0 (* #0 (* 0.5 (cos (/ pi 4)))) (* #0 (* 0.5 (sin (/ pi 4))))))",
    )


def dev():
    """Current expt: best-first on dials with rewrites."""
    compress(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        output="dials_bf.json",
        search="best-first",
        num_steps=500,
        debug_log=True,
        max_arity=2,
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
        row["output"] = compress(
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
    




EXPTS = {
    "all-mini": all_mini,
    "dials-debug": dials_debug,
    "dials-compress": dials_compress,
    "dials-follow": dials_follow,
    "dev": dev,
    "temp-sweep": temp_sweep,
}


if __name__ == "__main__":
    if len(sys.argv) != 2 or sys.argv[1] not in EXPTS:
        print(f"usage: python run.py <{'|'.join(EXPTS)}>", file=sys.stderr)
        sys.exit(1)
    EXPTS[sys.argv[1]]()

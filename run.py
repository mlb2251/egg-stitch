#!/usr/bin/env python3
"""Run a named experiment from the README. Usage: python run.py <name>"""

import sys
import json
from expts import compress, run_domain, runall, ALL_DOMAINS
import subprocess as sp




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
    




def bf_dfs():
    """Best-first with depth-first priority."""
    compress(
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
    compress(
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
    compress(
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
    compress(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        output="dials_bf_cost.json",
        search="best-first",
        # priority="cost",
        num_steps=5000,
        # debug_log=True,
        max_arity=2,
        # replay="/Users/maddy/proj/rust/egg-stitch/viz/results/2026-04-12_17-29-35/dials_bf_cost_replay.json",
        # samply=True,
    )


def dev_best_first():
    best_first()



def best_first_all():
    for domain in ALL_DOMAINS:
        compress(
            f"data/domains/cogsci/{domain}.json",
            rewrites=None,
            output=f"{domain}_bf_cost.json",
            search="best-first",
            # priority="cost",
            num_steps=5000,
            max_arity=2,
        )


def stitch():
    stitch_dir = "../stitch"
    relative_outfiles = []
    for domain in ALL_DOMAINS:
        name = f"{domain}"
        outfile = f"out/for-egg-stitch/{name}.json"
        relative_outfiles.append(f"{stitch_dir}/{outfile}")
        print(f"\033[92mRunning {domain}\033[0m")
        stitch_cmd = ["cargo", "run", "--release", "--bin=compress", f"data/cogsci/{domain}.json", "-i1", "-a2", "--out", outfile, "--no-curried-bodies", "--no-curried-metavars", "--silent"]
        sp.run(stitch_cmd, check=True, cwd=stitch_dir)
    
    for relative_outfile in relative_outfiles:
        with open(relative_outfile) as f:
            data = json.load(f)
        abstraction = data["abstractions"][0]
        print(f"From {relative_outfile}:")
        print("  ", abstraction["body"])
        print("  ", abstraction["arity"])
        print("  ", abstraction["compression_ratio"])

def babble():
    """Run babble on all domains, analogous to stitch()."""
    babble_dir = "../babble"
    results = []
    for domain in ALL_DOMAINS:
        outfile = f"harness/data_gen/cache/{domain}.csv"
        print(f"\033[92mRunning {domain}\033[0m")
        babble_cmd = [
            "cargo", "run", "--release", "--bin=drawings", "--",
            f"harness/data/cogsci/{domain}.bab",
            "--beams=400", "--lps=1", "--rounds=1", "--max-arity=2",
            f"--output={outfile}",
        ]
        proc = sp.run(babble_cmd, check=True, cwd=babble_dir, capture_output=True, text=True)
        # Parse library definitions from stdout ("lib <name> =\n  <body>\nin")
        libs = []
        lines = proc.stdout.splitlines()
        for i, l in enumerate(lines):
            if l.startswith("lib "):
                name = l.strip().removesuffix(" =")
                body = lines[i + 1].strip() if i + 1 < len(lines) else "?"
                libs.append(f"{name}: {body}")
        # Parse CSV for stats
        with open(f"{babble_dir}/{outfile}") as f:
            row = f.read().strip().split(",")
        # CSV fields: type,round,beams_start,beams_end,lps,?,rounds,initial_cost,final_cost,compression,num_libs,time
        results.append((domain, row, libs))

    for domain, row, libs in results:
        initial_cost, final_cost, compression, time_s = row[7], row[8], row[9], row[11]
        print(f"{domain}: {initial_cost} -> {final_cost} (compression {compression}, time {time_s}s)")
        for lib in libs:
            print(f"  {lib}")





def dev():
    best_first()
    # compress(
    #     "data/domains/cogsci/dials.json",
    #     rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
    #     output="dials_T1000.json",
    #     num_steps=100,
    #     num_particles=1000,
    #     temperature=1000,
    #     max_arity=2,
    # )




if __name__ == "__main__":
    fn = globals().get(sys.argv[1]) if len(sys.argv) == 2 else None
    if not callable(fn):
        print(f"usage: python run.py <function_name>", file=sys.stderr)
        sys.exit(1)
    fn()

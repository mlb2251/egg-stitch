#!/usr/bin/env python3
"""Run a named experiment from the README. Usage: ./run.py <name>"""

import sys
import json
from expts import ALL_DOMAINS, egg_stitch, table1, table2
from expts.stackpath import subgroup, stackpathiter
from expts.stitch import run_stitch
from expts.egg_stitch import run_ours
from expts.babble import run_babble
from expts import rewrites_path


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
        # samply=True,
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


def table2_arity_scaling(num_runs = 2):
    for max_arity in [0, 1, 2, 3]:
        with subgroup(f"max_arity={max_arity}"):
            table2(max_arity=max_arity, num_runs=num_runs)

# def extra_arities(num_runs = 2):
#     for max_arity in [4]:
#         with subgroup(f"max_arity={max_arity}"):
#             table2(max_arity=max_arity, num_runs=num_runs)

# def extra_stitch_arities(num_runs = 2):
#     for max_arity in stackpathiter([7, 8, 9]):
#         for domain in stackpathiter(ALL_DOMAINS):
#             stitch_config = dict(max_library_size=1, max_arity=max_arity)
#             run_stitch(domain, **stitch_config)

# def extra_us_arities(num_runs = 2):
#     for rep in stackpathiter(range(num_runs), lambda i: f"rep{i}"):
#         for max_arity in stackpathiter([5, 6, 7, 8, 9], lambda x: f"max_arity={x}"):
#             for domain in stackpathiter(ALL_DOMAINS):
#                 smc_config = dict(num_steps=100, max_arity=max_arity, num_particles=1000, temperature=1000., rewrites=None)
#                 enum_config = dict(num_steps=500, max_arity=max_arity, rewrites=None)
#                 with subgroup("best-first"):
#                     run_ours(domain, "best-first", **enum_config)
#                 with subgroup("smc"):
#                     run_ours(domain, "smc", **smc_config)

def sweep_num_steps_enum(num_runs = 3):
    for rep in stackpathiter(range(num_runs), lambda i: f"rep{i}"):
        for num_steps in stackpathiter([64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384]):
            for domain in stackpathiter(ALL_DOMAINS):
                enum_config = dict(num_steps=num_steps, max_arity=2, rewrites=None)
                run_ours(domain, "best-first", **enum_config)

def sweep_num_particles_smc(num_runs = 3):
    for rep in stackpathiter(range(num_runs), lambda i: f"rep{i}"):
        for num_particles in stackpathiter([4, 8, 16, 32, 64, 128, 256, 512, 1024, 2048, 4096]):
            for domain in stackpathiter(ALL_DOMAINS):
                smc_config = dict(num_steps=100, num_particles=num_particles, max_arity=2, rewrites=None)
                run_ours(domain, "smc", **smc_config)


def sweep_num_steps_smc(num_runs = 3):
    for rep in stackpathiter(range(num_runs), lambda i: f"rep{i}"):
        for num_steps in stackpathiter([4, 8, 16, 32, 64, 128, 256, 512, 1024]):
            for domain in stackpathiter(ALL_DOMAINS):
                smc_config = dict(num_steps=num_steps, num_particles=1000, max_arity=2, rewrites=None)
                run_ours(domain, "smc", **smc_config)


    # stitch_config = dict(max_library_size=1, max_arity=2)
    # smc_config = dict(num_steps=100, max_arity=2, num_particles=1000, temperature=1000., rewrites=None)
    # enum_config = dict(num_steps=500, max_arity=2, rewrites=None)
    # babble_config = dict(max_arity=2, dsr=None)

    # for domain in stackpathiter(ALL_DOMAINS):
    #     for i in stackpathiter(range(num_runs), lambda i: f"rep{i}"):
    #         with subgroup("best-first"):
    #             run_ours(domain, "best-first", **enum_config)
    #         with subgroup("smc"):
    #             run_ours(
    #                 domain, "smc", **smc_config)
    #         with subgroup("babble"):
    #             run_babble(domain, **babble_config)
    #         with subgroup("stitch"):
    #             run_stitch(domain, **stitch_config)



def quick_eval(num_runs = 3):
    results = []
    for rep in stackpathiter(range(num_runs), lambda i: f"rep{i}"):
        for domain in stackpathiter(ALL_DOMAINS):
            enum_config = dict(num_steps=500, max_arity=2, rewrites=rewrites_path(domain))
            res, _ = run_ours(domain, "best-first", **enum_config)
            results.append(dict(domain=domain, elapsed_secs=res.elapsed_secs))
    
    for domain in ALL_DOMAINS:
        domain_results = [r for r in results if r["domain"] == domain]
        mean = sum(r["elapsed_secs"] for r in domain_results) / len(domain_results)
        print(f"{domain} [{mean:.2f}]: ", end=" ")
        for res in domain_results:
            print(f"{res["elapsed_secs"]:.2f}", end=" ")
        print()


if __name__ == "__main__":
    fn = globals().get(sys.argv[1]) if len(sys.argv) == 2 else None
    if not callable(fn):
        print(f"usage: python run.py <function_name>", file=sys.stderr)
        sys.exit(1)
    fn()

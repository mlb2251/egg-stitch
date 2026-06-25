#!/usr/bin/env python3
"""Run a named experiment from the README. Usage: ./run.py <name>"""

import sys
import json
import math
from pathlib import Path
from expts import *


def dials_compress():
    egg_stitch(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        num_steps=10,
        num_particles=100,
    )


def dials_follow():
    egg_stitch(
        "data/domains/cogsci/dials.json",
        rewrites="../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites",
        num_steps=10,
        num_particles=100,
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
    The seen set (canonical-pattern dedup) is on by default; pass
    ``no_opt_seen=True`` to disable it (the benchmark runs without it).
    """
    egg_stitch(
        "data/domains/cogsci/nuts-bolts.json",
        rewrites="data/domains/cogsci/nuts-bolts.rewrites",
        output="nuts_bolts_enum.json",
        search="best-first",
        language="op-children",
        max_arity=2,
        num_abstractions=1,
        max_forced_expansion=1000,
        no_opt_useless_inline=True,
        no_seen_egraph_saturate_dynamic=False,
        seen_egraph_saturate_every=100,
        # verbose=True,
        # no_freeze_rule=True,
    )


# ── Table 1 & 2 BFS ("enum") column reproduced via the egg_stitch() escape hatch ──
# Each row is the BFS cell: best-first, arity 2, one abstraction, at the canonical
# sweep point num_steps=10000 (render_tables.TABLE_BFS_STEPS). This mirrors what
# expts.run_models.ours._run builds for the OursBf runner, but routes through
# egg_stitch() so every flag is visible and tweakable. Table 2 is without DSRs;
# Table 1 is the same but with the domain's rewrites file live during search.


def bfs_args(domain):
    """egg_stitch() kwargs for ``domain``'s BFS cell (shared by Table 1 and 2).

    Language follows the domain's weighting (cogsci/no-apps → op-children,
    dreamcoder/apps-equal → lambda-calc).
    """
    language = "op-children" if weighting_for(domain) == "no-apps" else "lambda-calc"
    return dict(search="best-first", language=language, max_arity=2,
                num_abstractions=1, num_steps=10000, no_opt_seen=True)

def run_bfs_cell(domain, use_dsrs=False, **overrides):
    """Run one BFS cell through egg_stitch(), once per input file, returning Paths.

    ``use_dsrs=False`` is the Table 2 cell; ``use_dsrs=True`` is the Table 1 cell
    (the domain's rewrites file goes live during search). ``overrides`` patch the
    argument dict (including ``rewrites`` to point at a different rule file).
    """
    args = {**bfs_args(domain), **overrides}
    tag = "table1" if use_dsrs else "table2"
    rewrites = args.pop("rewrites", rewrites_path(domain) if use_dsrs else None)
    return [egg_stitch(str(f), output=f"{tag}_{f.stem}.json", rewrites=rewrites, **args)
            for f in input_files(domain)]


def table2_all(**overrides):
    """Run every Table 2 BFS cell (no DSRs) through egg_stitch(), in table order.

    ``overrides`` are forwarded to every cell (e.g. ``no_opt_seen=True``).
    """
    return {d: run_bfs_cell(d, **overrides) for d in TABLE2_DOMAINS}


def table1_all(**overrides):
    """Run every Table 1 BFS cell (DSRs live) through egg_stitch(), in table order.

    ``overrides`` are forwarded to every cell (e.g. ``no_opt_seen=True``).
    """
    return {d: run_bfs_cell(d, use_dsrs=True, **overrides) for d in TABLE1_DOMAINS}


# ── Viewing results in the Table 1/2 format ───────────────────────────────────
# The published cells are NOT the binary's own compression_ratio: the table
# recomputes compression as a weighted geomean over a domain's files (final size
# includes learned abstraction bodies) and sums the time. These helpers reproduce
# that from the egg_stitch outputs so the numbers line up with the .tex. (Time
# here is egg-stitch's internal elapsed_secs; the table adds process+IO wall-clock
# on top, so it reads a touch higher there.)


def _cell_numbers(domain, output_files):
    """(compression_ratio, time_s) for a BFS cell, aggregated like the table.

    Mirrors expts.runner._bench_cost + render_common: per file, ratio is weighted
    initial / (final + abstraction bodies) AST size; the cell ratio is the geomean
    across files and the time is their summed elapsed_secs.
    """
    weighting = weighting_for(domain)
    ratios, total_time = [], 0.0
    for path in output_files:
        data = json.load(open(path))
        bodies = [a["pattern"].partition(": ")[2] or a["pattern"] for a in data.get("library", [])]
        ic = ast_size(data["original_programs"], weighting)
        fc = ast_size(data["rewritten_programs"], weighting) + ast_size(bodies, weighting)
        ratios.append(ic / fc)
        total_time += data["elapsed_secs"]
    geo = math.exp(sum(math.log(r) for r in ratios) / len(ratios)) if ratios else math.nan
    return geo, total_time


def _latest_results_folder():
    """The most recently modified viz/results/<timestamp> folder."""
    return max((p for p in RESULTS_DIR.iterdir() if p.is_dir()), key=lambda p: p.stat().st_mtime)


def _cell_files(domain, use_dsrs, folder):
    """The egg_stitch output Paths run_bfs_cell wrote for ``domain`` in ``folder``.

    One per input file; if a name collided and got a unique-path suffix, the most
    recent match wins.
    """
    tag = "table1" if use_dsrs else "table2"
    files = []
    for f in input_files(domain):
        matches = sorted(folder.glob(f"{tag}_{f.stem}*.json"), key=lambda p: p.stat().st_mtime)
        if matches:
            files.append(matches[-1])
    return files


def show_cell(domain, use_dsrs=False, folder=None):
    """Print one domain's BFS cell (compression ratio + time), table-style.

    Reads run_bfs_cell's outputs from ``folder`` (default: the latest results
    folder). Pass ``use_dsrs=True`` to view a Table 1 cell.
    """
    folder = Path(folder) if folder else _latest_results_folder()
    files = _cell_files(domain, use_dsrs, folder)
    if not files:
        print(f"{domain:14} (no outputs found in {folder})")
        return
    cr, t = _cell_numbers(domain, files)
    print(f"{domain:14} {cr:6.2f}   {t:8.3f}")


def show_table2(folder=None):
    """Print the BFS column of Table 2 from egg_stitch outputs."""
    print(f"{'Domain':14} {'Ratio':>6}   {'Time(s)':>8}")
    for d in TABLE2_DOMAINS:
        show_cell(d, folder=folder)


def show_table1(folder=None):
    """Print the BFS column of Table 1 from egg_stitch outputs."""
    print(f"{'Domain':14} {'Ratio':>6}   {'Time(s)':>8}")
    for d in TABLE1_DOMAINS:
        show_cell(d, use_dsrs=True, folder=folder)


def _parse_arg(s):
    """Coerce a CLI string to a bool, then an int, then a float, else a string."""
    if s in ("True", "False"):
        return s == "True"
    for cast in (int, float):
        try:
            return cast(s)
        except ValueError:
            pass
    return s


if __name__ == "__main__":
    fn = globals().get(sys.argv[1]) if len(sys.argv) >= 2 else None
    if not callable(fn):
        print("usage: python run.py <function_name> [arg | key=value ...]", file=sys.stderr)
        sys.exit(1)
    # ``key=value`` tokens become keyword arguments; everything else is positional.
    # Both keys' values are coerced via _parse_arg (bool → int → float → str).
    args, kwargs = [], {}
    for tok in sys.argv[2:]:
        key, sep, val = tok.partition("=")
        if sep and key.isidentifier():
            kwargs[key] = _parse_arg(val)
        else:
            args.append(_parse_arg(tok))
    fn(*args, **kwargs)

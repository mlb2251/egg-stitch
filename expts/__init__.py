#!/usr/bin/env python3
"""Run experiment variants and collect results into viz/results/."""

import os
import argparse
import shlex
import subprocess
from pathlib import Path

RESULTS_DIR = Path(__file__).parent.parent / "viz" / "results"

# Domains that have a matching drawings.<name>.rewrites file in ../babble.
DOMAINS_WITH_REWRITES = ["dials", "furniture", "nuts-bolts", "wheels"]
# All cogsci domains (with and without available rewrites).
ALL_DOMAINS = ["dials", "furniture", "nuts-bolts", "wheels"]


# babble doesnt try to do these domains
NON_BABBLE_DOMAINS = ["bridge", "castle", "city", "house"]


def compress(input, output="out.json", rewrites=None, **kwargs):
    """Run `cargo run --release` with the given input/output (relative to results dir)/optional rewrites."""
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    cmd = f"cargo run --release -- -i {input} --output {RESULTS_DIR / output}"
    if rewrites is not None:
        cmd += f" -r {rewrites}"
    for k, v in kwargs.items():
        k = k.replace("_", "-")
        cmd += f" --{k} {v}"
    print("+", cmd, flush=True)
    subprocess.run(shlex.split(cmd), check=True, env=dict(os.environ, RUST_BACKTRACE="1"))


def run_domain(domain, rewrites, **kwargs):
    """Run a cogsci domain benchmark, optionally with its rewrite set."""
    compress(
        input=f"data/domains/cogsci/{domain}.json",
        rewrites=f"../babble/harness/data/benchmark-dsrs/drawings.{domain}.rewrites" if rewrites else None,
        output=f"{domain}.json" if rewrites else f"{domain}_no_rewrites.json",
        **kwargs,
    )


def runall(**kwargs):
    """Run all experiments."""
    for d in ALL_DOMAINS:
        run_domain(d, rewrites=False, **kwargs)
    for d in DOMAINS_WITH_REWRITES:
        run_domain(d, rewrites=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("domain", nargs="?", default="all", help="domain name, or 'all'")
    parser.add_argument("--rewrites", action="store_true", help="use the domain's rewrite set")
    args = parser.parse_args()
    if args.domain == "all":
        runall()
    else:
        run_domain(args.domain, rewrites=args.rewrites)


if __name__ == "__main__":
    main()

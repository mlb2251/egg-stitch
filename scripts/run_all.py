#!/usr/bin/env python3
"""Run experiment variants and collect results into viz/results/."""

import argparse
import shlex
import subprocess
from pathlib import Path

RESULTS_DIR = Path(__file__).parent.parent / "viz" / "results"


def _run(cmd):
    """Invoke a shell-style command string after splitting with shlex."""
    RESULTS_DIR.mkdir(parents=True, exist_ok=True)
    print("+", cmd, flush=True)
    subprocess.run(shlex.split(cmd), check=True)


def dials():
    """Dials benchmark with rewrites."""
    _run(f"cargo run --release -- -i data/domains/cogsci/dials.json -r ../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites --output {RESULTS_DIR / 'dials.json'}")


def dials_no_rewrites():
    """Dials benchmark without rewrites."""
    _run(f"cargo run --release -- -i data/domains/cogsci/dials.json --output {RESULTS_DIR / 'dials_no_rewrites.json'}")


def runall():
    """Run all experiments."""
    dials()
    dials_no_rewrites()


EXPERIMENTS = {
    "dials": dials,
    "dials_no_rewrites": dials_no_rewrites,
    "all": runall,
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("experiment", choices=EXPERIMENTS.keys(), nargs="?", default="all")
    args = parser.parse_args()
    EXPERIMENTS[args.experiment]()


if __name__ == "__main__":
    main()

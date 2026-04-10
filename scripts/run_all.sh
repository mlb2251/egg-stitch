#!/usr/bin/env bash
# Run all experiment variants and collect results into viz/results/

set -euo pipefail

RESULTS_DIR="$(dirname "$0")/../viz/results"
mkdir -p "$RESULTS_DIR"

cargo run --release -- -i data/domains/cogsci/dials.json -r ../babble/harness/data/benchmark-dsrs/drawings.dials.rewrites --output "$RESULTS_DIR/dials.json"
cargo run --release -- -i data/domains/cogsci/dials.json --output "$RESULTS_DIR/dials_no_rewrites.json"


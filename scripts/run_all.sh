#!/usr/bin/env bash
# Run all experiment variants and collect results into viz/results/

set -euo pipefail

RESULTS_DIR="$(dirname "$0")/../viz/results"
mkdir -p "$RESULTS_DIR"

# Example:
# cargo run --release -- --input data/domains/simple-arithmetic/aplusbplusc.json \
#     --num-particles 1000 > "$RESULTS_DIR/variant_a.json"

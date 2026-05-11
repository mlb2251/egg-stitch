#!/usr/bin/env bash
# Regression check for a specific fv-related crash in `wrap_subst_args` /
# `apply_abstraction` (originally observed as an index OOB at cost.rs in the
# multi-abstraction SMC path). The bug only fires on some random traces, so
# we sweep seeds 0..=256 in parallel and fail (exit 1) if any seed crashes
# the binary.
set -u

JOBS="${JOBS:-$(nproc)}"
INPUT="${INPUT:-./data/domains/logo/logo_batch_50_1h_ellisk_2019-03-23T14.05.43__bench000_it0.json}"
BIN="${BIN:-./target/release/egg-stitch}"

if [ ! -x "$BIN" ]; then
    echo "building release binary..."
    cargo build --release
fi

FAILED=$(mktemp)
trap 'rm -f "$FAILED"' EXIT

run_one() {
    local seed=$1
    if ! "$BIN" -i "$INPUT" \
        --output /dev/null \
        --search smc --language lambda-calc \
        --max-arity 2 --num-abstractions 20 --rebuild-egraph \
        --num-steps 100 --num-particles 250 --temperature 1000.0 \
        --seed "$seed" >/dev/null 2>&1; then
        echo "$seed" >> "$2"
        echo "FAIL seed=$seed"
    fi
}
export -f run_one
export BIN INPUT

seq 0 256 | xargs -P "$JOBS" -I{} bash -c 'run_one "$@"' _ {} "$FAILED"

count=$(wc -l < "$FAILED" | tr -d ' ')
if [ "$count" -gt 0 ]; then
    echo "=== $count seed(s) crashed ==="
    sort -n "$FAILED"
    exit 1
fi
echo "all 257 seeds passed"

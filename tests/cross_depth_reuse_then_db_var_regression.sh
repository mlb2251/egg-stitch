#!/usr/bin/env bash
# Regression for the cross-depth-reuse-then-leaf-DB-var unsoundness.
#
# `subset_matches_reuse` merges two metavars whose captures are shift-related
# (here `$0` at depth 2 with `$1` at depth 3, k=1) and stores only the shallow
# id, collapsing `var_depth` to the min. A subsequent literal `$n` leaf
# expansion is then accepted by `target_is_free_db_var` (which only consults
# the min), and the same `$n` leaf is placed at both occurrences — but at the
# deep site it references a different binder than the original `$1`, so the
# resulting `(fn_0 foo) ≠ (lam (lam (foo $0 (lam $1))))`.
#
# On the buggy `variables-at-multiple-depths` branch SMC seed 0 picks this
# bad pattern deterministically; on a sound build the rewritten programs
# β-reduce back to the originals and `check_equiv.py` passes. We pin a small
# set of seeds rather than just seed 0 so a future RNG change can't silently
# stop exercising the path.
set -u

INPUT="${INPUT:-./data/domains/ho-bugs/cross_depth_reuse_then_db_var.json}"
BIN="${BIN:-./target/release/egg-stitch}"
CHECKER="${CHECKER:-./scripts/check_equiv.py}"
SEEDS="${SEEDS:-0 1 2 3 4 5 6 7}"

if [ ! -x "$BIN" ]; then
    echo "building release binary..."
    cargo build --release
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

fail=0
for seed in $SEEDS; do
    out="$TMP/seed-$seed.json"
    if ! "$BIN" -i "$INPUT" \
            --output "$out" \
            --search smc --language lambda-calc \
            --num-abstractions 1 \
            --num-particles 1000 --num-steps 1000 --temperature 1000 \
            --seed "$seed" >/dev/null 2>&1; then
        echo "FAIL seed=$seed: egg-stitch crashed"
        fail=$((fail + 1))
        continue
    fi
    if ! python3 "$CHECKER" "$out" >"$TMP/seed-$seed.log" 2>&1; then
        echo "FAIL seed=$seed: check_equiv rejected rewritten programs"
        sed 's/^/    /' "$TMP/seed-$seed.log"
        fail=$((fail + 1))
    fi
done

if [ "$fail" -gt 0 ]; then
    echo "=== $fail seed(s) failed ==="
    exit 1
fi
echo "all seeds passed"

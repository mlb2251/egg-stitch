#!/usr/bin/env bash
# Regression for the cross-depth `fv_outside_gap` unsoundness in `shift_equal`.
#
# `shift_eq_struct`'s same-e-class shortcut accepts a shared child whose free
# indices are `>= init_depth + s` (`fv_outside_gap`'s upper branch), but the
# per-leaf rule shifts *every* free index `>= init_depth` up by `s`. So a
# shared, non-closed child above the gap is wrongly treated as shift-invariant,
# and a cross-depth `reuse` that should be rejected is accepted.
#
# Corpus: `(lam (lam (g (lam (f $1 $2)) (f $1 $1))))`. The pattern
# `(g (lam ?#0) ?#0)` reuses a metavar across depths 1 and 0; the two captures
# `(f $1 $2)` and `(f $1 $1)` share the e-class `(f $1)` (fv {1}) sitting exactly
# at the gap boundary, so the buggy `shift_equal` returns true. The resulting
# abstraction `fn_0 = (lam (g (lam $1) $0))` reconstructs the deep occurrence as
# `(f $2 $2)` instead of `(f $1 $2)` — i.e. the rewritten corpus is NOT
# β-equivalent to the original, which `check_equiv.py` detects.
#
# We use `--follow` so the run is deterministic (no seed dependence): on a buggy
# build the reuse is enumerable and follow reaches the bad pattern; on a sound
# build `shift_equal` rejects the reuse, the follow target is unreachable, no
# abstraction is emitted, and the corpus is returned unchanged (trivially
# equivalent).
set -u

INPUT="${INPUT:-./data/domains/ho-bugs/cross_depth_fv_outside_gap.json}"
FOLLOW="${FOLLOW:-(g (lam ?#0) ?#0)}"
BIN="${BIN:-./target/release/egg-stitch}"
CHECKER="${CHECKER:-./scripts/check_equiv.py}"

if [ ! -x "$BIN" ]; then
    echo "building release binary..."
    cargo build --release
fi

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

fail=0
for search in best-first smc; do
    out="$TMP/$search.json"
    if ! "$BIN" -i "$INPUT" \
            --output "$out" \
            --search "$search" --language lambda-calc \
            --num-abstractions 1 \
            --num-particles 1000 --num-steps 5000 --temperature 1000 \
            --follow "$FOLLOW" >/dev/null 2>&1; then
        echo "FAIL search=$search: egg-stitch crashed"
        fail=$((fail + 1))
        continue
    fi
    if ! python3 "$CHECKER" "$out" >"$TMP/$search.log" 2>&1; then
        echo "FAIL search=$search: check_equiv rejected rewritten programs"
        sed 's/^/    /' "$TMP/$search.log"
        fail=$((fail + 1))
    fi
done

if [ "$fail" -gt 0 ]; then
    echo "=== $fail backend(s) failed ==="
    exit 1
fi
echo "all backends passed"

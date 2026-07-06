#!/usr/bin/env bash
# Run every ./run.py table experiment (plus the arity sweep) in order, then
# render them. Continues past a failing experiment and reports failures at the end.
set -uo pipefail

cd "$(dirname "$0")/.."

failed=()
for expt in table1 table2 table3 table4 table5 table7 arity_experiment; do
    echo "=== Running ${expt} ==="
    if ! ./run.py "${expt}"; then
        echo "!!! ${expt} failed, continuing"
        failed+=("${expt}")
    fi
done

echo "=== Rendering tables ==="
python scripts/render_tables.py
echo "=== Rendering arity ==="
python scripts/render_arity.py

if (( ${#failed[@]} > 0 )); then
    echo "Tables that failed: ${failed[*]}"
    exit 1
fi

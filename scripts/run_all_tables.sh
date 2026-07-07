#!/usr/bin/env bash
# Run every ./run.py tableX experiment (plus the arity sweep and ablation) in
# order, then render them all. Continues past a failing experiment and reports
# which ones failed at the end.
set -uo pipefail

cd "$(dirname "$0")/.."

failed=()
for table in table1 table2 table3 table4 table5 table7; do
    echo "=== Running ${table} ==="
    if ! ./run.py "${table}"; then
        echo "!!! ${table} failed, continuing"
        failed+=("${table}")
    fi
done

echo "=== Rendering tables ==="
python scripts/render_tables.py

echo "=== Running arity ==="
if ./run.py arity_experiment; then
    python scripts/render_arity.py
else
    echo "!!! arity_experiment failed, continuing"
    failed+=("arity_experiment")
fi

# Ablation study: reuses results/table{3,5,7}.json (the hardest experiment of
# each), so it runs after those tables. Every measurement is cached, so this is
# cheap to re-run.
echo "=== Running ablation ==="
if ./run.py ablation; then
    python scripts/render_ablation.py
else
    echo "!!! ablation failed, continuing"
    failed+=("ablation")
fi

if (( ${#failed[@]} > 0 )); then
    echo "Tables that failed: ${failed[*]}"
    exit 1
fi

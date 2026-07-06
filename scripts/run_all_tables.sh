#!/usr/bin/env bash
# Run every ./run.py tableX experiment in order, then render the tables.
# Continues past a failing table and reports which ones failed at the end.
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

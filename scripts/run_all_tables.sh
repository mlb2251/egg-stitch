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

if (( ${#failed[@]} > 0 )); then
    echo "Tables that failed: ${failed[*]}"
    exit 1
fi

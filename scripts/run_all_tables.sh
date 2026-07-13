#!/usr/bin/env bash
# Run every ./run.py table experiment (plus the arity sweep and ablation) in
# order, then render them. Continues past a failing experiment and reports failures at the end.
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

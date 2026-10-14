#!/usr/bin/env python3
"""Checks that the ablation's fixed domain and BFS point per table
(``ABLATION_PICKS``) are still what the table results would pick.

Run directly: `python3 scripts/test_ablation_sync.py`. Exits nonzero on any
mismatch.
"""

import sys
from dataclasses import replace
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from expts.ablation_study import (  # noqa: E402
    ABLATION_PICKS,
    TABLE_SPECS,
    _load_table,
    _reported_enum_point,
    hardest_domain,
)


def main() -> None:
    """Compare each table's pick against the one its results give."""
    failed = False
    for table, spec in TABLE_SPECS.items():
        saved = _load_table(spec)
        spec = replace(spec, enum_point=_reported_enum_point(spec, saved))
        picked = (hardest_domain(spec, saved), spec.enum_point)
        if picked != ABLATION_PICKS[table]:
            print(f"FAIL: {table}: ABLATION_PICKS has {ABLATION_PICKS[table]}, table results give {picked}")
            failed = True
    print("ablation picks match" if not failed else "ablation picks out of date")
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()

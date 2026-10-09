#!/usr/bin/env python3
"""Checks that `results/ablation.json` and the ablation cache agree with the
table results the ablation is derived from.

Run directly: `python3 scripts/test_ablation_sync.py`. Exits nonzero on any
mismatch.
"""

import json
import sys
from dataclasses import replace
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from expts.ablation_study import (  # noqa: E402
    TABLE_SPECS,
    TARGET_COMPRESSION_FRACTION,
    _cache_path,
    _load_table,
    _reported_enum_point,
    hardest_domain,
)
from expts.folders import SUMMARY_RESULTS_DIR  # noqa: E402


def mismatches() -> list[str]:
    """Every disagreement between ablation.json, its cache, and the table results."""
    recorded = json.loads((SUMMARY_RESULTS_DIR / "ablation.json").read_text())["tables"]
    errors = []
    for table, spec in TABLE_SPECS.items():
        saved = _load_table(spec)
        spec = replace(spec, enum_point=_reported_enum_point(spec, saved))
        domain = hardest_domain(spec, saved)
        rec = recorded[table]
        if (rec["domain"], rec["enum_point"]) != (domain, spec.enum_point):
            errors.append(f"{table}: ablation.json has {rec['domain']} at {rec['enum_point']}, "
                          f"table results give {domain} at {spec.enum_point}")
            continue

        def check(key: str, expected: dict, fields) -> None:
            path = _cache_path(spec, domain, key)
            if not path.exists():
                errors.append(f"{table}: missing {path}")
                return
            cached = json.loads(path.read_text())
            for f in fields:
                if cached[f] != expected[f]:
                    errors.append(f"{table}: {key}.{f} is {cached[f]} in the cache, {expected[f]} in ablation.json")

        target = _cache_path(spec, domain, "bfs_target")
        if not target.exists() or json.loads(target.read_text())["cr"] * TARGET_COMPRESSION_FRACTION != rec["target_cr"]:
            errors.append(f"{table}: {target} doesn't give ablation.json's target_cr")
        for name, r in rec["bfs"].items():
            check(f"bfs_{name}", r, ["time", "steps", "cr", "dnf"])
        for name, r in rec["smc"].items():
            if r["reached"]:
                check(f"smc_{name}_p{r['particles']}", r, ["time", "steps", "cr"])
    return errors


def main() -> None:
    """Print every mismatch and exit nonzero if there are any."""
    errors = mismatches()
    for e in errors:
        print(f"FAIL: {e}")
    print(f"{len(errors)} mismatches")
    sys.exit(1 if errors else 0)


if __name__ == "__main__":
    main()

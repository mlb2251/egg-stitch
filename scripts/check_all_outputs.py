#!/usr/bin/env python3
"""Run `check_equiv.py` on every `*.out.json` under `data/expected_outputs/`.

Each fixture is run β-only by default. Fixtures that the search produced
*with* a DSR file get checked against those same DSRs via `RULES_BY_PATH`;
without them, β alone can't bridge cases like `(* 0 ?x) ≡ 0`.

Fixtures whose library has no `lambda` field are skipped internally by
`check_equiv.py` (lambda-free OpChildren runs).

Exit 0 iff every applicable file checks out.
"""

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
CHECKER = HERE / "check_equiv.py"
ROOT = REPO / "data" / "expected_outputs"

# Fixtures where the search was run with `-r <rules>` and β alone is not
# enough to bridge the DSR-mediated equivalence in the rewritten programs.
# Keep in sync with `tests/stitch_compat_test.rs`.
RULES_BY_REL = {
    "fv-overapprox/annihilator.out.json": "data/domains/fv-overapprox/annihilator.rewrites",
    "stitch/nested.out.json": "data/domains/stitch/nested.rewrites",
}


def main():
    paths = sorted(ROOT.rglob("*.out.json"))
    if not paths:
        print(f"no *.out.json under {ROOT}", file=sys.stderr)
        sys.exit(1)
    # Group by rewrites file (None for β-only) so each batch becomes one
    # check_equiv invocation.
    batches = {}
    for p in paths:
        rel = str(p.relative_to(ROOT))
        rules = RULES_BY_REL.get(rel)
        batches.setdefault(rules, []).append(p)
    overall = 0
    for rules, group in batches.items():
        cmd = [sys.executable, str(CHECKER), *[str(p) for p in group]]
        if rules:
            cmd += ["--rewrites", rules]
        label = f"(rules={rules})" if rules else "(β-only)"
        print(f"$ check_equiv.py {label} <{len(group)} files>")
        res = subprocess.run(cmd, cwd=REPO)
        if res.returncode != 0:
            overall = res.returncode
    sys.exit(overall)


if __name__ == "__main__":
    main()

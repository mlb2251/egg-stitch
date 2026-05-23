#!/usr/bin/env python3
"""Run `follow_reaches.py` on every `*.json` under one or more domain dirs.

For each input, both backends must reach the abstraction discovered by a
deterministic best-first run. Used as a CI sweep to catch regressions where
follow mode loses the ability to reproduce stitch's own output.

Usage:
    scripts/follow_reaches_domain.py DOMAIN_DIR [DOMAIN_DIR …] [-- extra args]

Extra args after `--` are forwarded to `follow_reaches.py` (and thence to
every egg-stitch invocation).

Exit 0 iff every input passes (skips count as passes).
"""

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
RUNNER = HERE / "follow_reaches.py"

# Only these four cogsci corpora are flat s-exprs (op-children); the other
# files under data/domains/cogsci/ (bridge, castle, city, house) are curried
# lambda-calc, matching `expts/__init__.COGSCI_DOMAINS`.
COGSCI_OP_CHILDREN = {"dials", "furniture", "nuts-bolts", "wheels"}


def main():
    argv = sys.argv[1:]
    passthrough = []
    if "--" in argv:
        i = argv.index("--")
        argv, passthrough = argv[:i], argv[i + 1:]
    if not argv:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
    dirs = [Path(p) for p in argv]
    inputs = []
    for d in dirs:
        if not d.is_dir():
            print(f"{d}: not a directory", file=sys.stderr)
            sys.exit(2)
        inputs.extend(sorted(d.glob("*.json")))
    if not inputs:
        print(f"no *.json files found under {argv}", file=sys.stderr)
        sys.exit(1)

    print(f"sweeping {len(inputs)} problem(s) across {len(dirs)} domain dir(s)")
    failures = []
    for i, inp in enumerate(inputs, 1):
        rel = inp.relative_to(REPO) if inp.is_absolute() and REPO in inp.parents else inp
        print(f"\n[{i}/{len(inputs)}] {rel}")
        # Match the benchmark convention (`expts/runner.py:weighting_for`):
        # only the four canonical cogsci corpora are flat s-exprs → op-children;
        # everything else (incl. bridge/castle/city/house under cogsci/) is
        # curried dreamcoder-style → lambda-calc.
        language = "op-children" if inp.parent.name == "cogsci" and inp.stem in COGSCI_OP_CHILDREN else "lambda-calc"
        cmd = [sys.executable, str(RUNNER), str(inp), "--", "--language", language]
        if passthrough:
            cmd += [*passthrough]
        res = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True)
        if res.returncode != 0:
            failures.append(rel)
            sys.stdout.write(res.stdout)
            sys.stderr.write(res.stderr)
            print(f"FAIL: {rel}")
        else:
            # Just echo the trailing summary lines (or SKIP notice).
            for line in res.stdout.splitlines():
                if "PASS" in line or "FAIL" in line or "SKIP" in line or "summary" in line:
                    print("  " + line.strip())

    print(f"\n=== sweep summary: {len(inputs) - len(failures)}/{len(inputs)} passed ===")
    if failures:
        print("failures:")
        for f in failures:
            print(f"  {f}")
    sys.exit(0 if not failures else 1)


if __name__ == "__main__":
    main()

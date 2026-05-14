#!/usr/bin/env python3
"""Run `search_and_check.py` on every `*.json` under one or more domain
directories. Used to sweep real-world corpora (`physics`, `list`) end-to-end:
for each input, both backends run the search and the checker verifies that
the rewritten programs are β-equivalent to the original ones.

Usage:
    scripts/search_and_check_domain.py DOMAIN_DIR [DOMAIN_DIR …] [-- extra args]

Extra args after `--` are forwarded to `search_and_check.py`'s own
passthrough (i.e. on to `egg-stitch`), e.g.:

    scripts/search_and_check_domain.py data/domains/physics -- \\
        --language lambda-calc --num-steps 2000

Exit 0 iff every problem in every directory passes both backends.
"""

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
RUNNER = HERE / "search_and_check.py"


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
        cmd = [sys.executable, str(RUNNER), str(inp)]
        if passthrough:
            cmd += ["--", *passthrough]
        # Quieter: suppress per-run egg-stitch chatter except on failure.
        res = subprocess.run(cmd, cwd=REPO, capture_output=True, text=True)
        if res.returncode != 0:
            failures.append(rel)
            sys.stdout.write(res.stdout)
            sys.stderr.write(res.stderr)
            print(f"FAIL: {rel}")
        else:
            # Just print the trailing summary lines.
            for line in res.stdout.splitlines():
                if "PASS" in line or "FAIL" in line or "summary" in line:
                    print("  " + line.strip())

    print(f"\n=== sweep summary: {len(inputs) - len(failures)}/{len(inputs)} passed ===")
    if failures:
        print("failures:")
        for f in failures:
            print(f"  {f}")
    sys.exit(0 if not failures else 1)


if __name__ == "__main__":
    main()

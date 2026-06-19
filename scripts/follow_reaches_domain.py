#!/usr/bin/env python3
"""Run `follow_reaches.py` on every `*.json` under one or more domain dirs.

For each input, both backends must reach the abstraction discovered by the
reference `stitch` compressor. Used as a CI sweep to catch regressions where
follow mode loses the ability to reproduce stitch's output.

A handful of inputs legitimately *don't* reproduce stitch's exact abstraction
— egg-stitch finds a strictly better (more general) one, or stitch's only
abstraction is unrepresentable in egg-stitch. Those are listed in
`KNOWN_DIVERGENCES` and reported as XFAIL rather than failing the sweep; any
failure *outside* that allow-list is a real regression. The list is exact
(per input), so when egg-stitch improves and an entry starts passing it is
flagged as an UNEXPECTED PASS to prune.

Usage:
    scripts/follow_reaches_domain.py DOMAIN_DIR [DOMAIN_DIR …] [-- extra args]

Extra args after `--` are forwarded to `follow_reaches.py` (and thence to
every egg-stitch invocation).

Exit 0 iff every non-allow-listed input passes (skips and XFAILs count as
passes).
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

# Inputs (repo-relative path -> expected best-first divergence) where egg-stitch
# does not reproduce stitch's *exact* abstraction by design. `want` is stitch's
# (deterministic) target; `got` is the (deterministic best-first) pattern
# egg-stitch reaches instead. A failure outside this set, or one whose actual
# `want`/`got` no longer matches what's recorded here, is a real regression and
# fails the sweep — so improvements and new behaviours both get a human's eyes.
KNOWN_DIVERGENCES = {
    # egg-stitch finds a strictly *better* abstraction than stitch: it
    # higher-order-wraps an argument that genuinely varies across the corpus,
    # so its arity-(k+1) form has more uses and lower cost than stitch's
    # arity-k pick. (Verified on text it3: stitch `(lam (lam (fn_170 $0 $1)))`
    # = 4 uses / cost 276; egg-stitch `(lam (lam (fn_170 (?#0 $0) $1)))` = 9
    # uses / cost 225.) stitch declines the generalisation; egg-stitch takes it.
    "data/domains/text/text_ellisk_2019-01-24T21.49.39__bench003_it3.json":
        {"want": "(lam (lam (fn_170 $0 $1)))", "got": "(lam (lam (fn_170 (?#0 $0) $1)))"},
    "data/domains/text/text_ellisk_2019-01-24T21.58.02__bench003_it4.json":
        {"want": "(lam (lam (fn_36 $1 $0)))", "got": "(lam (lam (fn_36 $1 (?#0 $0))))"},
    "data/domains/text/text_ellisk_2019-01-25T20.19.06__bench003_it3.json":
        {"want": "(lam (lam (fn_170 $0 $1)))", "got": "(lam (lam (fn_170 (?#0 $0) $1)))"},
    "data/domains/ho-bugs/cross_slot_shift_memo.json":
        {"want": "(fold ?#0 0. (lam (lam (+. $0 $1))))", "got": "(fold ?#0 0. (lam (lam (+. $0 (?#1 $1)))))"},
    "data/domains/ho-bugs/zip_fold_capture.json":
        {"want": "(fold ?#0 0. (lam (lam (+. $0 $1))))", "got": "(fold ?#0 0. (lam (lam (+. $0 (?#1 $1)))))"},
}


def parse_divergences(stderr):
    """Extract the `(want, got)` pairs a follow_reaches run printed on failure.

    follow_reaches emits, per non-reaching backend, two stderr lines:
    ``  want: <target>`` then ``  got : <pattern-or-None>``.
    """
    pairs, want = [], None
    for line in stderr.splitlines():
        s = line.strip()
        if s.startswith("want:"):
            want = s[len("want:"):].strip()
        elif s.startswith("got :") and want is not None:
            pairs.append((want, s[len("got :"):].strip()))
            want = None
    return pairs


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
    failures, xfails, unexpected = [], [], []
    for i, inp in enumerate(inputs, 1):
        rel = inp.relative_to(REPO) if inp.is_absolute() and REPO in inp.parents else inp
        expected = KNOWN_DIVERGENCES.get(str(rel))
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
            # An allow-listed divergence is tolerated only if the exact recorded
            # (want, got) shows up — a changed divergence is a real regression.
            if expected is not None and (expected["want"], expected["got"]) in parse_divergences(res.stderr):
                xfails.append(rel)
                print(f"  XFAIL (known divergence): want {expected['want']} -> got {expected['got']}")
            else:
                failures.append(rel)
                sys.stdout.write(res.stdout)
                sys.stderr.write(res.stderr)
                if expected is not None:
                    print(f"FAIL: {rel} — allow-listed divergence changed; "
                          f"expected got {expected['got']!r}, saw {parse_divergences(res.stderr)}")
                else:
                    print(f"FAIL: {rel}")
        else:
            if expected is not None:
                unexpected.append(rel)
                print(f"  UNEXPECTED PASS: {rel} now reaches stitch's abstraction — drop it from KNOWN_DIVERGENCES")
            # Just echo the trailing summary lines (or SKIP notice).
            for line in res.stdout.splitlines():
                if "PASS" in line or "FAIL" in line or "SKIP" in line or "summary" in line:
                    print("  " + line.strip())

    passed = len(inputs) - len(failures) - len(xfails)
    print(f"\n=== sweep summary: {passed} passed, {len(xfails)} known-divergence XFAIL, "
          f"{len(failures)} failed (of {len(inputs)}) ===")
    if failures:
        print("failures:")
        for f in failures:
            print(f"  {f}")
    if unexpected:
        print("unexpected passes (prune from KNOWN_DIVERGENCES):")
        for f in unexpected:
            print(f"  {f}")
    sys.exit(0 if not failures else 1)


if __name__ == "__main__":
    main()

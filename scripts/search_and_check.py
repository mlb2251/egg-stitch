#!/usr/bin/env python3
"""Run egg-stitch with both `smc` and `best-first` on a single input + DSR
file, then verify each produced output is equivalent to the input using
`check_equiv.py`.

Any args after `--` are appended to both invocations (e.g.
`-- --sym-var-cost 100 --max-arity 1`). Search-specific defaults
(`--num-steps`, `--num-particles`, …) match `tests/stitch_compat_test.rs`
and can be overridden by passing them through.

Exit 0 only when both runs complete and both pass the equivalence check.
"""

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
CHECKER = HERE / "check_equiv.py"

# Match the defaults used in tests/stitch_compat_test.rs.
SMC_DEFAULTS = ["--num-particles", "1000", "--num-steps", "1000", "--temperature", "1000"]
BF_DEFAULTS = ["--num-steps", "50000"]


def cargo_binary():
    """Build the release binary once and return its path. `cargo build` is a
    no-op when up-to-date, so this is cheap on repeat runs."""
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=REPO, check=True)
    return REPO / "target" / "release" / "egg-stitch"


def _flag_names(args):
    """Return the set of long-option flag names (`--foo`) present in `args`."""
    return {a for a in args if a.startswith("--")}


def _drop_overridden(defaults, override_flags):
    """Strip `--flag VALUE` pairs from `defaults` whenever `--flag` appears in
    `override_flags`, so a user-supplied value wins over the built-in default."""
    out = []
    i = 0
    while i < len(defaults):
        a = defaults[i]
        if a.startswith("--") and a in override_flags and i + 1 < len(defaults):
            i += 2  # skip flag + its value
            continue
        out.append(a)
        i += 1
    return out


def run_search(binary, search, input_path, rewrites, output_path, passthrough):
    defaults = SMC_DEFAULTS if search == "smc" else BF_DEFAULTS
    defaults = _drop_overridden(defaults, _flag_names(passthrough))
    cmd = [
        str(binary),
        "--search", search,
        "--input", str(input_path),
        "--num-abstractions", "1",
        "--output", str(output_path),
    ]
    if rewrites:
        cmd += ["-r", str(rewrites)]
    cmd += defaults
    cmd += list(passthrough)
    print(f"$ {' '.join(cmd)}", file=sys.stderr)
    res = subprocess.run(cmd, cwd=REPO)
    return res.returncode == 0


def run_checker(output_path, rewrites, verbose):
    cmd = [sys.executable, str(CHECKER), str(output_path)]
    if rewrites:
        cmd += ["--rewrites", str(rewrites)]
    if verbose:
        cmd.append("-v")
    print(f"$ {' '.join(cmd)}", file=sys.stderr)
    res = subprocess.run(cmd, cwd=REPO)
    return res.returncode == 0


def main():
    # Split on the first literal `--` so our flags and the egg-stitch passthrough
    # are unambiguous (argparse's `REMAINDER` is greedy and slurps recognized
    # flags too, so we do this by hand).
    argv = sys.argv[1:]
    passthrough = []
    if "--" in argv:
        i = argv.index("--")
        argv, passthrough = argv[:i], argv[i + 1:]

    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("input", help="programs JSON (passed to egg-stitch --input)")
    ap.add_argument("--rewrites", help="DSR file (passed to egg-stitch -r and to check_equiv)")
    ap.add_argument("--keep", action="store_true", help="keep the per-search *.out.json files in --outdir")
    ap.add_argument("--outdir", default=None, help="directory for output JSONs (default: tempdir, cleaned unless --keep)")
    ap.add_argument("-v", "--verbose", action="store_true", help="forward -v to check_equiv")
    args = ap.parse_args(argv)

    binary = cargo_binary()
    cleanup = False
    if args.outdir is None:
        outdir = Path(tempfile.mkdtemp(prefix="search_and_check-"))
        cleanup = not args.keep
    else:
        outdir = Path(args.outdir)
        outdir.mkdir(parents=True, exist_ok=True)

    stem = Path(args.input).stem
    results = {}
    try:
        for search in ("best-first", "smc"):
            out = outdir / f"{stem}.{search}.out.json"
            print(f"\n=== {search} ===", file=sys.stderr)
            if not run_search(binary, search, args.input, args.rewrites, out, passthrough):
                print(f"{search}: search failed", file=sys.stderr)
                results[search] = False
                continue
            print(f"--- checking {search} output ---", file=sys.stderr)
            results[search] = run_checker(out, args.rewrites, args.verbose)
    finally:
        if cleanup:
            shutil.rmtree(outdir, ignore_errors=True)

    print("\n=== summary ===")
    for search, ok in results.items():
        print(f"  {search}: {'PASS' if ok else 'FAIL'}")
    sys.exit(0 if all(results.values()) else 1)


if __name__ == "__main__":
    main()

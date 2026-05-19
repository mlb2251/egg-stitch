#!/usr/bin/env python3
"""Verify that both search backends can reach the abstraction stitch finds.

For a given input (and optional DSR file):
  1. Run egg-stitch with `--search best-first` to discover an abstraction.
  2. Strip the `fn_N: ` prefix from `library[0].pattern` to get the body.
  3. Re-run egg-stitch with `--follow <body>` under both `smc` and
     `best-first`, asserting each run's `library[0].pattern` body matches.

If step 1 produces no library entry (e.g. the corpus has no compressible
abstraction), the input is skipped with a pass — there is nothing to follow.

Any args after `--` are forwarded to every egg-stitch invocation.
Exit 0 iff every follow run reaches the discovered pattern.
"""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent

# Mirror search_and_check.py: same defaults, same override behavior.
SMC_DEFAULTS = ["--num-particles", "1000", "--num-steps", "1000", "--temperature", "1000"]
BF_DEFAULTS = ["--num-steps", "50000"]


def cargo_binary():
    """Build the release binary once; cheap when up-to-date."""
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=REPO, check=True)
    return REPO / "target" / "release" / "egg-stitch"


def _flag_names(args):
    return {a for a in args if a.startswith("--")}


def _drop_overridden(defaults, override_flags):
    out, i = [], 0
    while i < len(defaults):
        a = defaults[i]
        if a.startswith("--") and a in override_flags and i + 1 < len(defaults):
            i += 2
            continue
        out.append(a)
        i += 1
    return out


def run_egg_stitch(binary, search, input_path, rewrites, output_path, passthrough, follow=None):
    """Invoke egg-stitch for one search; return True on exit code 0."""
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
    if follow is not None:
        cmd += ["--follow", follow]
    cmd += defaults + list(passthrough)
    print(f"$ {' '.join(cmd)}", file=sys.stderr)
    return subprocess.run(cmd, cwd=REPO).returncode == 0


def pattern_body(result_json):
    """Return the `fn_N: ` body of `library[0].pattern`, or None if no library."""
    lib = result_json.get("library") or []
    if not lib:
        return None
    full = lib[0]["pattern"]
    _, _, body = full.partition(": ")
    return body if body else full


def main():
    argv = sys.argv[1:]
    passthrough = []
    if "--" in argv:
        i = argv.index("--")
        argv, passthrough = argv[:i], argv[i + 1:]

    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("input", help="programs JSON")
    ap.add_argument("--rewrites", help="DSR file passed to egg-stitch -r")
    args = ap.parse_args(argv)

    binary = cargo_binary()

    with tempfile.TemporaryDirectory(prefix="follow_reaches-") as td:
        outdir = Path(td)
        stem = Path(args.input).stem

        # 1) Discovery run: best-first is deterministic given the same defaults,
        #    so the pattern we extract is reproducible.
        disc_out = outdir / f"{stem}.discovery.out.json"
        print("\n=== discovery (best-first) ===", file=sys.stderr)
        if not run_egg_stitch(binary, "best-first", args.input, args.rewrites, disc_out, passthrough):
            print("discovery: search failed", file=sys.stderr)
            sys.exit(1)
        disc = json.loads(disc_out.read_text())
        target = pattern_body(disc)
        if target is None:
            print(f"SKIP: no abstraction found for {args.input} — nothing to follow")
            sys.exit(0)
        print(f"follow target: {target}", file=sys.stderr)

        # 2) Follow runs: both backends must reach the same canonical pattern.
        #    Pattern stringification is canonical (alpha-equivalent patterns
        #    render identically), so plain string equality suffices.
        results = {}
        for search in ("best-first", "smc"):
            out = outdir / f"{stem}.{search}.follow.out.json"
            print(f"\n=== {search} (follow) ===", file=sys.stderr)
            if not run_egg_stitch(binary, search, args.input, args.rewrites, out, passthrough, follow=target):
                print(f"{search}: search failed", file=sys.stderr)
                results[search] = False
                continue
            got = pattern_body(json.loads(out.read_text()))
            ok = got == target
            if not ok:
                print(f"{search}: did not reach follow target", file=sys.stderr)
                print(f"  want: {target}", file=sys.stderr)
                print(f"  got : {got}", file=sys.stderr)
            results[search] = ok

    print("\n=== summary ===")
    for search, ok in results.items():
        print(f"  {search}: {'PASS' if ok else 'FAIL'}")
    sys.exit(0 if all(results.values()) else 1)


if __name__ == "__main__":
    main()

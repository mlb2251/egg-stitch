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

# Fixed at 2000 steps for both backends — the follow sweep is a CI diagnostic,
# not a quality search, so we want uniform, bounded runtime per input.
SMC_DEFAULTS = ["--num-particles", "1000", "--num-steps", "2000", "--temperature", "1000"]
BF_DEFAULTS = ["--num-steps", "2000"]


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


def parse_sexp(s):
    """Tiny s-expression parser: atoms become strings, lists become tuples."""
    toks, i = [], 0
    while i < len(s):
        c = s[i]
        if c.isspace():
            i += 1
        elif c in "()":
            toks.append(c)
            i += 1
        else:
            j = i
            while j < len(s) and not s[j].isspace() and s[j] not in "()":
                j += 1
            toks.append(s[i:j])
            i = j
    pos = [0]

    def read():
        t = toks[pos[0]]; pos[0] += 1
        if t != "(":
            return t
        out = []
        while toks[pos[0]] != ")":
            out.append(read())
        pos[0] += 1
        return tuple(out)

    return read()


def follow_equivalent(target, got):
    """Alpha-equivalence between two abstraction bodies, with the relaxation
    that `(?#k $a $b …)` (metavar HO-applied to De Bruijn vars) is equivalent
    to a bare `?#m`: either form represents an unrefined slot. Metavar names
    are matched under a consistent bijective rename.
    """
    a, b = parse_sexp(target), parse_sexp(got)
    fwd, rev = {}, {}

    def is_meta(x):
        return isinstance(x, str) and x.startswith("?#")

    def is_db(x):
        return isinstance(x, str) and x.startswith("$") and x[1:].isdigit()

    def meta_head(x):
        """If `x` is a bare metavar or `(?#k $a $b …)`, return the head name."""
        if is_meta(x):
            return x
        if isinstance(x, tuple) and x and is_meta(x[0]) and all(is_db(a) for a in x[1:]):
            return x[0]
        return None

    def split_meta_ho(x):
        """If `x` is `(?#k $a $b … rest …)` — leading metavar, then one or more
        bound-var args (the HO-wrap of slot `k`), then any number of non-DB
        siblings — return `(?#k, rest…)`. Folds the HO-wrap so an η-applied
        slot is alpha-equivalent to a bare metavar followed by its siblings.
        Returns None when the shape doesn't match (no HO prefix, or any
        non-trailing non-DB arg).
        """
        if not (isinstance(x, tuple) and len(x) >= 2 and is_meta(x[0])):
            return None
        i = 1
        while i < len(x) and is_db(x[i]):
            i += 1
        if i == 1:
            return None
        return (x[0],) + tuple(x[i:])

    def bind(av, bv):
        if av in fwd and fwd[av] != bv: return False
        if bv in rev and rev[bv] != av: return False
        fwd[av] = bv; rev[bv] = av
        return True

    def go(a, b):
        # Try the bare/HO-wrap collapse first — `(?#k $0)` ≡ `?#k` and
        # `(?#k $0 ?#j)` ≡ `(?#k ?#j)`, etc.
        ma, mb = meta_head(a), meta_head(b)
        if ma is not None and mb is not None:
            return bind(ma, mb)
        if ma is not None or mb is not None:
            return False
        # Also try stripping *both* sides — discovery and follow may render the
        # same metavar slot with different DB sequences (`($1 $0)` vs `($0 $1)`)
        # whenever the optimiser picks different `vis` orderings, so a strict
        # element-wise compare on the raw tuples would reject the alpha-equal
        # form even though the post-strip skeletons agree.
        sa, sb = split_meta_ho(a) or a, split_meta_ho(b) or b
        if (sa is not a or sb is not b) and isinstance(sa, tuple) and isinstance(sb, tuple) and len(sa) == len(sb):
            return go(sa, sb)
        if isinstance(a, str) and isinstance(b, str):
            return a == b
        if isinstance(a, tuple) and isinstance(b, tuple) and len(a) == len(b):
            return all(go(x, y) for x, y in zip(a, b))
        return False

    return go(a, b)


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

        # 2) Follow runs: both backends must reach a pattern alpha-equivalent
        #    to the discovery target. The match is liberal in one direction —
        #    `(?#k $a $b …)` (metavar HO-applied to bound vars) counts as
        #    equivalent to a bare `?#m`, since either form is an unrefined
        #    slot that the search can still specialise.
        # egg-stitch internally tries multiple surface-form variants of the
        # follow target (see `follow::follow_variants` in Rust), so the
        # script passes the discovered body once and only checks the result.
        results = {}
        for search in ("best-first", "smc"):
            out = outdir / f"{stem}.{search}.follow.out.json"
            print(f"\n=== {search} (follow) ===", file=sys.stderr)
            if not run_egg_stitch(binary, search, args.input, args.rewrites, out, passthrough, follow=target):
                print(f"{search}: search failed", file=sys.stderr)
                results[search] = False
                continue
            got = pattern_body(json.loads(out.read_text()))
            ok = got is not None and follow_equivalent(target, got)
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

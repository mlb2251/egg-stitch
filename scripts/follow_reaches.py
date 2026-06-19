#!/usr/bin/env python3
"""Verify that both egg-stitch search backends can reach the abstraction the
reference `stitch` compressor finds.

For a given input (and optional DSR file):
  1. Run `stitch` (self-cloned/built under `../stitch`) to discover an
     abstraction, rewriting its `#k` metavars to egg-stitch's `?#k` syntax.
  2. Re-run egg-stitch with `--follow <body>` under both `smc` and
     `best-first`, asserting each run's `library[0].pattern` body matches.

If stitch produces no abstraction (e.g. the corpus has no compressible
abstraction), the input is skipped with a pass — there is nothing to follow.

stitch can't take DSRs, so any `--rewrites` file is applied only to the
egg-stitch follow runs, not to discovery — the target itself is DSR-free.

Any args after `--` are forwarded to every egg-stitch invocation.
Exit 0 iff every follow run reaches the discovered pattern.
"""

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent

# stitch lives as a sibling clone of this repo (matching expts.run_models.stitch).
STITCH_DIR = (REPO.parent / "stitch").resolve()
STITCH_URL = "https://github.com/mlb2251/stitch.git"
# Cross-tool arity cap; mirror expts.bench.MAX_ARITY so the abstraction stitch
# finds here matches what the benchmark would run.
MAX_ARITY = 2

# Fixed at 2000 steps for both backends — the follow sweep is a CI diagnostic,
# not a quality search, so we want uniform, bounded runtime per input.
# `--max-arity` matches the `-a{MAX_ARITY}` stitch was run at: egg-stitch
# searches the same arity budget and should land on stitch's (possibly lower-
# arity) optimum on its own. (Passthrough may override it via `_drop_overridden`.)
SMC_DEFAULTS = ["--num-particles", "1000", "--num-steps", "2000", "--temperature", "1000", "--max-arity", str(MAX_ARITY)]
BF_DEFAULTS = ["--num-steps", "2000", "--max-arity", str(MAX_ARITY)]


def cargo_binary():
    """Build the release binary once; cheap when up-to-date."""
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=REPO, check=True)
    return REPO / "target" / "release" / "egg-stitch"


def stitch_binary():
    """Clone `../stitch` if missing, build the `compress` binary, return its path.

    The follow sweep is a compatibility diagnostic — egg-stitch must reproduce
    the abstraction the reference compressor finds — so it self-prepares the
    sibling clone rather than requiring a manual checkout, which is what lets CI
    run it with no extra setup step.
    """
    if not STITCH_DIR.exists():
        print(f"cloning stitch into {STITCH_DIR} ...", file=sys.stderr)
        subprocess.run(["git", "clone", "--depth", "1", STITCH_URL, str(STITCH_DIR)], check=True)
    subprocess.run(["cargo", "build", "--release", "--bin", "compress", "--quiet"], cwd=STITCH_DIR, check=True)
    return STITCH_DIR / "target" / "release" / "compress"


def language_from_passthrough(passthrough):
    """Return the `--language` value forwarded to egg-stitch (default lambda-calc)
    so stitch's cost flags match egg-stitch's reading of the corpus."""
    if "--language" in passthrough:
        i = passthrough.index("--language")
        if i + 1 < len(passthrough):
            return passthrough[i + 1]
    return "lambda-calc"


def stitch_follow_target(stitch_bin, input_path, language, output_path):
    """Run stitch on `input_path` and return its top abstraction as an egg-stitch
    follow pattern (stitch's `#k` metavars rewritten to `?#k`), or None when
    stitch finds nothing to abstract.

    Cost flags mirror expts.run_models.stitch: `op-children` is the no-apps
    weighting (non-app nodes cost 10000), everything else is apps-equal (all
    costs 1). `--no-curried-bodies --no-curried-metavars` are passed only on
    op-children, where curried (left-of-app) forms can't be expressed; on
    lambda-calc both stitch and egg-stitch can represent operator-position
    metavars, so we leave stitch unconstrained — forbidding them would only
    handicap stitch into a worse abstraction than egg-stitch finds.
    """
    no_apps = language == "op-children"
    cost = "10000" if no_apps else "1"
    cmd = [
        str(stitch_bin), str(input_path),
        "-i1", f"-a{MAX_ARITY}",
        "--out", str(output_path),
        "--silent", "--allow-single-task",
        "--cost-app", "1",
        "--cost-var", cost, "--cost-ivar", cost,
        "--cost-prim-default", cost, "--cost-lam", cost,
    ]
    if no_apps:
        cmd += ["--no-curried-bodies", "--no-curried-metavars"]
    print(f"$ {' '.join(cmd)}", file=sys.stderr)
    subprocess.run(cmd, check=True)
    abstractions = json.loads(Path(output_path).read_text()).get("abstractions") or []
    if not abstractions:
        return None
    return re.sub(r"(?<!\?)#", "?#", abstractions[0]["body"])


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
    stitch_bin = stitch_binary()

    with tempfile.TemporaryDirectory(prefix="follow_reaches-") as td:
        outdir = Path(td)
        stem = Path(args.input).stem

        # 1) Discovery run: the reference `stitch` compressor finds the
        #    abstraction egg-stitch's follow runs must then reproduce. stitch is
        #    deterministic, so the target is reproducible; it ignores DSRs, so
        #    `--rewrites` (if any) is applied only to the follow runs below.
        disc_out = outdir / f"{stem}.discovery.out.json"
        print("\n=== discovery (stitch) ===", file=sys.stderr)
        language = language_from_passthrough(passthrough)
        target = stitch_follow_target(stitch_bin, args.input, language, disc_out)
        if target is None:
            print(f"SKIP: stitch found no abstraction for {args.input} — nothing to follow")
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

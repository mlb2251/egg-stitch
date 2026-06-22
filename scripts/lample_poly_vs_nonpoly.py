#!/usr/bin/env python3
"""Live-vs-at-start DSR gap on poly vs non-poly Lample derivative corpora.

The physics.rewrites DSRs are purely arithmetic, so transcendental subterms
(sin/cos/exp/ln/...) are opaque to them. This asks whether keeping the DSRs
live during search still pays off once the corpus is full of such opaque
structure. We compare a size-matched (3000) pair:
  - non-poly: data/domains/lample-deriv/      (~82% transcendental)
  - poly:     first 3000 of lample-deriv-poly (no transcendentals)
For each corpus we run best-first with the physics DSRs in three modes
(no-DSR, DSRs-at-start, DSRs-live) and report the final cost plus the
live-vs-at-start gap. The two corpora are independent samples from the same
source, so the comparison is distributional, not program-for-program.

Env knobs: ABSTS (default 10), STEPS (10000), ARITY (3), TIMEOUT s (1800).
"""
import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "egg-stitch")
ABSTS = int(os.environ.get("ABSTS", 10))
STEPS = int(os.environ.get("STEPS", 10000))
ARITY = int(os.environ.get("ARITY", 3))
TIMEOUT = int(os.environ.get("TIMEOUT", 1800))

BASE_DSR = """\
add_comm: (@ (@ +. ?x) ?y) => (@ (@ +. ?y) ?x)
mul_comm: (@ (@ *. ?x) ?y) => (@ (@ *. ?y) ?x)
add_iden: (@ (@ +. ?x) 0.) => ?x
sub_iden: (@ (@ -. ?x) 0.) => ?x
mul_iden: (@ (@ *. ?x) 1.) => ?x
div_iden: (@ (@ /. ?x) 1.) => ?x
div_self: (@ (@ /. ?x) ?x) => 1.
pow_iden: (@ (@ power ?x) 1.) => ?x
"""
RW = "/tmp/physics_base.rw"
open(RW, "w").write(BASE_DSR)
TMP = tempfile.mkdtemp(prefix="polycmp_")


def make_inputs():
    """Write the size-matched non-poly / poly corpora; return [(name, path, n)]."""
    nonpoly = json.load(open(f"{ROOT}/data/domains/lample-deriv/all.json"))
    poly = json.load(open(f"{ROOT}/data/domains/lample-deriv-poly/all.json"))[:len(nonpoly)]
    out = []
    for name, progs in [("non-poly", nonpoly), ("poly", poly)]:
        p = f"{TMP}/{name}.json"
        json.dump(progs, open(p, "w"))
        out.append((name, p, len(progs)))
    return out


def run(inp, mode):
    """Run egg-stitch once; return final cost (last trajectory point) or None."""
    outp = f"{TMP}/{os.path.basename(inp)}.{mode}.json"
    cmd = [BIN, "-i", inp, "--output", outp, "--search", "best-first",
           "--language", "lambda-calc", "--max-arity", str(ARITY),
           "--num-abstractions", str(ABSTS), "--num-steps", str(STEPS)]
    if mode != "nodsr":
        cmd += ["-r", RW]
    if mode == "atstart":
        cmd.append("--only-use-dsrs-at-start")
    try:
        subprocess.run(cmd, cwd=ROOT, check=True, timeout=TIMEOUT,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    except subprocess.CalledProcessError as e:
        print(f"  FAIL {mode}: {e.stderr.decode()[-300:]}", file=sys.stderr)
        return None
    except subprocess.TimeoutExpired:
        print(f"  TIMEOUT {mode}", file=sys.stderr)
        return None
    traj = json.load(open(outp)).get("cost_at_end_of_each_iter") or []
    return traj[-1] if traj else None


def main():
    print(f"config: {ABSTS} absts, {STEPS} steps, arity {ARITY}, physics DSRs\n", flush=True)
    for name, inp, n in make_inputs():
        print(f"=== {name} ({n} programs) ===", flush=True)
        nd, a, l = run(inp, "nodsr"), run(inp, "atstart"), run(inp, "live")
        print(f"  final cost:  no-DSR={nd}  at-start={a}  live={l}")
        if a and l:
            print(f"  live-vs-at-start gap: {(a - l) / a * 100:+.2f}%  "
                  f"(positive = live DSRs compress better than at-start)")
        print(flush=True)


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Run `check_equiv.py` on every `*.out.json` under `data/expected_outputs/`.

Each fixture is run β-only by default. Fixtures that the search produced
*with* a DSR file get checked against those same DSRs via `RULES_BY_PATH`;
without them, β alone can't bridge cases like `(* 0 ?x) ≡ 0`.

Fixtures whose library has no `lambda` field are skipped internally by
`check_equiv.py` (lambda-free OpChildren runs).

The epfl-circuits domain is verified by `check_circuit_equiv.py` instead
(exhaustive boolean truth-table equivalence), which is definitive for its
<=K-input cones where rule-saturation can't always re-derive the equivalence.

Exit 0 iff every applicable file checks out.
"""

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
CHECKER = HERE / "check_equiv.py"
CIRCUIT_CHECKER = HERE / "check_circuit_equiv.py"
ROOT = REPO / "data" / "expected_outputs"

# Fixtures where the search was run with `-r <rules>` and β alone is not
# enough to bridge the DSR-mediated equivalence in the rewritten programs.
# Keep in sync with the suites that bless these fixtures: the stitch/arith/
# conditional/fv-overapprox/test entries come from `tests/stitch_compat_test.rs`,
# the `cogsci/*.dsr` ones from `tests/cogsci_bfs_test.rs`, and the
# `list`/`physics` ones from `tests/dreamcoder_bfs_test.rs`.
RULES_BY_REL = {
    "fv-overapprox/annihilator.out.json": "data/domains/fv-overapprox/annihilator.rewrites",
    "stitch/nested.out.json": "data/domains/stitch/nested.rewrites",
    "conditional/if_branch_unify.out.json": "data/domains/conditional/if_branch.rewrites",
    "conditional/if_branch_unify.cap5.out.json": "data/domains/conditional/if_branch.rewrites",
    "conditional/if_branch_unify.cap10.out.json": "data/domains/conditional/if_branch.rewrites",
    "test/crossed_wrap_collapse.out.json": "data/test/crossed_wrap_collapse.rewrites",
    "cogsci/dials.dsr.out.json": "data/domains/cogsci/dials.rewrites",
    "cogsci/dials.dsr-mfe3.out.json": "data/domains/cogsci/dials.rewrites",
    "cogsci/furniture.dsr.out.json": "data/domains/cogsci/furniture.rewrites",
    "cogsci/furniture.dsr-mfe3.out.json": "data/domains/cogsci/furniture.rewrites",
    "cogsci/nuts-bolts.dsr.out.json": "data/domains/cogsci/nuts-bolts.rewrites",
    "cogsci/nuts-bolts.dsr-mfe3.out.json": "data/domains/cogsci/nuts-bolts.rewrites",
    "cogsci/wheels.dsr.out.json": "data/domains/cogsci/wheels.rewrites",
    "cogsci/wheels.dsr-mfe3.out.json": "data/domains/cogsci/wheels.rewrites",
    "list/list.dsr.out.json": "data/domains/list/list.rewrites",
    "physics/physics.dsr.out.json": "data/domains/physics/physics.rewrites",
    "simple-arithmetic/const_fold.out.json": "data/domains/simple-arithmetic/const_fold.rewrites",
    "simple-arithmetic/const_fold_integers.out.json": "data/domains/simple-arithmetic/const_fold_integers.rewrites",
    "simple-arithmetic/const_fold_floats.out.json": "data/domains/simple-arithmetic/const_fold_floats.rewrites",
    "simple-arithmetic/const_fold_int_as_float.out.json": "data/domains/simple-arithmetic/const_fold_int_as_float.rewrites",
    "simple-arithmetic/fold_after_rewrite.out.json": "data/domains/simple-arithmetic/fold_after_rewrite.rewrites",
    "simple-arithmetic/fold_ops_trig.out.json": "data/domains/simple-arithmetic/fold_ops_trig.rewrites",
    "simple-arithmetic/fold_ops_restrict.out.json": "data/domains/simple-arithmetic/fold_ops_restrict.rewrites",
    "simple-arithmetic/fold_round6.out.json": "data/domains/simple-arithmetic/fold_round6.rewrites",
    "simple-arithmetic/fold_round3.out.json": "data/domains/simple-arithmetic/fold_round3.rewrites",
    "test/arith_unify.out.json": "data/test/arith.rewrites",
    "test/converge_tower.out.json": "data/test/converge_tower.rewrites",
    "test/nested_loop_tower.out.json": "data/test/nested_loop_tower.rewrites",
    "test/if_unify.out.json": "data/test/if.rewrites",
    # molecule domain (op-children): re-rooting / commutativity DSRs are what
    # make the rewritten trees equivalent, so β alone can't bridge them. Blessed
    # by the `molecules_*` cases in `tests/stitch_compat_test.rs`.
    "test/ethanol_two_rootings.out.json": "data/domains/molecules/molecules.rewrites",
    "domains/molecules/molecules.out.json": "data/domains/molecules/molecules.rewrites",
    "molecules/scramble/hexyl.scram.out.json": "data/domains/molecules/molecules.rewrites",
    "molecules/scramble/ester.scram.out.json": "data/domains/molecules/molecules.rewrites",
    "molecules/scramble/glycol.scram.out.json": "data/domains/molecules/molecules.rewrites",
}

# The plain-`op-children` half of the op-children-db contrast deliberately bakes
# the symbol `$0` into the body (`(f (g (h ?#0)) $0)`). check_equiv reads `$0` as
# a De Bruijn variable, so it sees an unsound rewrite — which is exactly the
# unsoundness `op-children-db` fixes by banning free DB vars from the body. The
# fixture only exists to pin that contrast, so skip it in the equivalence sweep;
# the snapshot test keeps it regression-locked. (epfl-circuits fixtures aren't
# skipped here -- `main` routes them to check_circuit_equiv.py instead.)
SKIP_REL = {"test/op_children_db_free_var.plain.out.json"}

# The cogsci `*.algebra` fixtures use our algebraic drawing rewrites
# (drawings.rewrites), which are non-confluent and node-exploding (matmul,
# overlay/arith commutativity, repeat-unroll) — the same blowup that forces
# `--iter-limit` on the real search. `check_equiv` saturates without such a cap,
# so it bails on the node limit (false negatives) and is brutally slow over the
# 250-program corpus. These fixtures are instead pinned EXACTLY by the
# `*_algebra` snapshot cases in `tests/cogsci_bfs_test.rs`.
SKIP_RELS = {
    "cogsci/dials.algebra.out.json",
    "cogsci/furniture.algebra.out.json",
    "cogsci/nuts-bolts.algebra.out.json",
    "cogsci/wheels.algebra.out.json",
}


def is_circuit(rel):
    return rel.startswith("epfl-circuits/")


def main():
    paths = sorted(ROOT.rglob("*.out.json"))
    if not paths:
        print(f"no *.out.json under {ROOT}", file=sys.stderr)
        sys.exit(1)
    overall = 0
    # epfl-circuits: exhaustive boolean-equivalence check (see check_circuit_equiv.py).
    circuits = [p for p in paths if is_circuit(str(p.relative_to(ROOT)))]
    if circuits:
        print(f"$ check_circuit_equiv.py <{len(circuits)} files>")
        res = subprocess.run([sys.executable, str(CIRCUIT_CHECKER), *map(str, circuits)], cwd=REPO)
        if res.returncode != 0:
            overall = res.returncode
    # Everything else: β-only or rule-mediated equivalence via check_equiv.
    # Group by rewrites file (None for β-only) so each batch becomes one call.
    batches = {}
    for p in paths:
        rel = str(p.relative_to(ROOT))
        if rel in SKIP_RELS:
            print(f"skip (oracle-intractable, pinned by snapshot test): {rel}")
            continue
        if rel in SKIP_REL or is_circuit(rel):
            continue
        rules = RULES_BY_REL.get(rel)
        batches.setdefault(rules, []).append(p)
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

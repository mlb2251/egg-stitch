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

# epfl-circuits (op-children AIG cones): the factoring DSRs (De Morgan +
# distributivity) are what make the rewritten cones equivalent, so β alone can't
# bridge them -- the `<circuit>.factoring.live` fixtures are checked against that
# rule file (the no-rules `<circuit>.baseline` ones stay β-only). Blessed by
# `tests/epfl_circuits_test.rs`.
RULES_BY_REL.update({
    f"epfl-circuits/{p.name}": "data/domains/epfl-circuits/and_or_demorgan_factor.rewrites"
    for p in sorted((ROOT / "epfl-circuits").glob("*.factoring.live.out.json"))
})

# The dsrs-only-at-start fixtures canonicalise the corpus deeply before
# abstracting, so re-deriving the equivalence needs far more saturation than is
# practical here (30 iters isn't close, and raising it is too slow for CI). Skip
# them in the equivalence sweep; they stay regression-locked by the snapshot test.
SKIP_REL = {
    str(p.relative_to(ROOT))
    for p in (ROOT / "epfl-circuits").glob("*.factoring.at-start.out.json")
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
        if rel in SKIP_REL:
            continue
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

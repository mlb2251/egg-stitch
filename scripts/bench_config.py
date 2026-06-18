"""Single source of truth for bench_pr's search hyperparameters.

bench_pr drives two searches per target. SMC keeps a fixed step / particle /
temperature budget. Best-first no longer uses one global step limit: each
target instead gets a forced-expansion cap, so the heap self-drains over the
``{forced ≤ cap}`` subspace and the search runs to *convergence* rather than
being clipped at an arbitrary expansion count. Targets a cap can't bound keep a
step limit. Caps and limits are tuned so each target finishes in roughly the
wall-clock of the former 5000-step runs.

Tuning notes (best-first, with DSRs, summed over a target's files): the cap is
set to the largest value before the {forced ≤ cap} subspace jumps (and the
search blows past ~1s), so each run converges in roughly a second.
    nuts-bolts  cap 11  -> 0.43s converged   (old 5000-step clip: 1.16s)
    dials       cap  8  -> 0.32s converged   (old clip: 0.33s)
    furniture   cap 34  -> 0.73s converged    (5638 exps; cap 36 jumps to 2.1s)
    wheels      cap 33  -> 0.74s converged    (4514 exps; cap 36 jumps to 2.2s)
    list/physics        -> already converge within the step limit (<0.6s each)
    molecules           -> forced-expansion doesn't separate their productive
                           patterns, so a cap can't bound the search; keep the
                           step limit (~2-3s/family, same as now).
Without DSRs every domain is cost-balanced, so the cap is a no-op there and the
search converges in <0.05s regardless of which cutoff is used.
"""

from expts.run_models import OursBf, OursSmc

# ── SMC ──────────────────────────────────────────────────────────────────────
SMC_STEPS = 100
SMC_PARTICLES = 1000
SMC_TEMP = 1000.0

# ── Best-first ───────────────────────────────────────────────────────────────
# Per-target best-first runner, keyed by the domain / molecule-family name
# bench_pr uses. A forced-expansion cap makes the search self-drain to
# convergence; a step limit is for targets a cap can't bound (the molecule
# scrambles) or that already converge within it (list, physics). Every target
# bench_pr runs must appear here — there is no default.
BF_RUNNERS = {
    "nuts-bolts": OursBf(num_steps=None, max_forced_expansion=11),
    "dials": OursBf(num_steps=None, max_forced_expansion=8),
    "furniture": OursBf(num_steps=None, max_forced_expansion=34),
    "wheels": OursBf(num_steps=None, max_forced_expansion=33),
    "list": OursBf(num_steps=5000),
    "physics": OursBf(num_steps=5000),
    "hexyl": OursBf(num_steps=5000),
    "ester": OursBf(num_steps=5000),
    "glycol": OursBf(num_steps=5000),
}


def smc_runner() -> OursSmc:
    """The SMC runner, shared across all targets."""
    return OursSmc(
        num_steps=SMC_STEPS, num_particles=SMC_PARTICLES, temperature=SMC_TEMP
    )

from expts.run_models import OursBf, OursSmc

# ── SMC ──────────────────────────────────────────────────────────────────────
SMC_STEPS = 100
SMC_PARTICLES = 1000
SMC_TEMP = 1000.0


MOLECULE_STEPS = 500 # 5000

# Forward --no-opt-seen (disable the best-first seen-set dedup) on every BF cell.
# Flip to False to benchmark with the seen-set on. See OursBf / src/lib.rs.
NO_OPT_SEEN = True

BF_RUNNERS = {
    # max_forced_expansion tuned so each target's with-DSRs best-first run takes
    # ~2s (nuts-bolts has a convergence cliff between 11 (~0.5s) and 12 (~2.5s),
    # so 12 is the closest reachable to 2s).
    "nuts-bolts": OursBf(num_steps=None, max_forced_expansion=12, no_opt_seen=NO_OPT_SEEN),
    "dials": OursBf(num_steps=None, max_forced_expansion=24, no_opt_seen=NO_OPT_SEEN),
    "furniture": OursBf(num_steps=None, max_forced_expansion=240, no_opt_seen=NO_OPT_SEEN),
    "wheels": OursBf(num_steps=None, max_forced_expansion=250, no_opt_seen=NO_OPT_SEEN),
    "list": OursBf(num_steps=5000, no_opt_seen=NO_OPT_SEEN),
    "physics": OursBf(num_steps=5000, no_opt_seen=NO_OPT_SEEN),
    "hexyl": OursBf(num_steps=MOLECULE_STEPS, no_opt_seen=NO_OPT_SEEN),
    "ester": OursBf(num_steps=MOLECULE_STEPS, no_opt_seen=NO_OPT_SEEN),
    "glycol": OursBf(num_steps=MOLECULE_STEPS, no_opt_seen=NO_OPT_SEEN),
}


def smc_runner() -> OursSmc:
    """The SMC runner, shared across all targets."""
    return OursSmc(
        num_steps=SMC_STEPS, num_particles=SMC_PARTICLES, temperature=SMC_TEMP
    )

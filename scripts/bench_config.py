from expts.run_models import OursBf, OursSmc

# ── SMC ──────────────────────────────────────────────────────────────────────
SMC_STEPS = 100
SMC_PARTICLES = 1000
SMC_TEMP = 1000.0


MOLECULE_STEPS = 500 # 5000

BF_RUNNERS = {
    # max_forced_expansion tuned so each target's with-DSRs best-first run takes
    # ~2s (nuts-bolts has a convergence cliff between 11 (~0.5s) and 12 (~2.5s),
    # so 12 is the closest reachable to 2s).
    "nuts-bolts": OursBf(num_steps=None, max_forced_expansion=12),
    "dials": OursBf(num_steps=None, max_forced_expansion=24),
    "furniture": OursBf(num_steps=None, max_forced_expansion=240),
    "wheels": OursBf(num_steps=None, max_forced_expansion=250),
    "list": OursBf(num_steps=5000),
    "physics": OursBf(num_steps=5000),
    "hexyl": OursBf(num_steps=MOLECULE_STEPS),
    "ester": OursBf(num_steps=MOLECULE_STEPS),
    "glycol": OursBf(num_steps=MOLECULE_STEPS),
}


def smc_runner() -> OursSmc:
    """The SMC runner, shared across all targets."""
    return OursSmc(
        num_steps=SMC_STEPS, num_particles=SMC_PARTICLES, temperature=SMC_TEMP
    )

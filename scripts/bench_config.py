from expts.run_models import OursBf, OursSmc

# ── SMC ──────────────────────────────────────────────────────────────────────
SMC_STEPS = 100
SMC_PARTICLES = 1000
SMC_TEMP = 1000.0

BF_RUNNERS = {
    "nuts-bolts": OursBf(num_steps=None, max_forced_expansion=12),
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

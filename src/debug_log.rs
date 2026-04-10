use serde::Serialize;

/// Full debug trace of an SMC run, one entry per step.
#[derive(Serialize)]
pub struct DebugLog {
    pub original_size: usize,
    pub num_particles: usize,
    pub temperature: f64,
    pub steps: Vec<StepLog>,
}

/// Per-step snapshot of all particles.
#[derive(Serialize)]
pub struct StepLog {
    pub step: usize,
    /// Each particle after the propose (expand) phase.
    pub particles: Vec<ParticleLog>,
    /// Indices chosen during resampling (length = num_particles).
    pub resample_indices: Vec<usize>,
    /// Global best cost so far (after this step).
    pub best_cost: Option<usize>,
    /// Global best pattern so far.
    pub best_pattern: Option<String>,
}

/// Snapshot of a single particle within a step.
#[derive(Serialize)]
pub struct ParticleLog {
    pub pattern: String,
    pub num_matches: usize,
    pub arity: usize,
    pub cost: usize,
    pub weight: f64,
}

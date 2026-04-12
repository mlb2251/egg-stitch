use serde::Serialize;

use crate::search::SearchState;

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
    pub particles: Vec<ParticleLog>,
    pub resample_indices: Vec<usize>,
    pub best_cost: Option<usize>,
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

/// Builds a ParticleLog for each particle (pre-resample snapshot).
pub fn build_particle_logs(states: &[SearchState], costs: &[usize], weights: &[f64]) -> Vec<ParticleLog> {
    states
        .iter()
        .enumerate()
        .map(|(i, s)| ParticleLog {
            pattern: s.pattern.to_string(),
            num_matches: s.matches.len(),
            arity: s.pattern.vars.len(),
            cost: costs[i],
            weight: weights[i],
        })
        .collect()
}

/// Appends a debug step log if debug mode is on.
#[allow(clippy::too_many_arguments)]
pub fn log_debug_step(debug: bool, steps: &mut Vec<StepLog>, step: usize, states: &[SearchState], costs: &[usize], weights: &[f64], best: &Option<(usize, SearchState)>, resample_indices: &[usize]) {
    if !debug {
        return;
    }
    steps.push(StepLog {
        step,
        particles: build_particle_logs(states, costs, weights),
        resample_indices: resample_indices.to_vec(),
        best_cost: best.as_ref().map(|(c, _)| *c),
        best_pattern: best.as_ref().map(|(_, s)| s.pattern.to_string()),
    });
}

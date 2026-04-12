use serde::Serialize;

use crate::search::SearchState;

/// A replay log: search config + a sequence of (pattern, action) pairs that
/// can be replayed in the interactive WASM viewer to reconstruct a search run.
#[derive(Serialize, serde::Deserialize)]
pub struct ReplayLog {
    pub config: ReplayConfig,
    pub steps: Vec<ReplayStep>,
}

/// Search configuration stored alongside a replay log so the viewer can
/// reproduce the exact same settings.
#[derive(Serialize, serde::Deserialize)]
pub struct ReplayConfig {
    pub priority: String,
    pub budget: usize,
    pub max_arity: usize,
}

/// One expansion decision in a replay log.
#[derive(Serialize, serde::Deserialize)]
pub struct ReplayStep {
    /// Pattern string of the node that was expanded.
    pub pattern: String,
    /// Action string describing the expansion (e.g. "expand #0 := op/2").
    /// None means "expand all successors" (deterministic best-first).
    /// Some(action) means a specific action was chosen (stochastic search).
    pub action: Option<String>,
    /// Expected number of e-class matches at the time of expansion (for replay validation).
    pub num_matches: usize,
    /// Expected cost at the time of expansion (for replay validation).
    pub cost: usize,
}

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

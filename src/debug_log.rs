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

/// Full debug trace of a best-first enumerative search: the explored search tree
/// plus the order in which nodes were popped for expansion and which one won.
#[derive(Serialize)]
pub struct SearchTreeLog {
    pub original_size: usize,
    pub nodes: Vec<TreeNodeLog>,
    /// Node ids in the order they were popped from the heap and expanded.
    pub expansion_order: Vec<usize>,
    /// Id of the lowest-cost node ever seen (or None if the tree is degenerate).
    pub best_node: Option<usize>,
}

/// One node in the best-first search tree.
#[derive(Serialize)]
pub struct TreeNodeLog {
    pub id: usize,
    pub parent: Option<usize>,
    /// Human-readable label for the move that produced this node from its parent.
    pub action: Option<String>,
    pub pattern: String,
    pub arity: usize,
    pub pattern_size: usize,
    pub num_matches: usize,
    pub cost: usize,
    /// Whether this node was actually popped and expanded (vs. just enqueued).
    pub expanded: bool,
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

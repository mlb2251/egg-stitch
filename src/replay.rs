use serde::Serialize;

use crate::best_first::{InteractiveSearch, SearchPriority};

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
    pub action: Option<String>,
    /// Expected number of e-class matches at the time of expansion (for replay validation).
    pub num_matches: usize,
    /// Expected cost at the time of expansion (for replay validation).
    pub cost: usize,
}

/// Parse a replay log JSON string, apply its config, and run all steps.
/// Returns the config so the caller can update UI.
pub fn replay_from_json(search: &mut InteractiveSearch, json: &str) -> Result<ReplayConfig, String> {
    let log: ReplayLog = serde_json::from_str(json).map_err(|e| format!("failed to parse replay: {e}"))?;
    if let Some(strategy) = SearchPriority::parse(&log.config.priority) {
        search.set_priority(strategy);
    }
    search.set_max_arity(log.config.max_arity);
    replay(search, &log.steps)?;
    Ok(log.config)
}

/// Replay a sequence of steps. Returns `Ok(steps_replayed)` on success,
/// or `Err(message)` on the first mismatch/missing pattern.
pub fn replay(search: &mut InteractiveSearch, steps: &[ReplayStep]) -> Result<usize, String> {
    for (i, step) in steps.iter().enumerate() {
        let node_id = match search.find_unexpanded_by_pattern(&step.pattern) {
            Some(id) => id,
            None => {
                if search.has_pattern(&step.pattern) {
                    continue; // already expanded, skip
                }
                return Err(format!("step {}: pattern not found: {}", i + 1, step.pattern));
            }
        };
        let (matches, cost) = search.node_matches_and_cost(node_id);
        if matches != step.num_matches {
            return Err(format!("step {}: matches mismatch for {}: got {} expected {}", i + 1, step.pattern, matches, step.num_matches));
        }
        if cost != step.cost {
            return Err(format!("step {}: cost mismatch for {}: got {} expected {}", i + 1, step.pattern, cost, step.cost));
        }
        search.expand_node(node_id);
    }
    Ok(steps.len())
}

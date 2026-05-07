use serde::Serialize;

/// Results for a single abstraction found during a run.
#[derive(Serialize)]
pub struct AbstractionResult {
    pub pattern: String,
    pub arity: usize,
    pub pattern_size: usize,
    pub num_matches: usize,
    /// Sum of corpus usage counts across all matching e-classes.
    pub usage_matches: usize,
    /// Approximate cost: `corpus_size_before - pattern_size * (usage_matches - 1)`.
    pub approx_cost: i64,
    pub num_steps_run: usize,
    pub num_expansions: Option<usize>,
    pub best_iteration: Option<usize>,
    /// Successive "new best" events recorded during best-first search.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub best_history: Option<Vec<crate::best_first::BestHistoryEntry>>,
    pub rewritten_programs: Vec<String>,
}

/// Full JSON-serializable record of a single run.
#[derive(Serialize)]
pub struct RunResult {
    pub timestamp: f64,
    /// Which search algorithm produced this run ("smc" or "best-first").
    pub search: String,
    pub input_file: String,
    pub rules_file: Option<String>,
    pub elapsed_secs: f64,
    /// Minimum AST size of the corpus before any rewrite rules were applied.
    pub initial_cost: usize,
    /// Minimum AST size of the corpus after rewrite rules were applied (before search).
    pub cost_after_rewrites: usize,
    /// Total cost after all abstractions (corpus size + sum of all pattern sizes).
    pub final_cost: Option<usize>,
    pub compression_ratio: Option<f64>,
    /// Filename of the debug log (in the same directory), if debug logging was enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_log_file: Option<String>,
    /// All abstractions found, in order (each stacks on the previous).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub library: Vec<AbstractionResult>,
}

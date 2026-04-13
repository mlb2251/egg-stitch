use serde::Serialize;

/// Full JSON-serializable record of a single SMC run.
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
    /// Minimum AST size of the corpus after rewrite rules were applied (before SMC).
    pub cost_after_rewrites: usize,
    pub final_cost: Option<usize>,
    pub compression_ratio: Option<f64>,
    pub pattern: Option<String>,
    pub arity: Option<usize>,
    pub pattern_size: Option<usize>,
    pub num_expansions: Option<usize>,
    pub num_matches: Option<usize>,
    /// Sum of corpus usage counts across all matching e-classes.
    pub usage_matches: Option<usize>,
    /// Approximate cost estimate: `cost_after_rewrites - pattern_size * (usage_matches - 1)`.
    pub approx_cost: Option<i64>,
    pub best_iteration: Option<usize>,
    pub num_steps_run: usize,
    pub rewritten_programs: Option<Vec<String>>,
    /// Filename of the debug log (in the same directory), if debug logging was enabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_log_file: Option<String>,
}

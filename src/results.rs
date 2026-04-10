use serde::Serialize;

/// The result of a single egg-stitch run, suitable for JSON export.
#[derive(Serialize)]
pub struct RunResult {
    /// Unix epoch seconds at which this run finished writing its result.
    pub timestamp: f64,
    pub input_file: String,
    pub rules_file: Option<String>,
    pub elapsed_secs: f64,
    pub initial_cost: usize,
    pub final_cost: Option<usize>,
    pub compression_ratio: Option<f64>,
    pub pattern: Option<String>,
    pub arity: Option<usize>,
    pub pattern_size: Option<usize>,
    /// Number of distinct e-classes where the best pattern matches.
    pub num_matches: Option<usize>,
    /// Sum of corpus usage counts across all matching e-classes.
    pub usage_matches: Option<usize>,
    /// Approximate cost estimate: `initial_cost - pattern_size * (usage_matches - 1)`.
    pub approx_cost: Option<i64>,
    pub num_expansions: Option<usize>,
    pub best_iteration: Option<usize>,
    pub num_steps_run: usize,
    pub rewritten_programs: Option<Vec<String>>,
}

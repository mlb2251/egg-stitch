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
    pub num_expansions: Option<usize>,
    pub best_iteration: Option<usize>,
    pub num_steps_run: usize,
    pub rewritten_programs: Option<Vec<String>>,
}

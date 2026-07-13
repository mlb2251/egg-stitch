use serde::Serialize;

/// Results for a single abstraction found during a run.
#[derive(Serialize)]
pub struct AbstractionResult {
    pub pattern: String,
    /// Closed-lambda form of the abstraction: inlining a call site
    /// `(fn_N a_0 … a_{k-1})` against this and β-reducing recovers the original
    /// captured term.
    pub lambda: String,
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
    /// Cost at each iteration of the search.
    pub cost_at_end_of_each_iter: Option<Vec<usize>>,
    /// Total cost after all abstractions (corpus size + sum of all pattern sizes). Always cost_at_end_of_each_iter[-1]
    pub final_cost: Option<usize>,
    pub compression_ratio: Option<f64>,
    /// Best-first heap size when each search iteration stopped, one entry per
    /// iteration run (so it can exceed `library.len()` by one when the final
    /// iteration finds no abstraction). `0` means that iteration's frontier was
    /// exhausted (the search converged); a non-zero value means it stopped at the
    /// `num_steps` cap. `None` for SMC (no heap).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heap_sizes_at_end: Option<Vec<usize>>,
    /// Programs as loaded from the input file (verbatim s-expression strings).
    pub original_programs: Vec<String>,
    /// Programs after all abstractions have been applied. Equal to `original_programs`
    /// if no abstractions were found; otherwise the rewritten corpus from the last
    /// abstraction in `library`.
    pub rewritten_programs: Vec<String>,
    /// All abstractions found, in order (each stacks on the previous).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub library: Vec<AbstractionResult>,
    /// Elapsed times for each iteration of the search.
    pub iteration_times: Vec<f64>,
}

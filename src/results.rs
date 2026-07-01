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
    /// Rich search diagnostics for the iteration that produced this abstraction
    /// (best-first only; `None` for SMC). See [`SearchStats`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_stats: Option<SearchStats>,
}

/// Everything the best-first `═══ STATS ═══` block prints, serialized so
/// downstream tooling aggregates structured numbers instead of scraping stdout.
/// One of these is attached per abstraction iteration. `None`-valued on the SMC
/// path, which reports its own diagnostics elsewhere.
#[derive(Serialize)]
pub struct SearchStats {
    /// Heap pops performed (`num_expansions`).
    pub expansions: usize,
    /// Total search-tree nodes created.
    pub nodes_created: usize,
    /// Frontier size when the loop stopped (0 = converged, else hit a cutoff).
    pub heap_size_at_end: usize,
    /// `compute_cost_and_select` calls and their summed wall-clock.
    pub cost_calls: usize,
    pub cost_secs: f64,
    /// Reuse-dominance short-circuit fires.
    pub dominance_hits: usize,
    /// Useless-frozen prunes and useless-non-frozen inline short-circuits.
    pub useless_frozen_hits: usize,
    pub useless_inline_hits: usize,
    /// Lower-bound pruner prunes and their summed wall-clock.
    pub lower_bound_hits: usize,
    pub lower_bound_secs: f64,
    /// Total wall-clock of the search loop.
    pub total_search_secs: f64,
    /// Seen-set diagnostics; `None` when `--no-opt-seen`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seen: Option<SeenStats>,
}

/// Seen-set diagnostics: both the `map` side and the shadow seen-egraph
/// (membership counts + the end-of-run saturation audit). Mirrors the
/// `── seen-egraph audit ──` stdout block. See [`crate::search::SeenTracker`].
#[derive(Serialize)]
pub struct SeenStats {
    /// Distinct patterns recorded, and the `map`-side skip verdicts.
    pub size: usize,
    pub hits: usize,
    pub egraph_hits: usize,
    pub exact_hits: usize,
    pub full_dom_hits: usize,
    /// Map/lookup bookkeeping wall-clock (excludes recexpr build + saturation).
    pub map_secs: f64,
    /// Lifted DSR count and the saturation cadence in effect.
    pub num_rules: usize,
    pub saturate_each: bool,
    pub saturate_every: usize,
    pub saturate_dynamic: bool,
    pub dynamic_effects: usize,
    pub dynamic_noeffects: usize,
    /// Which side drives the skip decision: `"egraph"` or `"map"`.
    pub decider: String,
    /// Per-insert saturation: summed wall-clock, run count, and egg's own
    /// search/apply/rebuild split; plus the frozen-recexpr build time.
    pub per_insert_saturate_secs: f64,
    pub saturate_calls: usize,
    pub egraph_search_secs: f64,
    pub egraph_apply_secs: f64,
    pub egraph_rebuild_secs: f64,
    pub recexpr_secs: f64,
    /// End-of-run audit-pass saturation outcome.
    pub audit_iterations: usize,
    pub audit_applications: usize,
    pub audit_stop_reason: String,
    /// Seen-egraph size before/after the audit saturation, with the distinct
    /// top-level (`Root`) class counts.
    pub nodes_before: usize,
    pub classes_before: usize,
    pub root_classes_before: usize,
    pub nodes_after: usize,
    pub classes_after: usize,
    pub root_classes_after: usize,
    /// Genuine inserts vs the true DSR-classes after full saturation; `deferred`
    /// is the gap batching/off left un-deduped (precision loss, never unsound).
    pub unique_inserted: usize,
    pub total_inserted: usize,
    pub deferred: usize,
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

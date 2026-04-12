use clap::{Parser, ValueEnum};

use egg_stitch::best_first::{BestFirstConfig, SearchPriority, best_first};
use egg_stitch::cost;
use egg_stitch::io;
use egg_stitch::results;
use egg_stitch::search::{self, SearchState};
use egg_stitch::smc::{SmcConfig, smc};

/// Which search algorithm to run (CLI wrapper).
#[derive(ValueEnum, Clone, Debug)]
enum CliSearchKind {
    Smc,
    BestFirst,
}

/// Heap priority (CLI wrapper with ValueEnum derive).
#[derive(ValueEnum, Clone, Debug)]
enum CliPriority {
    Cost,
    DepthFirst,
    BreadthFirst,
    MostMatches,
}

impl From<CliPriority> for SearchPriority {
    fn from(p: CliPriority) -> Self {
        match p {
            CliPriority::Cost => SearchPriority::Cost,
            CliPriority::DepthFirst => SearchPriority::DepthFirst,
            CliPriority::BreadthFirst => SearchPriority::BreadthFirst,
            CliPriority::MostMatches => SearchPriority::MostMatches,
        }
    }
}

/// E-graph based program synthesis.
#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// Search algorithm to use.
    #[arg(long, value_enum, default_value_t = CliSearchKind::Smc)]
    search: CliSearchKind,

    /// Path to the input JSON file containing programs.
    #[arg(short, long, default_value = "data/domains/cogsci/dials.json")]
    input: String,

    /// Path to rewrite rules file.
    #[arg(short, long)]
    rules: Option<String>,

    /// Follow pattern to constrain particle expansion.
    #[arg(short, long)]
    follow: Option<String>,

    /// Number of particles.
    #[arg(long, default_value_t = 10_000)]
    num_particles: usize,

    /// Number of SMC steps.
    #[arg(long, default_value_t = 1000)]
    num_steps: usize,

    /// Softmax temperature for resampling weights.
    #[arg(long, default_value_t = 100.0)]
    temperature: f64,

    /// Stop after this many steps with no improvement.
    #[arg(long, default_value_t = 50)]
    dead_runs: usize,

    /// Maximum arity of patterns to consider as "best".
    #[arg(long, default_value_t = 1000)]
    max_arity: usize,

    /// Heap priority for best-first search.
    #[arg(long, value_enum, default_value_t = CliPriority::Cost)]
    priority: CliPriority,

    /// Weight match selection by usage count during expansion.
    #[arg(long, default_value_t = false)]
    weight_by_usage: bool,

    /// Probability of attempting variable reuse during expansion.
    #[arg(long, default_value_t = 0.5)]
    p_reuse: f64,

    /// Enable slow rewrite check (assert fast == slow computation).
    #[arg(long, default_value_t = false)]
    check_slow: bool,

    /// Path to write JSON output.
    #[arg(short, long)]
    output: Option<String>,

    /// Enable detailed debug logging of all particles at each SMC step.
    #[arg(long, default_value_t = false)]
    debug_log: bool,

    /// Print per-step progress output (top particles, follow stats, etc.).
    #[arg(long, default_value_t = false)]
    verbose: bool,
}

fn main() {
    let args = Args::parse();
    let start = std::time::Instant::now();

    let rules = args.rules.as_deref();
    let (egraph, root, cost_before_rewrites) = io::load_egraph(&args.input, rules);

    let (shared, original_size) = search::setup_search(egraph, root, args.follow.as_deref(), args.weight_by_usage, args.p_reuse, args.check_slow);

    let (best, best_found_at, num_steps_run, debug_log_json): (Option<(usize, SearchState)>, Option<usize>, usize, Option<String>) = match args.search {
        CliSearchKind::Smc => {
            let config = SmcConfig {
                num_particles: args.num_particles,
                num_steps: args.num_steps,
                temperature: args.temperature,
                dead_runs: args.dead_runs,
                max_arity: args.max_arity,
                verbose: args.verbose,
                debug: args.debug_log,
            };
            let r = smc(&shared, root, original_size, &config);
            let json = r.debug_log.as_ref().map(|d| serde_json::to_string(d).expect("Failed to serialize debug log"));
            (r.best, r.best_found_at, r.num_steps_run, json)
        }
        CliSearchKind::BestFirst => {
            let config = BestFirstConfig {
                budget: args.num_steps,
                max_arity: args.max_arity,
                debug: args.debug_log,
                priority: args.priority.into(),
            };
            let initial = SearchState::new(&shared);
            let r = best_first(&shared, root, original_size, initial, &config);
            let json = r.tree_log.as_ref().map(|d| serde_json::to_string(d).expect("Failed to serialize tree log"));
            (r.best, r.best_found_at, r.num_expansions, json)
        }
    };

    let elapsed_secs = start.elapsed().as_secs_f64();

    let (final_cost, compression_ratio, pattern, arity, pattern_size, num_matches, usage_matches, approx_cost, rewritten_programs) = match &best {
        Some((c, state)) => {
            let pat_size = cost::compute_pattern_size(&state.pattern);
            let usage_counts = search::compute_usage_counts(&shared.egraph, root);
            let um: usize = state.matches.iter().map(|m| usage_counts.get(&m.root_eclass).copied().unwrap_or(1)).sum();
            let appx = original_size as i64 - pat_size as i64 * (um as i64 - 1);
            (
                Some(*c),
                Some(original_size as f64 / *c as f64),
                Some(state.pattern.to_string()),
                Some(state.pattern.vars.len()),
                Some(pat_size),
                Some(state.matches.len()),
                Some(um),
                Some(appx),
                Some(cost::extract_rewritten_programs(&shared.egraph, root, state)),
            )
        }
        None => (None, None, None, None, None, None, None, None, None),
    };

    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);

    let debug_log_file = if let (Some(json), Some(output_path)) = (debug_log_json, &args.output) {
        let debug_path = output_path.replace(".json", "_debug.json");
        std::fs::write(&debug_path, json).expect("Failed to write debug log");
        println!("wrote debug log to {}", debug_path);
        Some(std::path::Path::new(&debug_path).file_name().unwrap().to_string_lossy().into_owned())
    } else {
        None
    };

    let search_kind = match args.search {
        CliSearchKind::Smc => "smc",
        CliSearchKind::BestFirst => "best-first",
    };

    let run_result = results::RunResult {
        timestamp,
        search: search_kind.to_string(),
        input_file: args.input.clone(),
        rules_file: args.rules.clone(),
        elapsed_secs,
        initial_cost: cost_before_rewrites,
        cost_after_rewrites: original_size,
        final_cost,
        compression_ratio,
        pattern,
        arity,
        pattern_size,
        num_matches,
        usage_matches,
        approx_cost,
        num_expansions: best_found_at.map(|n| n + 1),
        best_iteration: best_found_at,
        num_steps_run,
        rewritten_programs,
        debug_log_file,
    };

    if let Some(ref output_path) = args.output {
        let json = serde_json::to_string_pretty(&run_result).expect("Failed to serialize result");
        std::fs::write(output_path, json).expect("Failed to write output file");
    }
}

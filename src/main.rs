mod cost;
mod debug_log;
mod io;
mod lang;
mod matching;
mod math;
mod pattern;
mod results;
mod revexpr;
mod search;
mod smc;

use clap::Parser;

/// E-graph based program synthesis via SMC.
#[derive(Parser, Debug)]
#[command(version)]
pub struct Args {
    /// Path to the input JSON file containing programs.
    #[arg(short, long, default_value = "data/domains/cogsci/dials.json")]
    pub input: String,

    /// Path to rewrite rules file.
    #[arg(short, long)]
    pub rules: Option<String>,

    /// Number of particles.
    #[arg(long, default_value_t = 10_000)]
    pub num_particles: usize,

    /// Number of SMC steps.
    #[arg(long, default_value_t = 1000)]
    pub num_steps: usize,

    /// Softmax temperature for resampling weights.
    #[arg(long, default_value_t = 100.0)]
    pub temperature: f64,

    /// Stop after this many steps with no improvement.
    #[arg(long, default_value_t = 50)]
    pub dead_runs: usize,

    /// Probability of attempting variable reuse during expansion.
    #[arg(long, default_value_t = 0.5)]
    pub p_reuse: f64,

    /// Maximum arity of patterns to consider as "best".
    #[arg(long, default_value_t = 1000)]
    pub max_arity: usize,

    /// Enable slow rewrite check (assert fast == slow computation).
    #[arg(long, default_value_t = false)]
    pub check_slow: bool,

    /// Weight match selection by usage count during expansion.
    #[arg(long, default_value_t = false)]
    pub weight_by_usage: bool,

    /// Path to write a JSON-serialized RunResult.
    #[arg(short, long)]
    pub output: Option<String>,

    /// Enable detailed debug logging of all particles at each SMC step.
    #[arg(long, default_value_t = false)]
    pub debug_log: bool,
}

fn main() {
    let args = Args::parse();
    let start = std::time::Instant::now();

    let (egraph, root, cost_before_rewrites) = io::load_egraph(&args.input, args.rules.as_deref());
    let smc_result = smc::smc(egraph, root, &args);

    let elapsed_secs = start.elapsed().as_secs_f64();

    let (final_cost, compression_ratio, pattern, arity, pattern_size, num_matches, usage_matches, approx_cost, rewritten_programs) = match &smc_result.best {
        Some((cost, state)) => {
            let pat_size = cost::compute_pattern_size(&state.pattern);
            let usage_counts = search::compute_usage_counts(&smc_result.egraph, root);
            let um: usize = state.matches.iter().map(|m| usage_counts.get(&m.root_eclass).copied().unwrap_or(1)).sum();
            let appx = smc_result.original_size as i64 - pat_size as i64 * (um as i64 - 1);
            (
                Some(*cost),
                Some(smc_result.original_size as f64 / *cost as f64),
                Some(state.pattern.to_string()),
                Some(state.pattern.vars.len()),
                Some(pat_size),
                Some(state.matches.len()),
                Some(um),
                Some(appx),
                Some(cost::extract_rewritten_programs(&smc_result.egraph, root, state)),
            )
        }
        None => (None, None, None, None, None, None, None, None, None),
    };

    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);

    let run_result = results::RunResult {
        timestamp,
        input_file: args.input.clone(),
        rules_file: args.rules.clone(),
        elapsed_secs,
        initial_cost: cost_before_rewrites,
        cost_after_rewrites: smc_result.original_size,
        final_cost,
        compression_ratio,
        pattern,
        arity,
        pattern_size,
        num_matches,
        usage_matches,
        approx_cost,
        best_iteration: smc_result.best_found_at,
        num_steps_run: smc_result.num_steps_run,
        rewritten_programs,
    };

    if let (Some(debug_log), Some(output_path)) = (smc_result.debug_log, &args.output) {
        let debug_path = output_path.replace(".json", "_debug.json");
        let json = serde_json::to_string(&debug_log).expect("Failed to serialize debug log");
        std::fs::write(&debug_path, json).expect("Failed to write debug log");
        println!("wrote debug log to {}", debug_path);
    }

    if let Some(ref output_path) = args.output {
        let json = serde_json::to_string_pretty(&run_result).expect("Failed to serialize result");
        std::fs::write(output_path, json).expect("Failed to write output file");
    }
}

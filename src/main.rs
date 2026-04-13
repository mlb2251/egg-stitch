use clap::{Parser, ValueEnum};
use colored::Colorize;

use egg_stitch::best_first::{InteractiveSearch, SearchPriority};
use egg_stitch::cost;
use egg_stitch::io;
use egg_stitch::replay;
use egg_stitch::results;
use egg_stitch::search;
use egg_stitch::smc::SmcConfig;

/// Which search algorithm to run (CLI wrapper).
#[derive(ValueEnum, Clone, Debug)]
enum CliSearchKind {
    Smc,
    BestFirst,
}

/// Heap priority (CLI wrapper with ValueEnum derive).
#[derive(ValueEnum, Clone, Copy, Debug)]
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

    /// Print per-step progress output (top particles, follow stats, etc.).
    #[arg(long, default_value_t = false)]
    verbose: bool,

    /// Path to a replay JSON file to replay instead of running a fresh search.
    #[arg(long)]
    replay: Option<String>,
}

fn main() {
    let args = Args::parse();
    let start = std::time::Instant::now();

    let rules = args.rules.as_deref();
    let (egraph, root, cost_before_rewrites) = io::load_egraph(&args.input, rules);
    let (shared, original_size) = search::setup_search(egraph, root, args.follow.as_deref(), args.weight_by_usage, args.p_reuse, args.check_slow);

    // ── Replay mode ─────────────────────────────────────────────────────
    if let Some(ref replay_path) = args.replay {
        run_replay(shared, root, original_size, replay_path);
        return;
    }

    // ── Search ──────────────────────────────────────────────────────────
    let (priority, budget) = match args.search {
        CliSearchKind::Smc => (SearchPriority::Cost, 0),
        CliSearchKind::BestFirst => (args.priority.into(), args.num_steps),
    };
    let mut search = InteractiveSearch::new(shared, root, original_size, priority, args.max_arity);
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    match args.search {
        CliSearchKind::Smc => {
            let config = SmcConfig {
                num_particles: args.num_particles,
                num_steps: args.num_steps,
                temperature: args.temperature,
                dead_runs: args.dead_runs,
                verbose: args.verbose,
            };
            egg_stitch::smc::smc(&mut search, &config);
        }
        CliSearchKind::BestFirst => {
            let search_start = std::time::Instant::now();
            loop {
                if search.num_expansions() >= budget {
                    println!("{}", format!("reached expansion budget {}", budget).yellow());
                    break;
                }
                let old_best = search.best_cost();
                if search.step().is_none() {
                    break;
                }
                if search.best_cost() != old_best {
                    let (cost, state) = search.best_state().unwrap();
                    println!("{} {} {}", format!("[expansion {}]", search.num_expansions() - 1).yellow().bold(), format!("new best: {}", cost).green().bold(), state.pattern.to_string().cyan());
                }
            }
            println!("{} {}", "search time:".dimmed(), format!("{:.1?}", search_start.elapsed()).yellow());
            println!("{} {}", "expansions:".dimmed(), search.num_expansions().to_string().yellow());
        }
    }

    // ── Print results ───────────────────────────────────────────────────
    println!("\n{}", "═══ RESULT ═══".green().bold());
    if let (Some(iter), Some((cost, state))) = (search.best_found_at(), search.best_state()) {
        println!("{} {}", "best found at expansion:".dimmed(), iter.to_string().yellow());
        println!("{} {}", "pattern:".dimmed(), state.pattern.to_string().cyan().bold());
        println!("{} {}", "cost:".dimmed(), cost.to_string().green().bold());
        println!("{} {}", "compression ratio:".dimmed(), format!("{:.2}x", original_size as f64 / cost as f64).green().bold());
    }

    // ── Save output ─────────────────────────────────────────────────────
    if let Some(ref output_path) = args.output {
        let run_result = build_run_result(&args, &search, original_size, cost_before_rewrites, start.elapsed().as_secs_f64(), budget, output_path);
        let json = serde_json::to_string_pretty(&run_result).expect("Failed to serialize result");
        std::fs::write(output_path, json).expect("Failed to write output file");
    }
}

/// Replay a saved search log and print summary.
fn run_replay(shared: search::SharedSearchData, root: egg::Id, original_size: usize, path: &str) {
    let json = std::fs::read_to_string(path).expect("Failed to read replay file");
    let mut search = InteractiveSearch::new(shared, root, original_size, SearchPriority::Cost, 2);
    let t0 = std::time::Instant::now();
    let config = replay::replay_from_json(&mut search, &json).expect("Replay failed");
    let elapsed = t0.elapsed();
    println!("{} {}", "priority:".dimmed(), config.priority.bold());
    println!("{} {}", "max_arity:".dimmed(), config.max_arity.to_string().bold());
    println!("{} {}", "steps replayed:".dimmed(), search.num_expansions().to_string().yellow());
    println!("{} {}", "nodes created:".dimmed(), search.num_nodes().to_string().yellow());
    println!("{} {}", "replay time:".dimmed(), format!("{:.1?}", elapsed).yellow());
    if let Some(cost) = search.best_cost() {
        println!("{} {}", "best cost:".dimmed(), cost.to_string().green().bold());
        println!("{} {}", "compression ratio:".dimmed(), format!("{:.2}x", original_size as f64 / cost as f64).green().bold());
    }
}

/// Build a RunResult from the completed search, saving replay log as a side effect.
fn build_run_result(args: &Args, search: &InteractiveSearch, original_size: usize, cost_before_rewrites: usize, elapsed_secs: f64, budget: usize, output_path: &str) -> results::RunResult {
    let shared = search.shared();
    let root = search.root();

    // Extract best-state metrics.
    let (final_cost, compression_ratio, pattern, arity, pattern_size, num_matches, usage_matches, approx_cost, rewritten_programs) = match search.best_state() {
        Some((c, state)) => {
            let pat_size = cost::compute_pattern_size(&state.pattern);
            let usage_counts = search::compute_usage_counts(&shared.egraph, root);
            let um: usize = state.matches.iter().map(|m| usage_counts.get(&m.root_eclass).copied().unwrap_or(1)).sum();
            let appx = original_size as i64 - pat_size as i64 * (um as i64 - 1);
            (
                Some(c),
                Some(original_size as f64 / c as f64),
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

    // Save replay log.
    let replay_log_file = {
        let replay_path = output_path.replace(".json", "_replay.json");
        let replay_json = serde_json::to_string(&search.replay_log(budget)).expect("Failed to serialize replay log");
        std::fs::write(&replay_path, &replay_json).expect("Failed to write replay log");
        println!("wrote replay log to {}", replay_path);
        Some(std::path::Path::new(&replay_path).file_name().unwrap().to_string_lossy().into_owned())
    };

    let search_kind = match args.search {
        CliSearchKind::Smc => "smc",
        CliSearchKind::BestFirst => "best-first",
    };
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);

    results::RunResult {
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
        num_expansions: search.best_found_at().map(|n| n + 1),
        best_iteration: search.best_found_at(),
        num_steps_run: search.num_expansions(),
        rewritten_programs,
        replay_log_file,
    }
}

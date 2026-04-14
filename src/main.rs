use clap::Parser;
use egg_stitch::{Args, SearchKind, best_first, cost, io, lang, results, search, smc};

fn main() {
    let args = Args::parse();
    let start = std::time::Instant::now();

    let rules = args.rules.as_deref();
    let (egraph, root, cost_before_rewrites) = io::load_egraph(&args.input, rules);

    // Dispatch to the requested search algorithm, flattening each driver's result
    // into a common tuple so the downstream RunResult wiring stays shared.
    #[allow(clippy::type_complexity)]
    let (best, original_size, best_found_at, num_steps_run, result_egraph, debug_log_json): (Option<(usize, search::SearchState)>, usize, Option<usize>, usize, lang::StitchEgraph, Option<String>) = match args.search {
        SearchKind::Smc => {
            let r = smc::smc(egraph, root, &args);
            let json = r.debug_log.as_ref().map(|d| serde_json::to_string(d).expect("Failed to serialize debug log"));
            (r.best, r.original_size, r.best_found_at, r.num_steps_run, r.egraph, json)
        }
        SearchKind::BestFirst => {
            let r = best_first::best_first(egraph, root, &args);
            let json = r.tree_log.as_ref().map(|d| serde_json::to_string(d).expect("Failed to serialize tree log"));
            (r.best, r.original_size, r.best_found_at, r.num_expansions, r.egraph, json)
        }
    };

    let elapsed_secs = start.elapsed().as_secs_f64();

    let (final_cost, compression_ratio, pattern, arity, pattern_size, num_matches, usage_matches, approx_cost, rewritten_programs) = match &best {
        Some((cost, state)) => {
            let pat_size = cost::compute_pattern_size(&state.pattern);
            let usage_counts = search::compute_usage_counts(&result_egraph, root);
            let um: usize = state.matches.iter().map(|m| usage_counts.get(&m.root_eclass).copied().unwrap_or(1)).sum();
            let appx = original_size as i64 - pat_size as i64 * (um as i64 - 1);
            (
                Some(*cost),
                Some(original_size as f64 / *cost as f64),
                Some(state.pattern.to_string()),
                Some(state.pattern.vars.len()),
                Some(pat_size),
                Some(state.matches.len()),
                Some(um),
                Some(appx),
                Some(cost::extract_rewritten_programs(&result_egraph, root, state)),
            )
        }
        None => (None, None, None, None, None, None, None, None, None),
    };

    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    // Write debug log if the driver produced one and an output path was given.
    let debug_log_file = if let (Some(json), Some(output_path)) = (debug_log_json, &args.output) {
        let debug_path = output_path.replace(".json", "_debug.json");
        std::fs::write(&debug_path, json).expect("Failed to write debug log");
        println!("wrote debug log to {}", debug_path);
        Some(std::path::Path::new(&debug_path).file_name().unwrap().to_string_lossy().into_owned())
    } else {
        None
    };

    let search_kind = match args.search {
        SearchKind::Smc => "smc",
        SearchKind::BestFirst => "best-first",
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

use clap::Parser;
use egg_stitch::{Args, SearchKind, io, lang::OpChildrenLanguage, multiple_step_search, results};

fn main() {
    let args = Args::parse();
    let start = std::time::Instant::now();

    let rules = args.rules.as_deref();
    let (egraph, root, cost_before_rewrites) = io::load_egraph::<OpChildrenLanguage>(&args.input, rules);

    let (library, original_size, final_cost) = multiple_step_search(egraph, root, &args);

    let elapsed_secs = start.elapsed().as_secs_f64();
    let compression_ratio = final_cost.map(|fc| original_size as f64 / fc as f64);

    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let debug_log_file = None; // debug log wiring removed; add back if needed

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
        debug_log_file,
        library,
    };

    if let Some(ref output_path) = args.output {
        let json = serde_json::to_string_pretty(&run_result).expect("Failed to serialize result");
        std::fs::write(output_path, json).expect("Failed to write output file");
    }
}

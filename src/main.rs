use clap::Parser;
use egg_stitch::{
    Args, LanguageChoice, SearchKind, io,
    lang::{LambdaCalc, Op, OpChildren},
    multiple_step_search, results,
};

fn main() {
    let args = Args::parse();
    let start = std::time::Instant::now();

    // Pick the language at the boundary; the rest of the pipeline is generic.
    let (library, original_size, final_cost, cost_before_rewrites) = match args.language {
        LanguageChoice::OpChildren => run::<OpChildren>(&args),
        LanguageChoice::LambdaCalc => run::<LambdaCalc>(&args),
    };

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

/// Loads the egraph and runs the multi-abstraction search loop, parameterized by the language family.
fn run<F: egg_stitch::lang::LanguageFamily>(args: &Args) -> (Vec<results::AbstractionResult>, usize, Option<usize>, usize) {
    let (egraph, root, cost_before_rewrites) = io::load_egraph::<F::Apply<Op>>(&args.input, args.rules.as_deref(), args.weights);
    let (library, original_size, final_cost) = multiple_step_search::<F, Op>(egraph, root, args);
    (library, original_size, final_cost, cost_before_rewrites)
}

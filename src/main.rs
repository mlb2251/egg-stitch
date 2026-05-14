use clap::Parser;
use egg_stitch::{
    Args, LanguageChoice, SearchKind, io,
    lang::{LambdaCalc, Op, OpChildren, OpDB, StitchOp},
    multiple_step_search, results,
};

/// Bundles everything `main` needs from a language-specialised run, so the
/// language-generic post-processing stays out of `run`.
struct RunOutput {
    library: Vec<results::AbstractionResult>,
    original_size: usize,
    final_cost: Option<usize>,
    final_rewritten: Option<Vec<String>>,
    cost_before_rewrites: usize,
    original_programs: Vec<String>,
}

fn main() {
    let args = Args::parse();
    let start = std::time::Instant::now();

    // Pick the language family AND its leaf-Op at the boundary. LambdaCalc
    // gets `OpDB<Op>` so `$n` parses as a real De Bruijn variable (the fv
    // analysis and depth-aware extraction need that). OpChildren has no
    // binders, so DB vars are meaningless there — keeps plain `Op`.
    let RunOutput {
        library,
        original_size,
        final_cost,
        final_rewritten,
        cost_before_rewrites,
        original_programs,
    } = match args.language {
        LanguageChoice::OpChildren => run::<OpChildren, Op>(&args),
        LanguageChoice::LambdaCalc => run::<LambdaCalc, OpDB<Op>>(&args),
    };

    let elapsed_secs = start.elapsed().as_secs_f64();
    let compression_ratio = final_cost.map(|fc| original_size as f64 / fc as f64);

    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    let debug_log_file = None; // debug log wiring removed; add back if needed

    let search_kind = match args.search {
        SearchKind::Smc => "smc",
        SearchKind::BestFirst => "best-first",
    };

    let rewritten_programs = final_rewritten.unwrap_or_else(|| original_programs.clone());

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
        original_programs,
        rewritten_programs,
        library,
    };

    if let Some(ref output_path) = args.output {
        let json = serde_json::to_string_pretty(&run_result).expect("Failed to serialize result");
        std::fs::write(output_path, json).expect("Failed to write output file");
    }
}

/// Loads the egraph and runs the multi-abstraction search loop, parameterized
/// by both the language family `F` and the leaf-Op `O`.
fn run<F: egg_stitch::lang::LanguageFamily, O: StitchOp>(args: &Args) -> RunOutput {
    let load_start = std::time::Instant::now();
    let (egraph, root, cost_before_rewrites, original_programs) = io::load_egraph::<F, O>(&args.input, args.rules.as_deref(), args.weights);
    println!("load_egraph took {:.3}s", load_start.elapsed().as_secs_f64());
    let (library, original_size, final_cost, final_rewritten) = multiple_step_search::<F, O>(egraph, root, args);
    RunOutput {
        library,
        original_size,
        final_cost,
        final_rewritten,
        cost_before_rewrites,
        original_programs,
    }
}

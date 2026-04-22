pub mod best_first;
pub mod cost;
pub mod debug_log;
pub mod follow;
pub mod io;
pub mod lang;
pub mod logging;
pub mod matching;
pub mod math;
pub mod pattern;
pub mod results;
pub mod revexpr;
pub mod search;
pub mod smc;

use clap::{Parser, ValueEnum};
use egg::Id;

pub use best_first::SearchPriority;

use crate::lang::Op;

/// Which search algorithm to run.
#[derive(ValueEnum, Clone, Debug)]
pub enum SearchKind {
    /// Sequential Monte Carlo (stochastic particle filter).
    Smc,
    /// Best-first enumerative search over canonical patterns.
    BestFirst,
}

/// E-graph based program synthesis via SMC.
#[derive(Parser, Debug)]
#[command(version)]
pub struct Args {
    /// Search algorithm to use.
    #[arg(long, value_enum, default_value_t = SearchKind::Smc)]
    pub search: SearchKind,

    /// Path to the input JSON file containing programs.
    #[arg(short, long, default_value = "data/domains/cogsci/dials.json")]
    pub input: String,

    /// Path to rewrite rules file.
    #[arg(short, long)]
    pub rules: Option<String>,

    /// Follow pattern to constrain particle expansion.
    #[arg(short, long)]
    pub follow: Option<String>,

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

    /// Maximum arity of patterns to consider as "best".
    #[arg(long, default_value_t = 1000)]
    pub max_arity: usize,

    /// Heap priority for best-first search (only used when --search=best-first).
    #[arg(long, value_enum, default_value_t = SearchPriority::Cost)]
    pub priority: SearchPriority,

    /// Weight match selection by usage count during expansion.
    #[arg(long, default_value_t = false)]
    pub weight_by_usage: bool,

    /// Probability of attempting variable reuse during expansion.
    #[arg(long, default_value_t = 0.5)]
    pub p_reuse: f64,

    /// Enable slow rewrite check (assert fast == slow computation).
    #[arg(long, default_value_t = false)]
    pub check_slow: bool,

    /// Number of abstractions to find sequentially (each stacks on the previous).
    #[arg(long, default_value_t = 1)]
    pub num_abstractions: usize,

    /// After each abstraction, rewrite programs to use it and rebuild the egraph from scratch,
    /// rather than unioning fn_N enodes into the existing egraph.
    #[arg(long, default_value_t = false)]
    pub rebuild_egraph: bool,

    /// Path to write JSON output.
    #[arg(short, long)]
    pub output: Option<String>,

    /// Enable detailed debug logging of all particles at each SMC step.
    #[arg(long, default_value_t = false)]
    pub debug_log: bool,

    /// Print per-step progress output (top particles, follow stats, etc.).
    #[arg(long, default_value_t = false)]
    pub verbose: bool,
}

/// Runs the multi-abstraction search loop, returning the per-abstraction results,
/// the corpus size after DSRs (before any abstractions), and the final combined cost.
///
/// After each abstraction is found, `fn_N(args...)` enodes are added directly to the
/// egraph and unioned with their match roots, then the egraph is rebuilt. This avoids
/// serialising programs to strings and re-parsing. The eclass arguments already carry
/// all DSR equivalences, so no re-saturation is needed.
pub fn multiple_step_search(egraph: lang::StitchEgraph, root: Id, args: &Args) -> (Vec<results::AbstractionResult>, usize, Option<usize>) {
    let mut egraph = egraph;
    let mut root = root;
    let mut library = Vec::new();
    let mut original_size = 0;
    let mut final_cost = None;

    for abstraction_idx in 0..args.num_abstractions {
        let (best, iter_original_size, best_found_at, num_steps_run, result_egraph) = match args.search {
            SearchKind::Smc => {
                let r = smc::smc(egraph, root, args);
                (r.best, r.original_size, r.best_found_at, r.num_steps_run, r.egraph)
            }
            SearchKind::BestFirst => {
                let r = best_first::best_first(egraph, root, args);
                (r.best, r.original_size, r.best_found_at, r.num_expansions, r.egraph)
            }
        };

        if abstraction_idx == 0 {
            original_size = iter_original_size;
        }

        match best {
            None => break,
            Some((best_cost, state)) => {
                let pat_size = cost::compute_pattern_size(&state.pattern);
                let usage_counts = search::compute_usage_counts(&result_egraph, root);
                let usage_matches: usize = state.matches.iter().map(|m| usage_counts.get(&m.root_eclass).copied().unwrap_or(1)).sum();
                let approx_cost = iter_original_size as i64 - pat_size as i64 * (usage_matches as i64 - 1);
                let fn_name = format!("fn_{abstraction_idx}");
                let (next_egraph, next_root, rewritten_programs) = apply_abstraction(result_egraph, root, &state, &fn_name, args.rebuild_egraph, args.rules.as_deref());

                final_cost = Some(best_cost);
                library.push(results::AbstractionResult {
                    pattern: format!("{fn_name}: {}", state.pattern),
                    arity: state.pattern.vars.len(),
                    pattern_size: pat_size,
                    num_matches: state.matches.len(),
                    usage_matches,
                    approx_cost,
                    num_steps_run,
                    num_expansions: best_found_at.map(|n| n + 1),
                    best_iteration: best_found_at,
                    rewritten_programs,
                });

                if abstraction_idx + 1 < args.num_abstractions {
                    egraph = next_egraph;
                    root = next_root;
                } else {
                    break;
                }
            }
        }
    }

    (library, original_size, final_cost)
}

/// Applies an abstraction to the egraph by adding `fn_name(args...)` enodes for every
/// match substitution and unioning each with its match root, then rebuilds.
///
/// If `rebuild` is true, the rewritten program strings are extracted and used to build a
/// fresh egraph (with DSR rules re-applied), discarding all prior equivalences.
/// If `rebuild` is false, the existing egraph with unions is returned as-is.
///
/// Returns the (possibly new) egraph, the root id within it, and the rewritten program strings.
fn apply_abstraction(egraph: lang::StitchEgraph, root: Id, state: &search::SearchState, fn_name: &str, rebuild: bool, rule_file: Option<&str>) -> (lang::StitchEgraph, Id, Vec<String>) {
    let fn_sym: egg::Symbol = fn_name.into();
    let mut egraph = egraph;
    for m in &state.matches {
        for subst in &m.substs {
            let node = lang::StitchLang { op: Op::Sym(fn_sym), children: subst.vars.clone() };
            let x = egraph.add(node);
            egraph.union(x, m.root_eclass);
        }
    }
    egraph.rebuild();
    let extractor = egg::Extractor::new(&egraph, egg::AstSize);
    let programs_node = egraph[root].nodes.iter().find(|n| n.op.as_str() == "programs").expect("root e-class should contain a `programs` enode");
    let programs: Vec<String> = programs_node.children.iter().map(|&child| extractor.find_best(child).1.to_string()).collect();

    if rebuild {
        let (fresh_egraph, fresh_root) = io::egraph_from_programs(&programs, rule_file);
        (fresh_egraph, fresh_root, programs)
    } else {
        (egraph, root, programs)
    }
}

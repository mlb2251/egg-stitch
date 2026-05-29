pub mod best_first;
pub mod candidates;
pub mod cost;
pub mod debug_log;
pub mod follow;
pub mod io;
pub mod lang;
pub mod logging;
pub mod lower_bound;
pub mod matching;
pub mod math;
pub mod pattern;
pub mod results;
pub mod revexpr;
pub mod search;
pub mod shared;
pub mod shift;
pub mod shift_equal;
pub mod smc;

use clap::{Parser, ValueEnum};
use colored::Colorize;
use egg::Language;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

pub use best_first::SearchPriority;

use crate::lang::{LanguageFamily, StitchEgraph, StitchLanguage, StitchOp, Weights};

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

    /// Seed for the search RNG. When omitted, a fresh u64 is generated and
    /// printed at startup so a crashing run can be replayed exactly.
    #[arg(long)]
    pub seed: Option<u64>,

    /// Number of search steps (SMC steps, or best-first heap pops).
    /// Required for SMC. For best-first, at least one of --num-steps or
    /// --time-limit must be provided.
    #[arg(long)]
    pub num_steps: Option<usize>,

    /// Wall-clock time limit in seconds for best-first search.
    /// At least one of --num-steps or --time-limit must be provided for best-first.
    #[arg(long)]
    pub time_limit: Option<f64>,

    /// Softmax temperature for resampling weights.
    #[arg(long, default_value_t = 100.0)]
    pub temperature: f64,

    /// Stop after this many steps with no improvement.
    #[arg(long, default_value_t = 50)]
    pub dead_runs: usize,

    /// Maximum arity of patterns to consider as "best".
    #[arg(long, default_value_t = 1000)]
    pub max_arity: usize,

    /// Disallow zero-arity abstractions (patterns with no metavariables).
    #[arg(long, default_value_t = false)]
    pub no_zero_arity: bool,

    /// Heap priority for best-first search (only used when --search=best-first).
    #[arg(long, value_enum, default_value_t = SearchPriority::Cost)]
    pub priority: SearchPriority,

    /// Multiplicative boost applied to reuse-action sampling weights in SMC.
    /// Each successor is weighted by its `(match, subst)` support count;
    /// reuse-action weights are additionally multiplied by `boost_reuse_weight`,
    /// while expand-action weights are left unscaled. Default 1.0 (no boost).
    #[arg(long, default_value_t = 1.0)]
    pub boost_reuse_weight: f64,

    /// Enable slow rewrite check (assert fast == slow computation).
    #[arg(long, default_value_t = false)]
    pub check_slow: bool,

    /// Number of abstractions to find sequentially (each stacks on the previous).
    #[arg(long, default_value_t = 1)]
    pub num_abstractions: usize,

    /// Enable the `seen` set in best-first search (canonical-pattern dedup).
    /// Off by default: the canonical-ordering rule (`frozen_count`) and the
    /// dominance/useless-inline short-circuits already eliminate most
    /// duplicates, so the seen-set typically catches only a few percent of
    /// states while costing a clone+hash on every successor. Empirically
    /// disabling it is ~2× faster on cogsci-scale runs and ~15% faster on
    /// small lambda-calc runs, with the same final cost (a few percent more
    /// expansions but cheaper per-expansion overall).
    #[arg(long = "opt-seen", default_value_t = false)]
    pub opt_seen: bool,

    /// Disable dominance pruning for the reuse branch (on by default).
    /// Reuse dominance: when reuse(i,j) preserves num_substs, return that
    /// reuse as a singleton successor (no cost check — sound by construction).
    #[arg(long = "no-opt-dominance-reuse", action = clap::ArgAction::SetFalse)]
    pub opt_dominance_reuse: bool,

    /// Disable the useless-frozen-variable check (on by default).
    /// When a frozen metavar `?#k` is bound to the same e-class in every
    /// match and that e-class has no above-pattern free vars, the state
    /// is pruned — the abstraction adds no compression at that slot.
    /// Stitch analog: "argument capture" / `is_useless_abstract`.
    #[arg(long = "no-opt-useless-frozen", action = clap::ArgAction::SetFalse)]
    pub opt_useless_frozen: bool,

    /// Disable the useless-non-frozen inlining short-circuit (on by default).
    /// When a non-frozen metavar `?#k` is bound to the same e-class in every
    /// match (and that e-class has no above-pattern free vars), this emits a
    /// single dominant successor that one-step expands `?#k` (and any other
    /// such non-frozen vars) to the size-minimal enode shape of its e-class —
    /// inlining a constant arg specialises the body without giving up matches.
    /// `frozen_count` is preserved since the move runs "before" any normal
    /// expand in the canonical order.
    #[arg(long = "no-opt-useless-inline", action = clap::ArgAction::SetFalse)]
    pub opt_useless_inline: bool,

    /// Disable lower-bound pruning of best-first children (on by default).
    /// Each child gets a `compute_lower_bound` estimate; if it already
    /// exceeds the current best, skip the full cost call. Bounds are also
    /// re-checked on heap pop in case the best improved meanwhile.
    #[arg(long = "no-opt-lower-bound", action = clap::ArgAction::SetFalse)]
    pub opt_lower_bound: bool,

    /// Path to write JSON output.
    #[arg(short, long)]
    pub output: Option<String>,

    /// Enable detailed debug logging of all particles at each SMC step.
    #[arg(long, default_value_t = false)]
    pub debug_log: bool,

    /// Print per-step progress output (top particles, follow stats, etc.).
    #[arg(long, default_value_t = false)]
    pub verbose: bool,

    /// Selects the language family the pipeline runs over. Patterns/programs/rules
    /// are always written in user-facing flat form; the language layer handles any
    /// conversion (e.g. currying for `lambda-calc`) at the boundary.
    #[arg(long, value_enum, default_value_t = LanguageChoice::OpChildren)]
    pub language: LanguageChoice,

    /// Cost weights applied per enode kind.
    #[command(flatten)]
    pub weights: Weights,
}

/// Which language family `multiple_step_search` runs over.
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum LanguageChoice {
    /// Flat n-ary nodes (`(f a b c)` is a single enode). Default.
    #[value(name = "op-children")]
    OpChildren,
    /// Lambda-calculus shape: curried binary `App`, unary `Lam`, multi-child
    /// `Programs` root.
    #[value(name = "lambda-calc")]
    LambdaCalc,
}

/// Runs the multi-abstraction search loop, returning the per-abstraction results,
/// the corpus size after DSRs (before any abstractions), the final combined cost,
/// and the final rewritten corpus (`Some` once any abstraction has been applied,
/// `None` if no abstraction was found).
///
/// After each abstraction is found, `fn_N(args...)` enodes are added and unioned with
/// their match roots, then the rewritten programs are extracted as strings and used to
/// build a fresh egraph for the next round (DSR rules are re-applied there).
pub fn multiple_step_search<F: LanguageFamily, O: StitchOp>(data: shared::SharedData<F, O>, args: &Args) -> (Vec<results::AbstractionResult>, usize, Option<usize>, Option<Vec<String>>) {
    let mut data = data;
    let mut library = Vec::new();
    let mut original_size = 0;
    let mut final_cost = None;
    let mut final_rewritten: Option<Vec<String>> = None;

    let seed = args.seed.unwrap_or_else(|| rand::rng().random());
    println!("{} {}", "rng seed:".dimmed(), seed.to_string().bold());
    let mut rng = StdRng::seed_from_u64(seed);

    // Pick the first `fn_N` name that doesn't collide with any leaf already
    // present in the input — otherwise rerunning the search on an already
    // abstracted corpus (or any input that happens to use `fn_N` as a symbol)
    // produces output that can't be inlined unambiguously.
    let fn_name_base = first_free_fn_index::<F::Apply<O>>(&data.egraph);

    for abstraction_idx in 0..args.num_abstractions {
        let (best, iter_original_size, best_found_at, num_steps_run, result_data, best_history) = match args.search {
            SearchKind::Smc => {
                let r = smc::smc::<F, O>(data, args, &mut rng);
                (r.best, r.original_size, r.best_found_at, r.num_steps_run, r.data, None)
            }
            SearchKind::BestFirst => {
                let r = best_first::best_first(data, args);
                (r.best, r.original_size, r.best_found_at, r.num_expansions, r.data, Some(r.best_history))
            }
        };

        if abstraction_idx == 0 {
            original_size = iter_original_size;
        }

        match best {
            None => break,
            Some((best_cost, best)) => {
                // The search already chose this candidate when it scored `state`,
                // and threaded it back via `best` — no need to redo the
                // optimisation here.
                let state = &best.state;
                let candidate = &best.selection.candidate;
                let variable_indices = &candidate.variable_indices;
                let ho_arity: Vec<u32> = variable_indices.iter().map(|v| v.len() as u32).collect();
                let pat_size = cost::compute_body_size_with_ho::<F, O>(&state.pattern, &ho_arity, &result_data.egraph.analysis.weights);
                let body_str = state.pattern.display_with_ho(variable_indices);
                let lambda = state.pattern.display_as_lambda(variable_indices);
                let usage_counts = search::compute_usage_counts(&result_data.egraph, result_data.root);
                // With the `None` sentinel every match keeps every subst, so every
                // match contributes its usage count; otherwise count only matches
                // whose kept list is non-empty.
                let usage_matches: usize = match &candidate.kept_substs {
                    None => state.matches.iter().map(|m| usage_counts.get(&m.root_eclass).copied().unwrap_or(1)).sum(),
                    Some(k) => state.matches.iter().zip(k).filter(|(_, ks)| !ks.is_empty()).map(|(m, _)| usage_counts.get(&m.root_eclass).copied().unwrap_or(1)).sum(),
                };
                let approx_cost = iter_original_size as i64 - pat_size as i64 * (usage_matches as i64 - 1);
                let fn_name = format!("fn_{}", fn_name_base + abstraction_idx);
                let (next_data, rewritten_programs) = apply_abstraction::<F, O>(result_data, state, candidate, &fn_name, args.rules.as_deref());

                // `best_cost` is the search's score for this iteration: rewritten
                // corpus + this abstraction's body. Earlier iterations rewrote the
                // corpus into `fn_K(...)` call sites only — their bodies live in
                // the library and must be added separately.
                let prev_bodies: usize = library.iter().map(|a: &results::AbstractionResult| a.pattern_size).sum();
                final_cost = Some(best_cost + prev_bodies);
                final_rewritten = Some(rewritten_programs);
                library.push(results::AbstractionResult {
                    pattern: format!("{fn_name}: {body_str}"),
                    lambda,
                    arity: state.pattern.vars.len(),
                    pattern_size: pat_size,
                    num_matches: state.matches.len(),
                    usage_matches,
                    approx_cost,
                    num_steps_run,
                    num_expansions: best_found_at.map(|n| n + 1),
                    best_iteration: best_found_at,
                    best_history,
                });

                if abstraction_idx + 1 < args.num_abstractions {
                    data = next_data;
                } else {
                    break;
                }
            }
        }
    }

    (library, original_size, final_cost, final_rewritten)
}

/// Smallest `k` such that no discriminant in `egraph` renders as `fn_k`,
/// `fn_{k+1}`, … Used to avoid clashes between the search's chosen abstraction
/// names and any `fn_*` symbol already present in the input (e.g. when
/// re-running the search over a corpus that already contains stitch output).
///
/// Scans the discriminant of every enode, not just leaves: in `OpChildren`
/// languages a multi-arity head like `(fn_0 1 2)` lives in a single enode
/// `(op=fn_0, children=[1,2])` whose op is what we'd collide with.
fn first_free_fn_index<L: StitchLanguage>(egraph: &StitchEgraph<L>) -> usize {
    let mut max_existing: Option<usize> = None;
    for class in egraph.classes() {
        for enode in &class.nodes {
            let name = enode.discriminant().to_string();
            if let Some(rest) = name.strip_prefix("fn_")
                && let Ok(idx) = rest.parse::<usize>()
            {
                max_existing = Some(max_existing.map_or(idx, |m| m.max(idx)));
            }
        }
    }
    max_existing.map_or(0, |m| m + 1)
}

/// Applies an abstraction to the egraph: adds `fn_name(args...)` enodes for every
/// match substitution and unions each with its match root, rebuilds, extracts the
/// rewritten programs as strings, and feeds them into a fresh egraph (with DSR
/// rules re-applied).
///
/// Returns the fresh egraph, its root id, and the rewritten program strings.
pub fn apply_abstraction<F: LanguageFamily, O: StitchOp>(data: shared::SharedData<F, O>, state: &search::SearchState<F, O>, candidate: &cost::CostCandidate, fn_name: &str, rule_file: Option<&str>) -> (shared::SharedData<F, O>, Vec<String>) {
    let shared::SharedData { egraph, root } = data;
    let egraph = cost::build_rewritten_egraph::<F, O>(egraph, state, candidate, fn_name);
    let extractor = egg::Extractor::new(&egraph, cost::WeightedSize { weights: egraph.analysis.weights });
    let programs_node = egraph[root].nodes.iter().find(|n| n.is_programs_node()).expect("root e-class should contain a `programs` enode");
    let programs: Vec<String> = programs_node.children().iter().map(|&child| <F::Apply<O> as StitchLanguage>::display_recexpr(&extractor.find_best(child).1)).collect();
    let weights = egraph.analysis.weights;
    let fresh = io::egraph_from_programs::<F, O>(&programs, rule_file, weights);
    (fresh, programs)
}

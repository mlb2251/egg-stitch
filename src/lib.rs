pub mod best_first;
pub mod candidates;
pub mod constant_folding;
pub mod cost;
pub mod debug_log;
pub mod egraph_util;
pub mod factor;
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

use std::time::Instant;

use clap::{Parser, ValueEnum};
use colored::Colorize;
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

/// `--max-forced-expansion` value: a slack bound `Some(k)`, or `none` to disable
/// the forced-expansion prune entirely.
#[derive(Clone, Copy, Debug)]
pub struct MaxForcedExpansion(pub Option<usize>);

impl std::str::FromStr for MaxForcedExpansion {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("none") {
            Ok(Self(None))
        } else {
            s.parse::<usize>().map(|n| Self(Some(n))).map_err(|_| format!("expected a non-negative integer or `none`, got {s:?}"))
        }
    }
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

    /// Apply DSRs (rewrite rules) only when building the initial egraph, not
    /// during search. With this set, the initial egraph is normalized by the
    /// rules, its min-term is extracted, and a fresh rule-free egraph is rebuilt
    /// from that min-term for the actual search (and between abstractions).
    #[arg(long, default_value_t = false)]
    pub only_use_dsrs_at_start: bool,

    /// Maximum number of e-saturation iterations when applying rewrite rules to
    /// the egraph (initial build and between abstractions).
    #[arg(long, default_value_t = 100)]
    pub iter_limit: usize,

    /// Maximum number of e-nodes the egraph may grow to during e-saturation
    /// before the runner stops (initial build and between abstractions).
    #[arg(long, default_value_t = 50_000_000)]
    pub node_limit: usize,

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
    /// At least one of --num-steps, --time-limit, or --max-forced-expansion must
    /// be provided for best-first.
    #[arg(long)]
    pub time_limit: Option<f64>,

    /// Softmax temperature for resampling weights.
    #[arg(long, default_value_t = 100.0)]
    pub temperature: f64,

    /// Stop after this many steps with no improvement.
    #[arg(long, default_value_t = 50)]
    pub dead_runs: usize,

    /// Maximum arity of patterns that can be returned by search.
    #[arg(long, default_value_t = 1000)]
    pub max_arity: usize,

    /// Disallow zero-arity abstractions (patterns with no metavariables).
    #[arg(long, default_value_t = false)]
    pub no_zero_arity: bool,

    /// Heap priority for best-first search (only used when --search=best-first).
    /// Default `forced-then-cost`: pop in non-decreasing forced-expansion order
    /// (cost breaking ties), so the productive low-forced region is searched
    /// first and the over-specialized tail last.
    #[arg(long, value_enum, default_value_t = SearchPriority::ForcedThenCost)]
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

    /// Prune patterns which force "expansions" (e.g., 4 -> (+ 4 0)) at *every*
    /// match site.
    ///
    /// Specifically for a match location r of p, let
    ///     - Cost(r) = min_{t in r} Cost(t)
    ///     - Cost(r | p) = min_{t in r, t matches p} Cost(t)
    ///     - ForcedExpansion(r, p) = Cost(r | p) - Cost(r)
    ///     - ForcedExpansion(p) = min_{r | p matches at r} ForcedExpansion(r, p)
    /// Each t that matches p matches at some (r, σ), and at this point,
    ///     Cost(t) = CostWithoutVars(p) + sum_{s in σ} Cost(s)
    /// As such, we can compute
    ///     Cost(r | p) = CostWithoutVars(p) + min_{σ | r matches at p with σ} sum_{s in σ} Cost(s)
    ///
    /// We keep only patterns with ForcedExpansion(p) ≤ `max_forced_expansion`
    ///
    /// Default `none`: the prune is off. Set to a non-negative integer to enable it.
    #[arg(long = "max-forced-expansion", default_value = "none")]
    pub max_forced_expansion: MaxForcedExpansion,

    /// Path to write JSON output.
    #[arg(short, long)]
    pub output: Option<String>,

    /// Enable detailed debug logging of all particles at each SMC step.
    #[arg(long, default_value_t = false)]
    pub debug_log: bool,

    /// Print per-step progress output (top particles, follow stats, etc.).
    #[arg(long, default_value_t = false)]
    pub verbose: bool,

    /// Print each best-first expansion's forced expansion (its argmin value and
    /// the min-term of the closest-to-minimal match root). Independent of
    /// `--verbose`; the min-term extraction makes it non-trivial, so it's its own
    /// flag.
    #[arg(long = "verbose-forced-expansion", default_value_t = false)]
    pub verbose_forced_expansion: bool,

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

/// Tuple returned by [`multiple_step_search`]: `(library, corpus size after DSRs,
/// final combined cost, final rewritten corpus, best-first heap size at stop for
/// each abstraction iteration)`.
type MultiStepSearchResult = (Vec<results::AbstractionResult>, usize, Option<Vec<usize>>, Option<Vec<String>>, Option<Vec<usize>>, Vec<Instant>);

/// Runs the multi-abstraction search loop, returning the per-abstraction results,
/// the corpus size after DSRs (before any abstractions), the final combined cost,
/// the final rewritten corpus (`Some` once any abstraction has been applied,
/// `None` if no abstraction was found), and the best-first heap size at stop for
/// each iteration (`None` for SMC).
///
/// After each abstraction is found, `fn_N(args...)` enodes are added and unioned with
/// their match roots, then the rewritten programs are extracted as strings and used to
/// build a fresh egraph for the next round (DSR rules are re-applied there).
pub fn multiple_step_search<F: LanguageFamily, O: StitchOp>(data: shared::SharedData<F, O>, args: &Args) -> MultiStepSearchResult {
    let mut data = data;
    let mut library = Vec::new();
    let mut original_size = 0;
    let mut cost_at_end_of_each_iter: Option<Vec<usize>> = None; // best cost at each iteration, for summary output
    let mut final_rewritten: Option<Vec<String>> = None;
    let mut heap_sizes_at_end: Option<Vec<usize>> = None; // best-first frontier size at stop, one per search iteration
    let mut search_ends: Vec<Instant> = Vec::new();
    // (fn name, cumulative corpus+library cost, usage matches in the minimal
    // corpus, body) after each abstraction, for the end-of-run summary of cost,
    // cumulative compression, match count, and the abstraction body.
    let mut summary: Vec<(String, usize, usize, String)> = Vec::new();

    let seed = args.seed.unwrap_or_else(|| rand::rng().random());
    println!("{} {}", "rng seed:".dimmed(), seed.to_string().bold());
    let mut rng = StdRng::seed_from_u64(seed);

    // Pick the first `fn_N` name that doesn't collide with any leaf already
    // present in the input — otherwise rerunning the search on an already
    // abstracted corpus (or any input that happens to use `fn_N` as a symbol)
    // produces output that can't be inlined unambiguously.
    let fn_name_base = first_free_fn_index::<F::Apply<O>>(&data.egraph);

    for abstraction_idx in 0..args.num_abstractions {
        let (best, iter_original_size, best_found_at, num_steps_run, result_data, best_history, iter_heap_size) = match args.search {
            SearchKind::Smc => {
                let r = smc::smc::<F, O>(data, args, &mut rng);
                (r.best, r.original_size, r.best_found_at, r.num_steps_run, r.data, None, None)
            }
            SearchKind::BestFirst => {
                let r = best_first::best_first(data, args);
                (r.best, r.original_size, r.best_found_at, r.num_expansions, r.data, Some(r.best_history), Some(r.heap_size_at_end))
            }
        };

        if abstraction_idx == 0 {
            original_size = iter_original_size;
        }
        if let Some(h) = iter_heap_size {
            heap_sizes_at_end.get_or_insert_with(Vec::new).push(h);
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
                let usage_counts = egraph_util::compute_usage_counts(&result_data.egraph, result_data.root);
                // A match contributes its usage count iff it keeps at least one
                // candidate-compatible subst (lambda-free domains keep them all).
                let var_depth = &state.pattern.var_depth;
                let usage_matches: usize = state
                    .matches
                    .iter()
                    .filter(|m| cost::filter_factors_by_candidate(&result_data.egraph, m, variable_indices, var_depth).is_some())
                    .map(|m| usage_counts.get(&m.root_eclass).copied().unwrap_or(1))
                    .sum();
                let approx_cost = iter_original_size as i64 - pat_size as i64 * (usage_matches as i64 - 1);
                let fn_name = format!("fn_{}", fn_name_base + abstraction_idx);
                // With `--only-use-dsrs-at-start` the rules are not re-applied to
                // the fresh egraph between abstractions.
                let rule_file = if args.only_use_dsrs_at_start { None } else { args.rules.as_deref() };
                let (next_data, rewritten_programs) = apply_abstraction::<F, O>(result_data, state, candidate, &fn_name, rule_file, args.iter_limit, args.node_limit);

                // `best_cost` is the search's score for this iteration: rewritten
                // corpus + this abstraction's body. Earlier iterations rewrote the
                // corpus into `fn_K(...)` call sites only — their bodies live in
                // the library and must be added separately.
                let prev_bodies: usize = library.iter().map(|a: &results::AbstractionResult| a.pattern_size).sum();
                cost_at_end_of_each_iter.get_or_insert_with(Vec::new).push(best_cost + prev_bodies);
                summary.push((fn_name.clone(), best_cost + prev_bodies, usage_matches, body_str.clone()));
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
                search_ends.push(Instant::now());

                if abstraction_idx + 1 < args.num_abstractions {
                    data = next_data;
                } else {
                    break;
                }
            }
        }
    }

    if !summary.is_empty() {
        // Right-align every number to its column's widest entry so the costs,
        // ratios, and match counts line up across abstractions. Pad the plain
        // strings *before* coloring — ANSI escapes would otherwise throw off the
        // width counts that `format!` uses.
        let name_w = summary.iter().map(|(n, ..)| n.len() + 1).max().unwrap_or(0);
        let cost_w = summary.iter().map(|(_, c, ..)| c.to_string().len()).max().unwrap_or(0).max(original_size.to_string().len());
        let ratio_w = summary.iter().map(|(_, c, ..)| format!("{:.2}x", original_size as f64 / *c as f64).len()).max().unwrap_or(0);
        let match_w = summary.iter().map(|(.., m, _)| m.to_string().len()).max().unwrap_or(0);
        println!("\n{}", "═══ LIBRARY SUMMARY ═══".green().bold());
        println!("{} {}", "initial cost:".dimmed(), format!("{:>cost_w$}", original_size).bold());
        for (name, cost, matches, body) in &summary {
            let ratio = original_size as f64 / *cost as f64;
            let name_s = format!("{:<name_w$}", format!("{name}:"));
            let cost_s = format!("{cost:>cost_w$}");
            let ratio_s = format!("{:>ratio_w$}", format!("{ratio:.2}x"));
            let match_s = format!("{matches:>match_w$}");
            println!("  {} cost {}  cumulative {}  matches {}", name_s.cyan().bold(), cost_s.dimmed(), ratio_s.green().bold(), match_s.bold(),);
            println!("    {}", body.dimmed());
        }
    }

    (library, original_size, cost_at_end_of_each_iter, final_rewritten, heap_sizes_at_end, search_ends)
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
pub fn apply_abstraction<F: LanguageFamily, O: StitchOp>(data: shared::SharedData<F, O>, state: &search::SearchState<F, O>, candidate: &cost::CostCandidate, fn_name: &str, rule_file: Option<&str>, iter_limit: usize, node_limit: usize) -> (shared::SharedData<F, O>, Vec<String>) {
    let shared::SharedData { egraph, root } = data;
    let egraph = cost::build_rewritten_egraph::<F, O>(egraph, state, candidate, fn_name);
    let programs = io::extract_programs::<F::Apply<O>>(&egraph, root);
    let weights = egraph.analysis.weights;
    let fresh = io::egraph_from_programs::<F, O>(&programs, rule_file, weights, iter_limit, node_limit);
    (fresh, programs)
}

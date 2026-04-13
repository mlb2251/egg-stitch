use colored::Colorize;

use crate::cost::compute_cost;
use crate::debug_log::{DebugLog, StepLog, build_particle_logs, log_debug_step};
use crate::lang::{StitchEgraph, StitchLang};
use crate::logging::{apply_follow_constraint, print_top_particles};
use crate::math::logaddexp;
use crate::revexpr::RevExpr;
use crate::search::{SearchState, setup_search};
use egg::ENodeOrVar;
use rand::Rng;
use rustc_hash::FxHashMap;

/// Inserts a freshly-expanded state into the parallel (states, mults) deduped-by-pattern
/// buffer, either bumping the multiplicity of an existing group or pushing a new one.
fn dedup_insert(s: SearchState, states: &mut Vec<SearchState>, mults: &mut Vec<usize>, dedup: &mut FxHashMap<RevExpr<ENodeOrVar<StitchLang>>, usize>) {
    match dedup.get(&s.pattern.pattern) {
        Some(&idx) => mults[idx] += 1,
        None => {
            let idx = states.len();
            dedup.insert(s.pattern.pattern.clone(), idx);
            states.push(s);
            mults.push(1);
        }
    }
}

/// Output of a completed SMC run.
pub struct SmcResult {
    pub best: Option<(usize, SearchState)>,
    pub original_size: usize,
    pub best_found_at: Option<usize>,
    pub num_steps_run: usize,
    pub egraph: StitchEgraph,
    pub debug_log: Option<DebugLog>,
}

/// Runs SMC to find a pattern that minimizes compressed corpus size.
///
/// Particles are stored as `(SearchState, multiplicity)` pairs. After each
/// expansion step, identical patterns are deduplicated and their counts merged,
/// so cost computation runs once per unique pattern instead of once per particle.
#[allow(clippy::needless_range_loop)]
pub fn smc(egraph: StitchEgraph, root: egg::Id, args: &crate::Args) -> SmcResult {
    let (shared, cost_cache, original_size) = setup_search(egraph, root, args);
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    let num_particles = args.num_particles;
    let num_steps = args.num_steps;
    let temperature = args.temperature;
    let dead_runs = args.dead_runs;
    let max_arity = args.max_arity;
    let verbose = args.verbose;

    let mut best_so_far: Option<(usize, SearchState)> = None;
    let mut best_found_at = None;
    let mut steps_run = 0;
    let debug = args.debug_log;
    let mut debug_steps: Vec<StepLog> = Vec::new();

    let mut particles: Vec<(SearchState, usize)> = vec![(SearchState::new(&shared), num_particles)];

    for step in 0..num_steps {
        // Expand each (state, mult) group into `mult` independent random expansions,
        // deduplicating identical resulting patterns.
        let mut expanded: Vec<SearchState> = Vec::new();
        let mut mults: Vec<usize> = Vec::new();
        let mut dedup: FxHashMap<RevExpr<ENodeOrVar<StitchLang>>, usize> = FxHashMap::default();
        for (state, mult) in particles.drain(..) {
            for _ in 1..mult {
                let mut s = state.clone();
                s.expand_random(&shared, false);
                dedup_insert(s, &mut expanded, &mut mults, &mut dedup);
            }
            let mut s = state;
            s.expand_random(&shared, false);
            dedup_insert(s, &mut expanded, &mut mults, &mut dedup);
        }
        drop(dedup);

        let costs: Vec<usize> = expanded.iter().map(|s| compute_cost(&shared.egraph, root, &cost_cache, s, shared.check_slow)).collect();

        for (i, cost) in costs.iter().enumerate() {
            if expanded[i].pattern.vars.len() <= max_arity && best_so_far.as_ref().is_none_or(|best| *cost < best.0) {
                println!("{} {} {}", format!("[iteration {}]", step).yellow().bold(), format!("new best: {}", cost).green().bold(), expanded[i].pattern.to_string().cyan());
                best_so_far = Some((*cost, expanded[i].clone()));
                best_found_at = Some(step);
            }
        }

        // log-space weights: logw_i = -cost_i / temperature
        let mut log_weights: Vec<f64> = costs.iter().map(|c| -(*c as f64) / temperature).collect();

        for (i, s) in expanded.iter().enumerate() {
            if s.pattern.vars.is_empty() {
                log_weights[i] = f64::NEG_INFINITY;
            }
        }

        if let Some(ref follow) = shared.follow {
            apply_follow_constraint(&expanded, &mut log_weights, follow, &shared, original_size, &costs, verbose);
        }

        let total_weight = log_weights.iter().copied().fold(f64::NEG_INFINITY, logaddexp);
        let mut weights: Vec<f64> = if total_weight.is_finite() { log_weights.iter().map(|lw| (lw - total_weight).exp()).collect() } else { vec![0.0; log_weights.len()] };

        if weights.iter().sum::<f64>() == 0.0 {
            log_debug_step(debug, &mut debug_steps, step, &expanded, &costs, &weights, &best_so_far, &[]);
            steps_run = step + 1;
            println!("{}", "all particles died, stopping".red().bold());
            break;
        }
        if best_found_at.is_some_and(|bf| (step as i64) - (bf as i64) > dead_runs as i64) {
            log_debug_step(debug, &mut debug_steps, step, &expanded, &costs, &weights, &best_so_far, &[]);
            steps_run = step + 1;
            println!("{}", format!("no progress in {} steps, stopping at {}", dead_runs, step).yellow());
            break;
        }

        if verbose {
            println!("{}", format!("Step {}: expanded all particles", step).dimmed());
            print_top_particles(&expanded, &weights, &shared, original_size, |i| costs[i]);
        }

        let weights_acc = normalize_and_accumulate(&mut weights);
        let mut counts: Vec<usize> = vec![0; expanded.len()];
        let resample_indices: Vec<usize> = (0..num_particles)
            .map(|_| {
                let idx = weighted_choice(&weights_acc);
                counts[idx] += 1;
                idx
            })
            .collect();

        if debug {
            debug_steps.push(StepLog {
                step,
                particles: build_particle_logs(&expanded, &costs, &weights),
                resample_indices,
                best_cost: best_so_far.as_ref().map(|(c, _)| *c),
                best_pattern: best_so_far.as_ref().map(|(_, s)| s.pattern.to_string()),
            });
        }

        if verbose {
            println!("{}", format!("Step {}: resampled all particles", step).dimmed());
            let resample_weights: Vec<f64> = counts.iter().map(|&c| c as f64 / num_particles as f64).collect();
            print_top_particles(&expanded, &resample_weights, &shared, original_size, |i| costs[i]);
        }

        particles = expanded.into_iter().zip(counts).filter(|(_, c)| *c > 0).collect();
        steps_run = step + 1;
    }

    println!("\n{}", "═══ RESULT ═══".green().bold());
    if let (Some(iter), Some((cost, state))) = (best_found_at, best_so_far.as_ref()) {
        println!("{} {}", "best found at iteration:".dimmed(), iter.to_string().yellow());
        println!("{} {}", "pattern:".dimmed(), state.pattern.to_string().cyan().bold());
        println!("{} {}", "cost:".dimmed(), cost.to_string().green().bold());
        println!("{} {}", "compression ratio:".dimmed(), format!("{:.2}x", original_size as f64 / *cost as f64).green().bold());
    }

    let debug_log = if debug {
        Some(DebugLog {
            original_size,
            num_particles,
            temperature,
            steps: debug_steps,
        })
    } else {
        None
    };
    SmcResult {
        best: best_so_far,
        original_size,
        best_found_at,
        num_steps_run: steps_run,
        egraph: shared.egraph,
        debug_log,
    }
}

/// Samples an index from a normalized cumulative weight array.
pub fn weighted_choice(acc_weights: &[f64]) -> usize {
    let r: f64 = rand::rng().random_range(0.0..1.0);
    match acc_weights.binary_search_by(|&w| w.partial_cmp(&r).unwrap()) {
        Ok(idx) => idx,
        Err(idx) => idx,
    }
}

/// Normalizes weights in-place and returns a separate cumulative distribution.
pub fn normalize_and_accumulate(weights: &mut [f64]) -> Vec<f64> {
    let weight_sum = weights.iter().sum::<f64>();
    if weight_sum == 0.0 {
        let len = weights.len();
        weights.fill(1.0 / len as f64);
    } else {
        weights.iter_mut().for_each(|w| *w /= weight_sum);
    }
    let mut weights_acc = Vec::with_capacity(weights.len());
    let mut accum = 0.0;
    for w in weights.iter() {
        accum += *w;
        weights_acc.push(accum);
    }
    weights_acc
}

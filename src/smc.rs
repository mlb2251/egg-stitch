use colored::Colorize;

use crate::cost::{compute_cost, compute_size};
use crate::debug_log::{DebugLog, StepLog, build_particle_logs, log_debug_step};
use crate::lang::StitchEgraph;
use crate::logging::{apply_follow_constraint, print_top_particles};
use crate::revexpr::RevExpr;
use crate::search::{SearchState, SharedSearchData};
use egg::ENodeOrVar;
use rand::Rng;

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
#[allow(clippy::needless_range_loop)]
pub fn smc(egraph: StitchEgraph, root: egg::Id, args: &crate::Args) -> SmcResult {
    let follow_expr: Option<RevExpr<ENodeOrVar<crate::lang::StitchLang>>> = args.follow.as_deref().map(|s| s.parse().unwrap_or_else(|e| panic!("failed to parse follow pattern '{}': {:?}", s, e)));
    let usage_counts = crate::search::compute_usage_counts(&egraph, root);
    let shared = SharedSearchData {
        egraph,
        follow: follow_expr,
        weight_by_usage: args.weight_by_usage,
        usage_counts,
        p_reuse: args.p_reuse,
        check_slow: args.check_slow,
    };

    let original_size = compute_size(&shared.egraph, root, &SearchState::new(&shared), shared.check_slow);
    println!("{} {}", "original size of egraph:".dimmed(), original_size.to_string().bold());

    let num_particles = args.num_particles;
    let num_steps = args.num_steps;
    let temperature = args.temperature;
    let dead_runs = args.dead_runs;
    let max_arity = args.max_arity;

    let mut best_so_far: Option<(usize, SearchState)> = None;
    let mut best_found_at = None;
    let mut steps_run = 0;
    let debug = args.debug_log;
    let mut debug_steps: Vec<StepLog> = Vec::new();

    let mut search_states: Vec<SearchState> = (0..num_particles).map(|_| SearchState::new(&shared)).collect();

    for step in 0..num_steps {

        // === PROPOSE ===
        for ss in search_states.iter_mut() {
            ss.expand_random(&shared, false);
        }

        // === COST ===
        let costs: Vec<usize> = search_states.iter().map(|s| compute_cost(&shared.egraph, root, s, shared.check_slow)).collect();
        
        // === BEST-SO-FAR ===
        for (i, cost) in costs.iter().enumerate() {
            if search_states[i].pattern.vars.len() <= max_arity && best_so_far.as_ref().is_none_or(|best| *cost < best.0) {
                println!("{} {} {}", format!("[iteration {}]", step).yellow().bold(), format!("new best: {}", cost).green().bold(), search_states[i].pattern.to_string().cyan());
                best_so_far = Some((*cost, search_states[i].clone()));
                best_found_at = Some(step);
            }
        }

        // === WEIGHT ===
        // weight = exp(-cost / temperature) / total_weight
        let mut weights: Vec<f64> = costs.iter().map(|c| -(*c as f64) / temperature).collect();
        let max_weight = weights.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        for w in &mut weights {
            *w = (*w - max_weight).exp();
        }

        // weight=0 for programs without holes (to make space for new ones)
        for (i, s) in search_states.iter().enumerate() {
            if s.pattern.vars.is_empty() {
                weights[i] = 0.0;
            }
        }

        // weight=0 for programs that don't match the follow pattern
        if let Some(ref follow) = shared.follow {
            apply_follow_constraint(&search_states, &mut weights, follow);
        }

        let norm_weights = normalize_weights(&weights);

        if weights.iter().sum::<f64>() == 0.0 {
            log_debug_step(debug, &mut debug_steps, step, &search_states, &costs, &norm_weights, &best_so_far, &[]);
            steps_run = step + 1;
            println!("{}", "all particles died, stopping".red().bold());
            break;
        }
        if best_found_at.is_some_and(|bf| (step as i64) - (bf as i64) > dead_runs as i64) {
            log_debug_step(debug, &mut debug_steps, step, &search_states, &costs, &norm_weights, &best_so_far, &[]);
            steps_run = step + 1;
            println!("{}", format!("no progress in {} steps, stopping at {}", dead_runs, step).yellow());
            break;
        }

        let debug_particles = if debug { Some(build_particle_logs(&search_states, &costs, &norm_weights)) } else { None };

        normalize_and_accumulate(&mut weights);
        println!("{}", format!("Step {}: expanded all particles", step).dimmed());
        print_top_particles(&search_states, &weights, &shared, original_size, |i| costs[i]);

        // === RESAMPLE ===
        let mut resample_indices: Vec<usize> = Vec::new();
        search_states = (0..num_particles)
            .map(|_| {
                let idx = weighted_choice(&weights);
                resample_indices.push(idx);
                search_states[idx].clone()
            })
            .collect();

        if let Some(particles) = debug_particles {
            debug_steps.push(StepLog {
                step,
                particles,
                resample_indices: resample_indices.clone(),
                best_cost: best_so_far.as_ref().map(|(c, _)| *c),
                best_pattern: best_so_far.as_ref().map(|(_, s)| s.pattern.to_string()),
            });
        }

        println!("{}", format!("Step {}: resampled all particles", step).dimmed());
        print_top_particles(&search_states, &weights, &shared, original_size, |i| compute_cost(&shared.egraph, root, &search_states[i], shared.check_slow));
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

/// Returns weights normalized to sum to 1 (or all zeros if sum is zero).
fn normalize_weights(weights: &[f64]) -> Vec<f64> {
    let sum: f64 = weights.iter().sum();
    if sum == 0.0 { vec![0.0; weights.len()] } else { weights.iter().map(|w| w / sum).collect() }
}

/// Samples an index from a normalized cumulative weight array.
pub fn weighted_choice(acc_weights: &[f64]) -> usize {
    let r: f64 = rand::rng().random_range(0.0..1.0);
    match acc_weights.binary_search_by(|&w| w.partial_cmp(&r).unwrap()) {
        Ok(idx) => idx,
        Err(idx) => idx,
    }
}

/// Normalizes weights to sum to 1 then converts to a cumulative distribution in-place.
pub fn normalize_and_accumulate(weights: &mut Vec<f64>) {
    let weight_sum = weights.iter().sum::<f64>();
    if weight_sum == 0.0 {
        let len = weights.len();
        weights.fill(1.0 / len as f64);
    } else {
        weights.iter_mut().for_each(|w| *w /= weight_sum);
    }
    let mut accum = 0.0;
    for w in weights {
        accum += *w;
        *w = accum;
    }
}

use colored::Colorize;

use crate::cost::compute_cost;
use crate::debug_log::{DebugLog, StepLog, build_particle_logs, log_debug_step};
use crate::lang::StitchEgraph;
use crate::logging::{apply_follow_constraint, print_top_particles};
use crate::math::logaddexp;
use crate::search::{SearchState, setup_search};
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
pub fn smc(egraph: StitchEgraph, root: egg::Id, args: &crate::Args) -> SmcResult {
    let (shared, original_size) = setup_search(egraph, root, args);
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

    let mut search_states: Vec<SearchState> = (0..num_particles).map(|_| SearchState::new(&shared)).collect();

    for step in 0..num_steps {
        for ss in search_states.iter_mut() {
            ss.expand_random(&shared, false);
        }

        let costs: Vec<usize> = search_states.iter().map(|s| compute_cost(&shared.egraph, root, s, shared.check_slow)).collect();

        for (i, cost) in costs.iter().enumerate() {
            if search_states[i].pattern.vars.len() <= max_arity && best_so_far.as_ref().is_none_or(|best| *cost < best.0) {
                println!("{} {} {}", format!("[iteration {}]", step).yellow().bold(), format!("new best: {}", cost).green().bold(), search_states[i].pattern.to_string().cyan());
                best_so_far = Some((*cost, search_states[i].clone()));
                best_found_at = Some(step);
            }
        }

        // log-space weights: logw_i = -cost_i / temperature
        let mut log_weights: Vec<f64> = costs.iter().map(|c| -(*c as f64) / temperature).collect();

        for (i, s) in search_states.iter().enumerate() {
            if s.pattern.vars.is_empty() {
                log_weights[i] = f64::NEG_INFINITY;
            }
        }

        if let Some(ref follow) = shared.follow {
            apply_follow_constraint(&search_states, &mut log_weights, follow, &shared, original_size, &costs, verbose);
        }

        let total_weight = log_weights.iter().copied().fold(f64::NEG_INFINITY, logaddexp);
        let mut weights: Vec<f64> = if total_weight.is_finite() {
            log_weights.iter().map(|lw| (lw - total_weight).exp()).collect()
        } else {
            vec![0.0; log_weights.len()]
        };

        if weights.iter().sum::<f64>() == 0.0 {
            log_debug_step(debug, &mut debug_steps, step, &search_states, &costs, &weights, &best_so_far, &[]);
            steps_run = step + 1;
            println!("{}", "all particles died, stopping".red().bold());
            break;
        }
        if best_found_at.is_some_and(|bf| (step as i64) - (bf as i64) > dead_runs as i64) {
            log_debug_step(debug, &mut debug_steps, step, &search_states, &costs, &weights, &best_so_far, &[]);
            steps_run = step + 1;
            println!("{}", format!("no progress in {} steps, stopping at {}", dead_runs, step).yellow());
            break;
        }

        if verbose {
            println!("{}", format!("Step {}: expanded all particles", step).dimmed());
            print_top_particles(&search_states, &weights, &shared, original_size, |i| costs[i]);
        }

        let weights_acc = normalize_and_accumulate(&mut weights);
        let resample_indices: Vec<usize> = (0..num_particles).map(|_| weighted_choice(&weights_acc)).collect();
        search_states = resample_indices.iter().map(|&idx| search_states[idx].clone()).collect();

        if debug {
            debug_steps.push(StepLog {
                step,
                particles: build_particle_logs(&search_states, &costs, &weights),
                resample_indices,
                best_cost: best_so_far.as_ref().map(|(c, _)| *c),
                best_pattern: best_so_far.as_ref().map(|(_, s)| s.pattern.to_string()),
            });
        }

        if verbose {
            println!("{}", format!("Step {}: resampled all particles", step).dimmed());
            print_top_particles(&search_states, &weights, &shared, original_size, |i| compute_cost(&shared.egraph, root, &search_states[i], shared.check_slow));
        }
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
        Some(DebugLog { original_size, num_particles, temperature, steps: debug_steps })
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

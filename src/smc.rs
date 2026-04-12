use std::cmp::min;

use crate::cost::{compute_cost, compute_size};
use crate::debug_log::{DebugLog, StepLog, build_particle_logs, log_debug_step};
use crate::lang::StitchEgraph;
use crate::math::logaddexp;
use crate::search::{SearchState, SharedSearchData};
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

pub fn smc(egraph: StitchEgraph, root: egg::Id, args: &crate::Args) -> SmcResult {
    let usage_counts = crate::search::compute_usage_counts(&egraph, root);
    let shared = SharedSearchData {
        egraph,
        p_reuse: args.p_reuse,
        check_slow: args.check_slow,
        weight_by_usage: args.weight_by_usage,
        usage_counts,
    };

    let original_size = compute_size(&shared.egraph, root, &SearchState::new(&shared), shared.check_slow);
    println!("original size of egraph: {}", original_size);

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
        for ss in search_states.iter_mut() {
            ss.expand_random(&shared);
        }

        let costs: Vec<usize> = search_states.iter().map(|s| compute_cost(&shared.egraph, root, s, shared.check_slow)).collect();

        for (i, cost) in costs.iter().enumerate() {
            if search_states[i].pattern.vars.len() <= max_arity && best_so_far.as_ref().is_none_or(|best| *cost < best.0) {
                println!("[iteration {}] new best: {} {}", step, cost, search_states[i].pattern);
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

        let total_weight = log_weights.iter().copied().fold(f64::NEG_INFINITY, logaddexp);
        let mut weights: Vec<f64> = if total_weight.is_finite() {
            log_weights.iter().map(|lw| (lw - total_weight).exp()).collect()
        } else {
            vec![0.0; log_weights.len()]
        };

        if weights.iter().sum::<f64>() == 0.0 {
            log_debug_step(debug, &mut debug_steps, step, &search_states, &costs, &weights, &best_so_far, &[]);
            steps_run = step + 1;
            println!("all particles died, stopping");
            break;
        }
        if best_found_at.is_some_and(|bf| (step as i64) - (bf as i64) > dead_runs as i64) {
            log_debug_step(debug, &mut debug_steps, step, &search_states, &costs, &weights, &best_so_far, &[]);
            steps_run = step + 1;
            println!("no progress in {} steps, stopping at {}", dead_runs, step);
            break;
        }

        let weights_acc = normalize_and_accumulate(&mut weights);

        println!("Step {}: expanded all particles", step);
        for i in 0..min(5, search_states.len()) {
            println!("Sample particle {}: {}; cost={} weight={}", i, search_states[i].pattern, costs[i], weights[i]);
        }

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
        steps_run = step + 1;
    }

    if let (Some(iter), Some((cost, state))) = (best_found_at, best_so_far.as_ref()) {
        println!("best found at iteration {}: {}", iter, cost);
        println!("program: {}", state.pattern);
        println!("best: {}", cost);
        println!("Compression ratio: {}", original_size as f64 / *cost as f64);
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
